use axum::{
    extract::{State, FromRequestParts, Path},
    http::{request::Parts, StatusCode, header::AUTHORIZATION, Method, HeaderMap},
    routing::{get, post, any},
    response::IntoResponse,
    body::to_bytes,
    Json, Router,
    async_trait,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use arc_swap::ArcSwap;
use crate::core::{KeyPool};
use crate::auth::{AuthManager, Claims};
use crate::config::{AppConfig};
use crate::mcp::{BalancerMcpServer, McpRequest, McpResponse};
use crate::db::{Database, LogEntry};
use utoipa::{OpenApi, ToSchema};
use std::time::Duration;
use tokio::time::{sleep, Instant};

#[derive(Debug, Deserialize, ToSchema)]
pub struct ExecuteRequest {
    pub task_name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExecuteResponse {
    pub status: String,
    pub key_id: String,
    pub message: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterClientRequest {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegisterClientResponse {
    pub token: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ConfigPatchRequest {
    pub server: Option<crate::config::ServerConfig>,
    pub auth: Option<crate::config::AuthConfig>,
}

pub struct AppState {
    pub pool: KeyPool,
    pub auth: AuthManager,
    pub config: Arc<ArcSwap<AppConfig>>,
    pub mcp: BalancerMcpServer,
    pub db: Database,
    pub http_client: reqwest::Client,
}

/// OAuth 2.1 Bearer Token extractor
pub struct AuthToken(pub Claims);
#[allow(dead_code)]
pub struct AdminToken(pub Claims);

#[async_trait]
impl<S> FromRequestParts<S> for AuthToken
where
    Arc<AppState>: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = Arc::from_ref(state);
        let config = state.config.load();

        // 0. Bypass if auth is disabled (for local usage)
        if !config.auth.enabled {
            return Ok(AuthToken(Claims {
                sub: "local-client".to_string(),
                exp: 0,
                iss: "local".to_string(),
                aud: "local".to_string(),
                role: Some("admin".to_string()),
            }));
        }
        
        // 1. Check for X-Admin-Key (Bypass for admin)
        let admin_header = parts.headers.get("X-Admin-Key").and_then(|h| h.to_str().ok());
        let admin_secret = std::env::var("ADMIN_API_KEY").unwrap_or_else(|_| "change-me".to_string());

        if let Some(key) = admin_header {
            if key == admin_secret {
                return Ok(AuthToken(Claims {
                    sub: "admin-bypass".to_string(),
                    exp: 0,
                    iss: "internal".to_string(),
                    aud: "admin".to_string(),
                    role: Some("admin".to_string()),
                }));
            }
        }

        // 2. Check for Bearer token
        let auth_header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .filter(|h| h.starts_with("Bearer "))
            .map(|h| &h[7..]);

        match auth_header {
            Some(token) => {
                match state.auth.validate_token(token) {
                    Ok(claims) => Ok(AuthToken(claims)),
                    Err(e) => Err((StatusCode::UNAUTHORIZED, format!("Unauthorized: {}", e))),
                }
            }
            None => Err((StatusCode::UNAUTHORIZED, "Missing authentication".to_string())),
        }
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for AdminToken
where
    Arc<AppState>: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let auth = AuthToken::from_request_parts(parts, state).await?;
        if auth.0.role.as_deref() == Some("admin") || auth.0.sub == "admin-bypass" {
            Ok(AdminToken(auth.0))
        } else {
            Err((StatusCode::FORBIDDEN, "Admin privileges required".to_string()))
        }
    }
}

pub trait FromRef<T> {
    fn from_ref(input: &T) -> Self;
}

impl FromRef<Arc<AppState>> for Arc<AppState> {
    fn from_ref(input: &Arc<AppState>) -> Self {
        Arc::clone(input)
    }
}

pub fn create_router(pool: KeyPool, auth: AuthManager, config: Arc<ArcSwap<AppConfig>>, db: Database) -> Router {
    let mcp = BalancerMcpServer::new(pool.clone(), config.clone());
    let state = Arc::new(AppState { 
        pool, 
        auth,
        config,
        mcp,
        db,
        http_client: reqwest::Client::new(),
    });
    
    Router::new()
        .route("/execute", post(handle_execute))
        .route("/stats", get(handle_stats))
        .route("/config", get(handle_get_config).patch(handle_patch_config))
        .route("/admin/clients", post(handle_register_client))
        .route("/proxy/:pool_name/*path", any(handle_proxy))
        .route("/mcp", post(handle_mcp))
        .with_state(state)
}

#[utoipa::path(
    post,
    path = "/execute",
    request_body = ExecuteRequest,
    responses(
        (status = 200, description = "Task executed successfully", body = ExecuteResponse),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limit exceeded")
    ),
    security(
        ("bearer_auth" = []),
        ("admin_key" = [])
    )
)]
async fn handle_execute(
    State(state): State<Arc<AppState>>,
    token: AuthToken,
    Json(payload): Json<ExecuteRequest>,
) -> Json<ExecuteResponse> {
    let start = Instant::now();
    let pool = &state.pool;
    let key = pool.acquire().await;
    
    let result = if let Err(e) = key.try_use() {
        pool.release(key).await;
        
        let response = ExecuteResponse {
            status: "error".to_string(),
            key_id: "none".to_string(),
            message: format!("Rate limit hit: {}", e),
        };

        // Log failure
        let _ = state.db.log_request(LogEntry {
            client_id: Some(token.0.sub),
            key_id: None,
            pool_id: None,
            status: "rate_limited".to_string(),
            latency_ms: Some(start.elapsed().as_millis() as i64),
            error_message: Some(e.to_string()),
            request_ip: None,
        }).await;

        return Json(response);
    } else {
        sleep(Duration::from_millis(50)).await;
        
        let key_id = key.id();
        pool.release(key).await;

        ExecuteResponse {
            status: "success".to_string(),
            key_id: key_id.clone(),
            message: format!("Task '{}' completed safely", payload.task_name),
        }
    };

    // Log success
    let _ = state.db.log_request(LogEntry {
        client_id: Some(token.0.sub),
        key_id: Some(result.key_id.clone()),
        pool_id: None,
        status: "success".to_string(),
        latency_ms: Some(start.elapsed().as_millis() as i64),
        error_message: None,
        request_ip: None,
    }).await;

    Json(result)
}

#[derive(OpenApi)]
#[openapi(
    paths(
        handle_execute,
        handle_stats,
        handle_get_config,
        handle_patch_config,
    ),
    components(
        schemas(
            ExecuteRequest, 
            ExecuteResponse, 
            RegisterClientRequest,
            RegisterClientResponse,
            ConfigPatchRequest, 
            crate::config::AppConfig,
            crate::config::ServerConfig,
            crate::config::AuthConfig,
            crate::config::PoolConfig,
            crate::config::KeyConfig,
            crate::db::LogEntry,
        )
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

#[utoipa::path(
    post,
    path = "/admin/clients",
    request_body = RegisterClientRequest,
    responses(
        (status = 200, description = "Client registered successfully", body = RegisterClientResponse),
        (status = 403, description = "Forbidden")
    ),
    security(("admin_key" = []))
)]
async fn handle_register_client(
    State(state): State<Arc<AppState>>,
    _token: AdminToken,
    Json(payload): Json<RegisterClientRequest>,
) -> (StatusCode, Json<RegisterClientResponse>) {
    let token = state.auth.generate_token(&payload.id, None).unwrap();
    
    if let Err(e) = state.db.register_client(&payload.id, &payload.name, &token).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(RegisterClientResponse { token: format!("Error: {}", e) }));
    }

    (StatusCode::CREATED, Json(RegisterClientResponse { token }))
}

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.as_mut().unwrap();
        components.add_security_scheme(
            "bearer_auth",
            utoipa::openapi::security::SecurityScheme::Http(
                utoipa::openapi::security::HttpBuilder::new()
                    .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
        components.add_security_scheme(
            "admin_key",
            utoipa::openapi::security::SecurityScheme::ApiKey(
                utoipa::openapi::security::ApiKey::Header(utoipa::openapi::security::ApiKeyValue::new("X-Admin-Key")),
            ),
        );
    }
}

#[utoipa::path(
    get,
    path = "/stats",
    responses(
        (status = 200, description = "Current pool statistics", body = serde_json::Value),
        (status = 403, description = "Forbidden")
    ),
    security(("admin_key" = []))
)]
async fn handle_stats(
    State(state): State<Arc<AppState>>,
    _token: AdminToken
) -> Json<serde_json::Value> {
    let stats = state.db.get_stats().await.unwrap_or_else(|_| serde_json::json!({"error": "Failed to fetch stats"}));
    Json(stats)
}

#[utoipa::path(
    get,
    path = "/config",
    responses(
        (status = 200, description = "Current application configuration", body = AppConfig),
        (status = 403, description = "Forbidden")
    ),
    security(("admin_key" = []))
)]
async fn handle_get_config(
    State(state): State<Arc<AppState>>,
    _token: AdminToken,
) -> Json<AppConfig> {
    let mut config = (**state.config.load()).clone();
    config.auth.secret = "[REDACTED]".to_string();
    Json(config)
}

#[utoipa::path(
    patch,
    path = "/config",
    request_body = ConfigPatchRequest,
    responses(
        (status = 200, description = "Configuration updated", body = AppConfig),
        (status = 403, description = "Forbidden")
    ),
    security(("admin_key" = []))
)]
async fn handle_patch_config(
    State(state): State<Arc<AppState>>,
    _token: AdminToken,
    Json(payload): Json<ConfigPatchRequest>,
) -> Json<AppConfig> {
    let current_config = state.config.load();
    let mut new_config = (**current_config).clone();

    if let Some(s) = payload.server {
        new_config.server = s;
    }
    if let Some(a) = payload.auth {
        new_config.auth = a;
    }

    state.config.store(Arc::new(new_config.clone()));
    
    let mut sanitized = new_config;
    sanitized.auth.secret = "[REDACTED]".to_string();
    Json(sanitized)
}

/// Simple MCP JSON-RPC handler
async fn handle_mcp(
    State(state): State<Arc<AppState>>,
    token: AuthToken,
    Json(request): Json<McpRequest>,
) -> Json<McpResponse> {
    let result = match request.method.as_str() {
        "list_pools" => {
            let allowed_pools = if token.0.role.as_deref() == Some("admin") {
                None // Admin sees all
            } else {
                Some(state.db.get_allowed_pools(&token.0.sub).await.unwrap_or_default())
            };

            let mut pools = state.mcp.list_pools().await;
            if let Some(allowed) = allowed_pools {
                pools.retain(|p| {
                    if let Some(name) = p["name"].as_str() {
                        allowed.contains(&name.to_string())
                    } else {
                        false
                    }
                });
            }
            Some(serde_json::to_value(pools).unwrap())
        },
        "get_config" => Some(state.mcp.get_config_resource().await),
        "update_description" => {
            if token.0.role.as_deref() != Some("admin") && token.0.sub != "admin-bypass" {
                return Json(McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: None,
                    error: Some(serde_json::Value::String("Admin privileges required for this MCP method".to_string())),
                });
            }
            let args: crate::mcp::UpdateDescriptionArgs = serde_json::from_value(request.params.unwrap_or_default()).unwrap();
            match state.mcp.update_pool_description(args).await {
                Ok(msg) => Some(serde_json::Value::String(msg)),
                Err(e) => return Json(McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: None,
                    error: Some(serde_json::Value::String(e)),
                }),
            }
        },
        _ => None,
    };

    let is_none = result.is_none();
    Json(McpResponse {
        jsonrpc: "2.0".to_string(),
        id: request.id,
        result,
        error: if is_none { Some(serde_json::Value::String("Method not found".to_string())) } else { None },
    })
}

/// Transparent Proxy Handler
async fn handle_proxy(
    State(state): State<Arc<AppState>>,
    _token: AuthToken, // Ensure client is authenticated
    method: Method,
    Path((pool_name, path)): Path<(String, String)>,
    headers: HeaderMap,
    body: axum::body::Body,
) -> impl axum::response::IntoResponse {
    let config = state.config.load();
    let pool_cfg = config.pools.iter().find(|p| p.name == pool_name);

    if pool_cfg.is_none() {
        return (StatusCode::NOT_FOUND, "Pool not found").into_response();
    }

    let target_base = &pool_cfg.unwrap().target_url;
    let target_url = format!("{}/{}", target_base.trim_end_matches('/'), path);

    // Acquire key from pool
    // Note: We currently only support 'primary' pool in KeyPool struct. 
    // For multiple pools, we'd need a Map of KeyPools.
    // For now, let's assume we use the primary pool for all proxying for simplicity.
    let pool = &state.pool;
    let key = pool.acquire().await;
    
    // Get actual secret from storage
    // Actually, KeyConfig has secret_name. We need to load it.
    // BUT we don't store SecretStorage in AppState yet.
    // Let's assume the secret is already in the ApiKeyInner (we added it as _secret earlier).
    let secret = {
        let inner = key.inner.lock().unwrap();
        inner._secret.clone()
    };

    // Forward the request
    let mut req_builder = state.http_client.request(method, &target_url);

    // Forward headers (except authorization)
    for (name, value) in headers.iter() {
        if name != AUTHORIZATION && name != "host" {
            req_builder = req_builder.header(name, value);
        }
    }

    // Inject our balanced key based on provider detection
    let is_google = target_url.contains("googleapis.com");
    let is_anthropic = target_url.contains("anthropic.com");

    if is_google {
        req_builder = req_builder.header("x-goog-api-key", &secret);
    } else if is_anthropic {
        req_builder = req_builder.header("x-api-key", &secret);
        req_builder = req_builder.header("anthropic-version", "2023-06-01");
    } else {
        // Default to OpenAI-compatible Bearer token
        req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", secret));
    }
    
    let body_bytes = to_bytes(body, 25 * 1024 * 1024).await.unwrap_or_default(); // 25MB limit
    let res = req_builder.body(body_bytes).send().await;

    pool.release(key).await;

    match res {
        Ok(response) => {
            let status = response.status();
            let mut res_builder = axum::response::Response::builder().status(status);
            
            for (name, value) in response.headers().iter() {
                res_builder = res_builder.header(name, value);
            }

            let body_bytes = response.bytes().await.unwrap_or_default();
            res_builder.body(axum::body::Body::from(body_bytes)).unwrap().into_response()
        }
        Err(e) => (StatusCode::BAD_GATEWAY, format!("Proxy error: {}", e)).into_response(),
    }
}
