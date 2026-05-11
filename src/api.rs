use axum::{
    extract::{State, FromRequestParts, Path},
    http::{request::Parts, StatusCode, header::AUTHORIZATION, Method, HeaderMap, Response, Uri},
    routing::{get, post, any},
    response::IntoResponse,
    body::{to_bytes, Body},
    Json, Router,
    async_trait,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::collections::HashMap;
use arc_swap::ArcSwap;
use uuid::Uuid;
use futures::StreamExt;
use crate::core::{KeyPool};
use crate::auth::{AuthManager, Claims};
use crate::config::{AppConfig};
use crate::mcp::{BalancerMcpServer, McpRequest, McpResponse};
use crate::db::{Database, LogEntry};
use utoipa::{OpenApi, ToSchema};
use std::time::Duration;
use tokio::time::{sleep, Instant};

fn is_hop_by_hop_header(name: &axum::http::HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
    )
}

fn build_target_url(target_base: &str, path: &str, query: Option<&str>) -> String {
    let mut url = if path.is_empty() || path == "/" {
        target_base.to_string()
    } else if target_base.ends_with('/') {
        format!("{}{}", target_base, path.trim_start_matches('/'))
    } else {
        format!("{}/{}", target_base, path.trim_start_matches('/'))
    };

    if let Some(query) = query.filter(|q| !q.is_empty()) {
        let separator = if url.contains('?') { '&' } else { '?' };
        url.push(separator);
        url.push_str(query);
    }

    url
}

fn extract_response_tokens(bytes: &[u8], input_tokens: u32) -> u32 {
    let mut output_tokens = 0;

    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(bytes) {
        if let Some(total) = json.get("usage").and_then(|u| u.get("total_tokens")).and_then(|t| t.as_u64()) {
            output_tokens = total as u32;
        } else if let Some(total) = json.get("usageMetadata").and_then(|u| u.get("totalTokenCount")).and_then(|t| t.as_u64()) {
            output_tokens = total as u32;
        } else {
            output_tokens = crate::utils::estimate_response_tokens(&json) as u32;
        }
    } else if let Ok(text) = std::str::from_utf8(bytes) {
        for payload in text.split("\n\n") {
            for line in payload.lines() {
                let line = line.trim();
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if data == "[DONE]" || data.is_empty() {
                        continue;
                    }

                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(total) = json.get("usage").and_then(|u| u.get("total_tokens")).and_then(|t| t.as_u64()) {
                            output_tokens = output_tokens.max(total as u32);
                        } else if let Some(total) = json.get("usageMetadata").and_then(|u| u.get("totalTokenCount")).and_then(|t| t.as_u64()) {
                            output_tokens = output_tokens.max(total as u32);
                        } else {
                            output_tokens = output_tokens.max(crate::utils::estimate_response_tokens(&json) as u32);
                        }
                    }
                }
            }
        }
    }

    if output_tokens > input_tokens {
        output_tokens
    } else {
        input_tokens.saturating_add(output_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::{build_target_url, extract_response_tokens};

    #[test]
    fn builds_target_url_with_path_and_query() {
        let url = build_target_url(
            "https://generativelanguage.googleapis.com/v1beta",
            "models/gemini-flash-lite-latest:streamGenerateContent",
            Some("alt=sse"),
        );
        assert_eq!(
            url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-flash-lite-latest:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn extracts_tokens_from_gemini_sse_payload() {
        let payload = br#"data: {"candidates":[{"content":{"parts":[{"text":"hi"}]}}],"usageMetadata":{"totalTokenCount":42}}

data: [DONE]

"#;

        assert_eq!(extract_response_tokens(payload, 10), 42);
    }
}

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
    pub id: Option<String>,
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegisterClientResponse {
    pub client_id: String,
    pub token: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ConfigPatchRequest {
    pub server: Option<crate::config::ServerConfig>,
    pub auth: Option<crate::config::AuthConfig>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct KeyExportResponse {
    pub key: crate::config::KeyConfig,
    pub secret: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct KeyImportRequest {
    pub key: crate::config::KeyConfig,
    pub secret: String,
}

pub struct AppState {
    pub pools: HashMap<String, KeyPool>,
    pub auth: AuthManager,
    pub config: Arc<ArcSwap<AppConfig>>,
    pub mcp: BalancerMcpServer,
    pub db: Database,
    pub storage: crate::storage::SecretStorage,
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
        let admin_secret = config.auth.admin_key.clone()
            .or_else(|| std::env::var("ADMIN_API_KEY").ok())
            .unwrap_or_else(|| "admin-secret-key-2026".to_string());


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
                // 2.1 Check for Master Key (Universal shared secret)
                if let Some(ref master) = config.auth.master_key {
                    if token == master {
                        return Ok(AuthToken(Claims {
                            sub: "master-user".to_string(),
                            exp: 0,
                            iss: "internal".to_string(),
                            aud: "all".to_string(),
                            role: Some("admin".to_string()),
                        }));
                    }
                }

                // 2.2 Standard JWT validation
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

pub fn create_router(
    pools: HashMap<String, KeyPool>, 
    auth: AuthManager, 
    config: Arc<ArcSwap<AppConfig>>, 
    db: Database,
    storage: crate::storage::SecretStorage,
) -> Router {
    // Note: MCP currently still uses 'primary' pool or first pool for simplicity
    let mcp = BalancerMcpServer::new(pools.clone(), config.clone(), storage.clone());
    
    let state = Arc::new(AppState { 
        pools, 
        auth,
        config,
        mcp,
        db,
        storage,
        http_client: reqwest::Client::new(),
    });
    
    Router::new()
        .route("/execute", post(handle_execute))
        .route("/stats", get(handle_stats))
        .route("/config", get(handle_get_config).patch(handle_patch_config))
        .route("/auth/register", post(handle_public_register))
        .route("/admin/clients", post(handle_register_client))
        .route("/admin/keys/:pool_name/:key_id", get(handle_export_key))
        .route("/admin/keys/:pool_name", post(handle_import_key))
        .route("/proxy/:pool_name", any(handle_proxy))
        .route("/proxy/:pool_name/*path", any(handle_proxy))
        .route("/mcp", post(handle_mcp))
        .with_state(state)
}

/// Public endpoint for client registration (if enabled)
#[utoipa::path(
    post,
    path = "/auth/register",
    request_body = RegisterClientRequest,
    responses(
        (status = 201, description = "Client registered successfully", body = RegisterClientResponse),
        (status = 403, description = "Public registration is disabled"),
        (status = 500, description = "Internal server error")
    )
)]
async fn handle_public_register(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RegisterClientRequest>,
) -> impl IntoResponse {
    let config = state.config.load();
    if !config.auth.public_registration {
        return (StatusCode::FORBIDDEN, "Public registration is disabled").into_response();
    }

    let client_id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let token = state.auth.generate_token(&client_id, Some("client".to_string())).unwrap();

    if let Err(e) = state.db.register_client(&client_id, &payload.name, &token).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to register client: {}", e)).into_response();
    }

    (StatusCode::CREATED, Json(RegisterClientResponse {
        client_id,
        token,
    })).into_response()
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
    
    // For /execute, we use 'primary' pool or the first one
    let pool = match state.pools.get("primary").or_else(|| state.pools.values().next()) {
        Some(p) => p,
        None => return Json(ExecuteResponse { status: "error".to_string(), key_id: "none".to_string(), message: "No pools configured".to_string() }),
    };

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
            tokens_used: 0,
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
        tokens_used: 0,
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
        handle_register_client,
        handle_public_register,
        handle_export_key,
        handle_import_key,
    ),
    components(
        schemas(
            ExecuteRequest, 
            ExecuteResponse, 
            RegisterClientRequest,
            RegisterClientResponse,
            KeyExportResponse,
            KeyImportRequest,
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
) -> impl IntoResponse {
    let client_id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let token = state.auth.generate_token(&client_id, Some("admin".to_string())).unwrap();
    
    if let Err(e) = state.db.register_client(&client_id, &payload.name, &token).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {}", e)).into_response();
    }

    (StatusCode::CREATED, Json(RegisterClientResponse { client_id, token })).into_response()
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
    path = "/admin/keys/{pool_name}/{key_id}",
    params(
        ("pool_name" = String, Path, description = "Name of the pool"),
        ("key_id" = String, Path, description = "ID of the key")
    ),
    responses(
        (status = 200, description = "Key exported successfully", body = KeyExportResponse),
        (status = 404, description = "Key or pool not found"),
        (status = 403, description = "Forbidden")
    ),
    security(("admin_key" = []))
)]
async fn handle_export_key(
    State(state): State<Arc<AppState>>,
    _token: AdminToken,
    Path((pool_name, key_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let config = state.config.load();
    let pool_cfg = config.pools.iter().find(|p| p.name == pool_name);
    
    if let Some(pool) = pool_cfg {
        if let Some(key_cfg) = pool.keys.iter().find(|k| k.id == key_id) {
            match state.storage.load_secret(&key_cfg.secret_name) {
                Ok(secret) => return (StatusCode::OK, Json(KeyExportResponse { 
                    key: key_cfg.clone(), 
                    secret 
                })).into_response(),
                Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to load secret: {}", e)).into_response(),
            }
        }
    }
    
    (StatusCode::NOT_FOUND, "Key or pool not found").into_response()
}

#[utoipa::path(
    post,
    path = "/admin/keys/{pool_name}",
    params(
        ("pool_name" = String, Path, description = "Name of the pool")
    ),
    request_body = KeyImportRequest,
    responses(
        (status = 201, description = "Key imported successfully"),
        (status = 404, description = "Pool not found"),
        (status = 403, description = "Forbidden")
    ),
    security(("admin_key" = []))
)]
async fn handle_import_key(
    State(state): State<Arc<AppState>>,
    _token: AdminToken,
    Path(pool_name): Path<String>,
    Json(payload): Json<KeyImportRequest>,
) -> impl IntoResponse {
    // 1. Save secret to disk
    if let Err(e) = state.storage.save_secret(&payload.key.secret_name, &payload.secret) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save secret: {}", e)).into_response();
    }

    // 2. Update config in memory
    let mut new_config = (**state.config.load()).clone();
    let pool_idx = new_config.pools.iter().position(|p| p.name == pool_name);
    
    if let Some(idx) = pool_idx {
        new_config.pools[idx].keys.push(payload.key.clone());
        
        // 3. Persist config to disk
        if let Err(e) = new_config.save("config.yaml") {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save config: {}", e)).into_response();
        }
        
        // 4. Update the running pool
        if let Some(pool) = state.pools.get(&pool_name) {
            let key = crate::core::ApiKey::new(
                &payload.key.id,
                payload.key.rps_limit,
                payload.key.rpd_limit,
                payload.key.tpm_limit,
                payload.key.tpd_limit,
                payload.key.max_request_tokens,
                payload.key.cooldown_on_limit.unwrap_or(false),
                payload.secret,
                payload.key.secret_type.clone(),
                None,
            );
            
            for _ in 0..payload.key.concurrency {
                if let Err(e) = pool.add_key(key.clone()) {
                    return (StatusCode::BAD_REQUEST, format!("Failed to inject key into running pool: {}", e)).into_response();
                }
            }
        }

        state.config.store(Arc::new(new_config));
        (StatusCode::CREATED, "Key imported successfully").into_response()
    } else {
        (StatusCode::NOT_FOUND, "Pool not found").into_response()
    }
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
    if config.auth.master_key.is_some() {
        config.auth.master_key = Some("[REDACTED]".to_string());
    }
    Json(config)
}

#[utoipa::path(
    patch,
    path = "/config",
    request_body = ConfigPatchRequest,
    responses(
        (status = 200, description = "Configuration updated successfully", body = AppConfig),
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
    if sanitized.auth.master_key.is_some() {
        sanitized.auth.master_key = Some("[REDACTED]".to_string());
    }
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
        "export_key" => {
            if token.0.role.as_deref() != Some("admin") {
                return Json(McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: None,
                    error: Some(serde_json::Value::String("Admin privileges required for export_key".to_string())),
                });
            }
            let params = request.params.unwrap_or_default();
            let pool_name = params["pool_name"].as_str().unwrap_or_default();
            let key_id = params["key_id"].as_str().unwrap_or_default();
            match state.mcp.export_key(pool_name, key_id).await {
                Ok(v) => Some(v),
                Err(e) => return Json(McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: None,
                    error: Some(serde_json::Value::String(e)),
                }),
            }
        },
        "import_key" => {
            if token.0.role.as_deref() != Some("admin") {
                return Json(McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: None,
                    error: Some(serde_json::Value::String("Admin privileges required for import_key".to_string())),
                });
            }
            let params = request.params.unwrap_or_default();
            let pool_name = params["pool_name"].as_str().unwrap_or_default();
            let key_cfg_res: Result<crate::config::KeyConfig, _> = serde_json::from_value(params["key_cfg"].clone());
            let secret = params["secret"].as_str().unwrap_or_default().to_string();
            
            match key_cfg_res {
                Ok(key_cfg) => {
                    match state.mcp.import_key(pool_name, key_cfg, secret).await {
                        Ok(msg) => Some(serde_json::Value::String(msg)),
                        Err(e) => return Json(McpResponse {
                            jsonrpc: "2.0".to_string(),
                            id: request.id,
                            result: None,
                            error: Some(serde_json::Value::String(e)),
                        }),
                    }
                },
                Err(e) => return Json(McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: None,
                    error: Some(serde_json::Value::String(format!("Invalid key_cfg: {}", e))),
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
    token: AuthToken,
    method: Method,
    Path(params): Path<HashMap<String, String>>,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> impl axum::response::IntoResponse {
    let Some(pool_name) = params.get("pool_name").cloned() else {
        return (StatusCode::BAD_REQUEST, "Missing pool name").into_response();
    };
    let path = params.get("path").cloned().unwrap_or_default();

    let config = state.config.load();
    let pool_cfg = config.pools.iter().find(|p| p.name == pool_name);

    if pool_cfg.is_none() {
        return (StatusCode::NOT_FOUND, "Pool not found").into_response();
    }

    let target_base = &pool_cfg.unwrap().target_url;
    let provider = &pool_cfg.unwrap().provider;
    
    let target_url = build_target_url(target_base, &path, uri.query());
    // Acquire key from the specific pool
    let pool = match state.pools.get(&pool_name) {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "Pool implementation not found").into_response(),
    };
    let key = pool.acquire().await;
    let key_id = key.id();

    let body_bytes = to_bytes(body, 25 * 1024 * 1024).await.unwrap_or_default();
    let input_tokens = if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
        crate::utils::estimate_request_tokens(&json) as u32
    } else {
        0
    };

    if let Some(limit) = key.max_request_tokens() {
        if input_tokens > limit {
            pool.release(key).await;
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("Request token estimate {} exceeds per-request limit {}", input_tokens, limit),
            ).into_response();
        }
    }

    // Check limits
    if let Err(e) = key.try_use() {
        pool.release(key).await;
        return (StatusCode::TOO_MANY_REQUESTS, format!("Rate limit exceeded for key {}: {}", key_id, e)).into_response();
    }
    
    // Get actual secret from storage
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
    if provider == "google" || target_url.contains("googleapis.com") {
        req_builder = req_builder.header("x-goog-api-key", &secret);
    } else if provider == "anthropic" || target_url.contains("anthropic.com") {
        req_builder = req_builder.header("x-api-key", &secret);
        req_builder = req_builder.header("anthropic-version", "2023-06-01");
    } else {
        // Default to OpenAI-compatible Bearer token
        req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", secret));
    }

    let res = req_builder.body(body_bytes).send().await;

    match res {
        Ok(resp) => {
            let status = resp.status();
            let mut res_builder = Response::builder().status(status);
            let is_sse = resp
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.starts_with("text/event-stream"))
                .unwrap_or(false);
            
            for (name, value) in resp.headers().iter() {
                if !is_hop_by_hop_header(name) {
                    res_builder = res_builder.header(name, value);
                }
            }

            if is_sse {
                let db = state.db.clone();
                let pool = state.pools.get(&pool_name).cloned().unwrap();
                let client_id = token.0.sub.clone();
                let pool_name_for_log = pool_name.clone();
                let key_id_for_log = key_id.clone();
                let key_for_stream = key.clone();
                let status_for_log = if status.is_success() { "success".to_string() } else { status.as_str().to_string() };

                let stream = futures::stream::unfold(
                    (
                        resp.bytes_stream(),
                        Vec::<u8>::new(),
                        Some((db, pool, key_for_stream, client_id, key_id_for_log, pool_name_for_log, status_for_log)),
                    ),
                    move |(mut upstream, mut collected, resources)| async move {
                        match upstream.next().await {
                            Some(Ok(chunk)) => {
                                collected.extend_from_slice(&chunk);
                                Some((Ok::<_, std::io::Error>(chunk), (upstream, collected, resources)))
                            }
                            Some(Err(err)) => {
                                if let Some((db, pool, key, client_id, key_id, pool_name, _status_for_log)) = resources {
                                    let total_tokens = extract_response_tokens(&collected, input_tokens);
                                    key.record_usage(total_tokens);
                                    pool.release(key).await;
                                    let _ = db.log_request(crate::db::LogEntry {
                                        client_id: Some(client_id),
                                        key_id: Some(key_id),
                                        pool_id: Some(pool_name),
                                        status: "stream_error".to_string(),
                                        tokens_used: total_tokens,
                                        error_message: Some(err.to_string()),
                                        ..Default::default()
                                    }).await;
                                }
                                Some((Err(std::io::Error::other(err.to_string())), (upstream, collected, None)))
                            }
                            None => {
                                if let Some((db, pool, key, client_id, key_id, pool_name, status_for_log)) = resources {
                                    let total_tokens = extract_response_tokens(&collected, input_tokens);
                                    key.record_usage(total_tokens);
                                    pool.release(key).await;
                                    if let Err(e) = db.log_request(crate::db::LogEntry {
                                        client_id: Some(client_id),
                                        key_id: Some(key_id),
                                        pool_id: Some(pool_name),
                                        status: status_for_log,
                                        tokens_used: total_tokens,
                                        ..Default::default()
                                    }).await {
                                        eprintln!("Failed to log streaming request to DB: {}", e);
                                    }
                                }
                                None
                            }
                        }
                    },
                );

                res_builder.body(Body::from_stream(stream)).unwrap().into_response()
            } else {
                let bytes = resp.bytes().await.unwrap_or_default();
                let total_tokens = extract_response_tokens(&bytes, input_tokens);
                key.record_usage(total_tokens);

                state.pools.get(&pool_name).unwrap().release(key).await;
                
                if let Err(e) = state.db.log_request(crate::db::LogEntry {
                    client_id: Some(token.0.sub.clone()),
                    key_id: Some(key_id),
                    pool_id: Some(pool_name),
                    status: if status.is_success() { "success".to_string() } else { status.as_str().to_string() },
                    tokens_used: total_tokens,
                    ..Default::default()
                }).await {
                    eprintln!("Failed to log request to DB: {}", e);
                }

                res_builder.body(Body::from(bytes)).unwrap().into_response()
            }
        }
        Err(e) => {
            state.pools.get(&pool_name).unwrap().release(key).await;
            (StatusCode::BAD_GATEWAY, format!("Upstream error: {}", e)).into_response()
        }
    }
}
