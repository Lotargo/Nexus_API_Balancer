use axum::{
    extract::{State, FromRequestParts, Path},
    http::{request::Parts, StatusCode, header::AUTHORIZATION, Method, HeaderMap, HeaderName, Response, Uri},
    routing::{get, post, any},
    response::IntoResponse,
    body::{to_bytes, Body, Bytes},
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
use tokio::time::{timeout, Instant};
use std::time::Duration;
use chrono::Local;

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

fn should_forward_request_header(name: &HeaderName) -> bool {
    !is_hop_by_hop_header(name)
        && !matches!(
            name.as_str(),
            "authorization" | "host" | "x-goog-api-key" | "x-api-key" | "api-key"
        )
}

fn build_target_url(target_base: &str, path: &str, query: Option<&str>) -> String {
    // Avoid doubling the version prefix (e.g. /v1) if the path already includes it
    let clean_path = path.trim_start_matches('/');
    let target_trimmed = target_base.trim_end_matches('/');

    let mut final_path = clean_path.to_string();

    // For Google OpenAI-compatible endpoint (/v1beta/openai/), the base URL
    // already includes the API version. Strip any leading version prefix
    // from the incoming OpenAI path (e.g. /v1/chat/completions -> chat/completions).
    let is_openai_compat = target_trimmed.contains("/openai/");

    // List of common API versions
    let versions = ["v1", "v1beta", "v2"];
    for v in versions {
        let path_starts_with_v = clean_path.starts_with(&format!("{}/", v)) || clean_path == v;
        let target_ends_with_v = target_trimmed.ends_with(&format!("/{}", v));

        if path_starts_with_v && (target_ends_with_v || is_openai_compat) {
            // Strip the version from path since it's already in the target
            final_path = clean_path.replacen(&format!("{}/", v), "", 1);
            if final_path == v {
                final_path = "".to_string();
            }
            break;
        }
    }

    let mut url = if final_path.is_empty() {
        target_trimmed.to_string()
    } else {
        format!("{}/{}", target_trimmed, final_path)
    };

    if let Some(query) = query.filter(|q| !q.is_empty()) {
        // Remove 'key=' parameter if present to avoid conflicts with upstream
        let filtered: Vec<&str> = query.split('&')
            .filter(|pair| !pair.starts_with("key="))
            .collect();
            
        if !filtered.is_empty() {
            let separator = if url.contains('?') { '&' } else { '?' };
            url.push(separator);
            url.push_str(&filtered.join("&"));
        }
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

fn is_streaming_request(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|json| json.get("stream").and_then(|stream| stream.as_bool()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{build_target_url, extract_response_tokens, is_streaming_request, parse_explicit_model, should_forward_request_header};
    use axum::http::HeaderName;

    #[test]
    fn parses_explicit_provider_model() {
        // Standard //provider//model format
        let (provider, model) = parse_explicit_model("//cerebras//llama-3.1-8b");
        assert_eq!(provider, Some("cerebras".to_string()));
        assert_eq!(model, "llama-3.1-8b");

        // //sambanova//model
        let (provider, model) = parse_explicit_model("//sambanova//llama-3.1-8b");
        assert_eq!(provider, Some("sambanova".to_string()));
        assert_eq!(model, "llama-3.1-8b");

        // No prefix - passthrough
        let (provider, model) = parse_explicit_model("mistral-large-latest");
        assert_eq!(provider, None);
        assert_eq!(model, "mistral-large-latest");

        // Groq style with single slash - NOT matched
        let (provider, model) = parse_explicit_model("openai/gpt-oss-120b");
        assert_eq!(provider, None);
        assert_eq!(model, "openai/gpt-oss-120b");

        // Empty provider - not matched
        let (provider, model) = parse_explicit_model("//model");
        assert_eq!(provider, None);
        assert_eq!(model, "//model");

        // Nested slashes in model_id (e.g. openai/gpt-oss-120b)
        let (provider, model) = parse_explicit_model("//groq//openai/gpt-oss-120b");
        assert_eq!(provider, Some("groq".to_string()));
        assert_eq!(model, "openai/gpt-oss-120b");
    }

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

        let url2 = build_target_url(
            "https://api.mistral.ai/v1",
            "/v1/chat/completions",
            None,
        );
        assert_eq!(url2, "https://api.mistral.ai/v1/chat/completions");

        let url3 = build_target_url(
            "https://api.openai.com/v1/",
            "v1/models",
            None,
        );
        assert_eq!(url3, "https://api.openai.com/v1/models");
    }

    #[test]
    fn extracts_tokens_from_gemini_sse_payload() {
        let payload = br#"data: {"candidates":[{"content":{"parts":[{"text":"hi"}]}}],"usageMetadata":{"totalTokenCount":42}}

data: [DONE]

"#;

        assert_eq!(extract_response_tokens(payload, 10), 42);
    }

    #[test]
    fn detects_streaming_openai_requests() {
        assert!(is_streaming_request(br#"{"model":"x","stream":true}"#));
        assert!(!is_streaming_request(br#"{"model":"x","stream":false}"#));
        assert!(!is_streaming_request(br#"not-json"#));
    }

    #[test]
    fn drops_request_hop_by_hop_and_auth_headers() {
        for name in ["content-length", "connection", "authorization", "host", "x-api-key", "api-key", "x-goog-api-key"] {
            let header = HeaderName::from_bytes(name.as_bytes()).unwrap();
            assert!(!should_forward_request_header(&header));
        }

        let content_type = HeaderName::from_static("content-type");
        assert!(should_forward_request_header(&content_type));
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
    pub provider: Option<String>,
    pub kv_cache: Option<bool>,
}

pub struct AppState {
    pub pools: HashMap<String, KeyPool>,
    pub auth: AuthManager,
    pub config: Arc<ArcSwap<AppConfig>>,
    pub mcp: BalancerMcpServer,
    pub db: Database,
    pub storage: crate::storage::SecretStorage,
    pub http_client: reqwest::Client,
    pub model_registry: Arc<crate::model_registry::ModelRegistry>,
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

        // 2. Check for token in headers or query
        let auth_header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .filter(|h| h.starts_with("Bearer "))
            .map(|h| &h[7..]);

        let token = auth_header
            .or_else(|| parts.headers.get("x-goog-api-key").and_then(|h| h.to_str().ok()))
            .or_else(|| parts.headers.get("x-api-key").and_then(|h| h.to_str().ok()))
            .or_else(|| parts.headers.get("api-key").and_then(|h| h.to_str().ok()))
            .or_else(|| {
                parts.uri.query()
                    .and_then(|q| q.split('&').find(|pair| pair.starts_with("key=")))
                    .map(|pair| &pair[4..])
            });

        match token {
            Some(token) => {
                if let Some(ref master) = config.auth.master_key {
                    if token == master {
                        let client_id = parts.headers.get("X-Nexus-Client-Id")
                            .and_then(|h| h.to_str().ok())
                            .unwrap_or("master-user");
                            
                        return Ok(AuthToken(Claims {
                            sub: client_id.to_string(),
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
    model_registry: Arc<crate::model_registry::ModelRegistry>,
) -> Router {
    let http_client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    
    // Note: MCP currently still uses 'primary' pool or first pool for simplicity
    let mcp = BalancerMcpServer::new(
        pools.clone(), 
        config.clone(), 
        storage.clone(), 
        http_client.clone(),
        db.clone(),
    );
    
    let state = Arc::new(AppState { 
        pools, 
        auth,
        config,
        mcp,
        db,
        storage,
        http_client,
        model_registry,
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
        // Unified Gateway (Auto-routing by model)
        .route("/v1/*path", any(handle_unified_proxy))
        .route("/v1beta/*path", any(handle_unified_proxy))
        .route("/v1/models", get(handle_list_models))
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
        (status = 400, description = "Invalid key or unsupported provider"),
        (status = 404, description = "Pool not found"),
        (status = 403, description = "Forbidden")
    ),
    security(("admin_key" = []))
)]
async fn handle_import_key(
    State(state): State<Arc<AppState>>,
    token: AuthToken,
    Path(pool_name): Path<String>,
    Json(payload): Json<KeyImportRequest>,
) -> impl IntoResponse {
    let result = state.mcp.import_key(&token.0.sub, &pool_name, payload.key, payload.secret, payload.provider, payload.kv_cache).await;
    match result {
        Ok(msg) => (StatusCode::CREATED, msg).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
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
            let provider = params["provider"].as_str().map(|s| s.to_string());
            let kv_cache = params.get("kv_cache").and_then(|v| v.as_bool());
            
            match key_cfg_res {
                Ok(key_cfg) => {
                    match state.mcp.import_key(&token.0.sub, pool_name, key_cfg, secret, provider, kv_cache).await {

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

async fn handle_list_models(
    State(state): State<Arc<AppState>>,
    token: AuthToken,
) -> Json<serde_json::Value> {
    let allowed_pools = if token.0.role.as_deref() == Some("admin") {
        None
    } else {
        Some(state.db.get_allowed_pools(&token.0.sub).await.unwrap_or_default())
    };

    let models = state.db.get_all_models().await.unwrap_or_default();

    let data: Vec<serde_json::Value> = models.into_iter()
        .filter(|m| {
            allowed_pools.as_ref().map_or(true, |ap| ap.contains(&m.pool_name))
        })
        .map(|m| {
            let mut entry = serde_json::json!({
                "id": m.model_id,
                "object": "model",
                "owned_by": m.owned_by,
                "pool_name": m.pool_name,
            });
            if let Some(cw) = m.context_window {
                entry["context_window"] = serde_json::json!(cw);
            }
            entry
        })
        .collect();

    Json(serde_json::json!({"object": "list", "data": data}))
}

/// Parse model name for explicit provider routing: `//provider//real_model_name`
/// Returns (explicit_provider, real_model_name).
/// If no `//provider//` prefix, returns (None, original_model).
fn parse_explicit_model(model: &str) -> (Option<String>, String) {
    if model.starts_with("//") {
        let rest = &model[2..];
        if let Some(end) = rest.find("//") {
            let provider = rest[..end].to_string();
            let real_model = rest[end + 2..].to_string();
            if !provider.is_empty() && !real_model.is_empty() {
                return (Some(provider.to_lowercase()), real_model);
            }
        }
    }
    (None, model.to_string())
}

/// Unified Gateway Handler: Routes requests based on the 'model' field in the body
async fn handle_unified_proxy(
    State(state): State<Arc<AppState>>,
    token: AuthToken,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> impl axum::response::IntoResponse {
    let path = uri.path().to_string();
    let mut body_bytes = to_bytes(body, 25 * 1024 * 1024).await.unwrap_or_default();

    // 1. Try to detect model from body
    let mut model_name = if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
        json["model"].as_str().map(|s| s.to_string())
    } else {
        None
    };

    // 2. Check for explicit provider routing via `//provider//model` format
    let mut explicit_provider: Option<String> = None;
    if let Some(ref model) = model_name.clone() {
        let (provider, real_model) = parse_explicit_model(model);
        if let Some(prov) = provider {
            explicit_provider = Some(prov);
            // Rewrite body with real model name (without //provider// prefix)
            if let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                if let Some(obj) = json.as_object_mut() {
                    obj.insert("model".to_string(), serde_json::Value::String(real_model.clone()));
                    body_bytes = Bytes::from(serde_json::to_vec(&json).unwrap_or(body_bytes.to_vec()));
                    model_name = Some(real_model);
                }
            }
        }
    }

    // 3. If not in body, try to detect from path (Google Gemini style: /v1beta/models/...)
    if model_name.is_none() {
        if let Some(idx) = path.find("/models/") {
            let model_part = &path[idx + 8..];
            // Take up to the first ':' or '/'
            let model = model_part.split(':').next().unwrap_or(model_part).split('/').next().unwrap_or(model_part);
            model_name = Some(model.to_string());
        }
    }

    // 4. Routing Logic: Find the best pool
    let config = state.config.load();

    // Get allowed pools for this client
    let allowed_pools = if token.0.role.as_deref() == Some("admin") {
        None // Admin sees all
    } else {
        Some(state.db.get_allowed_pools(&token.0.sub).await.unwrap_or_default())
    };

    let find_pool = |providers: &[&str]| {
        config.pools.iter()
            .filter(|p| providers.contains(&p.provider.as_str()))
            .filter(|p| allowed_pools.as_ref().map_or(true, |allowed| allowed.contains(&p.name)))
            .next()
            .map(|p| p.name.clone())
    };

    let pool_name = if let Some(ref provider) = explicit_provider {
        // Explicit provider from //provider//model
        find_pool(&[provider.as_str()])
    } else if let Some(ref model) = model_name {
        // Data-driven routing via ModelRegistry
        state.model_registry.resolve_model_filtered(model, allowed_pools.as_ref())
            .or_else(|| {
                // Fallback: try to find by provider name heuristics if model not in registry
                let model_low = model.to_lowercase();
                if model_low.starts_with("gpt-") || model_low.starts_with("o1-") || model_low.starts_with("text-davinci") {
                    find_pool(&["openai"])
                } else if model_low.starts_with("claude-") {
                    find_pool(&["anthropic", "claude"])
                } else if model_low.starts_with("gemini-") {
                    find_pool(&["google", "gemini"])
                } else if model_low.starts_with("deepseek-") {
                    find_pool(&["deepseek"])
                } else if model_low.starts_with("mistral-") || model_low.starts_with("codestral-") || model_low.starts_with("pixtral-") || model_low.starts_with("ministral-") || model_low.starts_with("open-mixtral-") {
                    find_pool(&["mistral"])
                } else {
                    None
                }
            })
    } else {
        None
    };

    println!(" [{}] [DEBUG] Routing request to pool: '{}' for model: '{:?}' for client: '{}'", Local::now().format("%H:%M:%S%.3f"), pool_name.as_deref().unwrap_or("none"), model_name, token.0.sub);

    // Fallback to first allowed pool if no match or no model
    let pool_name = pool_name.or_else(|| {
        config.pools.iter()
            .filter(|p| allowed_pools.as_ref().map_or(true, |allowed| allowed.contains(&p.name)))
            .next()
            .map(|p| p.name.clone())
    });

    let Some(pool_name) = pool_name else {
        return (StatusCode::FORBIDDEN, "No authorized pools available for routing").into_response();
    };

    // 5. Delegate to the standard proxy handler (re-using the logic)
    // We create a new Path params map for handle_proxy
    let mut params = HashMap::new();
    params.insert("pool_name".to_string(), pool_name);

    // the request path may already contain `/v1` prefix from unified routing
    // e.g. `/v1/chat/completions`. We pass it entirely to handle_proxy_internal.
    // handle_proxy_internal will append this to the target_url.
    // if target_url is `https://api.mistral.ai/v1`, it becomes `https://api.mistral.ai/v1/v1/chat/completions`.
    // to fix this, strip `/v1` if target_url also ends with `/v1`.
    // We do this cleanup inside `build_target_url` to be safe for all providers.

    params.insert("path".to_string(), path);

    handle_proxy_internal(state, token, method, params, uri, headers, body_bytes).await
}

async fn handle_proxy(
    State(state): State<Arc<AppState>>,
    token: AuthToken,
    method: Method,
    Path(params): Path<HashMap<String, String>>,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> impl IntoResponse {
    let body_bytes = to_bytes(body, 25 * 1024 * 1024).await.unwrap_or_default();
    handle_proxy_internal(state, token, method, params, uri, headers, body_bytes).await
}

async fn handle_proxy_internal(
    state: Arc<AppState>,
    token: AuthToken,
    method: Method,
    params: HashMap<String, String>,
    uri: Uri,
    headers: HeaderMap,
    body_bytes: Bytes,
) -> Response<Body> {
    let start_time = Instant::now();
    println!(" [{}] [DEBUG] Proxy: Processing request (Body size: {} bytes)", Local::now().format("%H:%M:%S%.3f"), body_bytes.len());

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
    
    // Authorization & KV Cache Check
    let mut kv_cache_enabled = false;
    if token.0.sub != "admin-bypass" {
        let allowed: HashMap<String, bool> = state.db.get_allowed_pools_ext(&token.0.sub).await.unwrap_or_default();
        
        // If not admin, strictly check authorization
        if token.0.role.as_deref() != Some("admin") {
            if !allowed.contains_key(&pool_name) {
                return (StatusCode::FORBIDDEN, format!("Client '{}' is not authorized to use pool '{}'", token.0.sub, pool_name)).into_response();
            }
        }
        
        kv_cache_enabled = allowed.get(&pool_name).copied().unwrap_or(false);
        if kv_cache_enabled {
            println!(" [{}] [DEBUG] KV Cache is ENABLED for client: '{}' on pool: '{}'", Local::now().format("%H:%M:%S%.3f"), token.0.sub, pool_name);
        }
    }

    let target_url = build_target_url(target_base, &path, uri.query());
    // Acquire key from the specific pool
    let pool = match state.pools.get(&pool_name) {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "Pool implementation not found").into_response(),
    };
    let key = pool.acquire().await;
    let acquire_elapsed = start_time.elapsed();
    let key_id = key.id();

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

    // Determine final target URL and headers
    let is_google = provider == "google" || provider == "gemini" || target_url.contains("googleapis.com");
    let is_gemini_openai = is_google && target_url.contains("/openai/");
    let mut final_url = target_url;

    if is_google && !is_gemini_openai {
        if kv_cache_enabled {
            // Context Caching in Gemini requires v1beta
            if final_url.contains("/v1/") {
                final_url = final_url.replace("/v1/", "/v1beta/");
            } else if final_url.contains("/v1alpha/") {
                final_url = final_url.replace("/v1alpha/", "/v1beta/");
            } else if !final_url.contains("/v1beta/") {
                // If no version found in path, try to inject it if it's a standard models path
                if final_url.contains("/models/") {
                   final_url = final_url.replace("/models/", "/v1beta/models/");
                }
            }
        }

        // Gemini often requires ?key= in addition to or instead of headers
        let separator = if final_url.contains('?') { '&' } else { '?' };
        if !final_url.contains("key=") {
            final_url = format!("{}{}key={}", final_url, separator, secret);
        }
    }

    // Collect headers to forward (once, before retry loop)
    let forward_headers: Vec<(String, String)> = headers.iter()
        .filter_map(|(name, value)| {
            if should_forward_request_header(name) {
                Some((name.as_str().to_string(), value.to_str().unwrap_or("").to_string()))
            } else {
                None
            }
        })
        .collect();

    let max_retries = if !is_streaming_request(&body_bytes)
        && (method == Method::GET || method == Method::HEAD)
    {
        2
    } else {
        0
    };
    let mut last_error = String::new();
    let mut res: Option<reqwest::Response> = None;

    for attempt in 0..=max_retries {
        if attempt > 0 {
            let delay = Duration::from_millis(500 * 2u64.pow(attempt as u32 - 1));
            tokio::time::sleep(delay).await;
            println!(" [{}] [DEBUG] Proxy: Retry attempt {}/{} for pool '{}'", Local::now().format("%H:%M:%S%.3f"), attempt, max_retries, pool_name);
        }

        let mut req_builder = state.http_client.request(method.clone(), &final_url);

        for (name, value) in &forward_headers {
            req_builder = req_builder.header(name.as_str(), value.as_str());
        }

        if is_gemini_openai {
            req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", secret));
        } else if is_google {
            req_builder = req_builder.header("x-goog-api-key", &secret);
        } else if provider == "anthropic" || provider == "claude" || final_url.contains("anthropic.com") {
            req_builder = req_builder.header("x-api-key", &secret);
            req_builder = req_builder.header("anthropic-version", "2023-06-01");
        } else {
            req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", secret));
        }

        let result = timeout(
            Duration::from_secs(60),
            req_builder.body(body_bytes.clone()).send(),
        ).await;

        match result {
            Ok(Ok(resp)) => {
                res = Some(resp);
                break;
            }
            Ok(Err(e)) => {
                last_error = format!("error sending request: {}", e);
                eprintln!(" [{}] [WARN] Proxy: Upstream request failed (attempt {}/{}): {}", Local::now().format("%H:%M:%S%.3f"), attempt + 1, max_retries + 1, last_error);
            }
            Err(_) => {
                last_error = "request timeout after 60s".to_string();
                eprintln!(" [{}] [WARN] Proxy: Upstream request timeout (attempt {}/{})", Local::now().format("%H:%M:%S%.3f"), attempt + 1, max_retries + 1);
            }
        }
    }

    match res {
        Some(resp) => {
            let status = resp.status();
            let mut res_builder = Response::builder().status(status);
            let is_sse = resp
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.starts_with("text/event-stream"))
                .unwrap_or(false);
            
            let upstream_elapsed = start_time.elapsed();
            println!(" [{}] [DEBUG] Proxy: Upstream status {}, SSE: {}, Acquire: {:?}, Total: {:?}", 
                     Local::now().format("%H:%M:%S%.3f"), status, is_sse, acquire_elapsed, upstream_elapsed);
            
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
                let status_for_log = if status.is_success() {
                    "success".to_string()
                } else {
                    status.as_str().to_string()
                };

                let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(16);
                tokio::spawn(async move {
                    let mut upstream = resp.bytes_stream();
                    let mut collected = Vec::<u8>::new();
                    let mut final_status = status_for_log;
                    let mut error_message = None;

                    loop {
                        tokio::select! {
                            _ = tx.closed() => {
                                final_status = "client_disconnected".to_string();
                                break;
                            }
                            next = upstream.next() => {
                                match next {
                                    Some(Ok(chunk)) => {
                                        collected.extend_from_slice(&chunk);
                                        if tx.send(Ok(chunk)).await.is_err() {
                                            final_status = "client_disconnected".to_string();
                                            break;
                                        }
                                    }
                                    Some(Err(err)) => {
                                        let message = err.to_string();
                                        final_status = "stream_error".to_string();
                                        error_message = Some(message.clone());
                                        let _ = tx.send(Err(std::io::Error::other(message))).await;
                                        break;
                                    }
                                    None => break,
                                }
                            }
                        }
                    }

                    let total_tokens = extract_response_tokens(&collected, input_tokens);
                    key_for_stream.record_usage(total_tokens);
                    pool.release(key_for_stream).await;
                    if let Err(e) = db.log_request(crate::db::LogEntry {
                        client_id: Some(client_id),
                        key_id: Some(key_id_for_log),
                        pool_id: Some(pool_name_for_log),
                        status: final_status,
                        tokens_used: total_tokens,
                        error_message,
                        ..Default::default()
                    }).await {
                        eprintln!("Failed to log streaming request to DB: {}", e);
                    }
                });

                let stream = futures::stream::unfold(rx, |mut rx| async move {
                    rx.recv().await.map(|chunk| (chunk, rx))
                });

                res_builder.body(Body::from_stream(stream)).unwrap().into_response()
            } else {
                let bytes = match timeout(Duration::from_secs(120), resp.bytes()).await {
                    Ok(Ok(b)) => b,
                    Ok(Err(e)) => {
                        eprintln!(" [{}] [ERROR] Proxy: Failed to read response body: {}", Local::now().format("%H:%M:%S%.3f"), e);
                        key.record_usage(0);
                        state.pools.get(&pool_name).unwrap().release(key).await;
                        return (StatusCode::BAD_GATEWAY, format!("Failed to read upstream response body: {}", e)).into_response();
                    }
                    Err(_) => {
                        eprintln!(" [{}] [ERROR] Proxy: Timeout reading response body", Local::now().format("%H:%M:%S%.3f"));
                        key.record_usage(0);
                        state.pools.get(&pool_name).unwrap().release(key).await;
                        return (StatusCode::BAD_GATEWAY, "Timeout reading upstream response body".to_string()).into_response();
                    }
                };
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
        None => {
            state.pools.get(&pool_name).unwrap().release(key).await;
            (StatusCode::BAD_GATEWAY, format!("502 Upstream error after {} retries: {}", max_retries, last_error)).into_response()
        }
    }
}
