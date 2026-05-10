use axum::{
    extract::{State, FromRequestParts},
    http::{request::Parts, StatusCode, header::AUTHORIZATION},
    routing::{get, post, patch},
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
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Deserialize)]
pub struct ExecuteRequest {
    pub task_name: String,
}

#[derive(Debug, Serialize)]
pub struct ExecuteResponse {
    pub status: String,
    pub key_id: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct ConfigPatchRequest {
    pub server: Option<crate::config::ServerConfig>,
    pub auth: Option<crate::config::AuthConfig>,
}

pub struct AppState {
    pub pool: KeyPool,
    pub auth: AuthManager,
    pub config: Arc<ArcSwap<AppConfig>>,
    pub mcp: BalancerMcpServer,
}

/// OAuth 2.1 Bearer Token extractor
pub struct AuthToken(pub Claims);

#[async_trait]
impl<S> FromRequestParts<S> for AuthToken
where
    Arc<AppState>: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = Arc::from_ref(state);
        
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
            None => Err((StatusCode::UNAUTHORIZED, "Missing Bearer token in Authorization header".to_string())),
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

pub fn create_router(pool: KeyPool, auth: AuthManager, config: Arc<ArcSwap<AppConfig>>) -> Router {
    let mcp = BalancerMcpServer::new(pool.clone(), config.clone());
    let state = Arc::new(AppState { 
        pool, 
        auth,
        config,
        mcp,
    });
    
    Router::new()
        .route("/execute", post(handle_execute))
        .route("/stats", get(handle_stats))
        .route("/config", get(handle_get_config).patch(handle_patch_config))
        .route("/mcp", post(handle_mcp)) // Standard MCP over HTTP endpoint
        .with_state(state)
}

async fn handle_execute(
    State(state): State<Arc<AppState>>,
    _token: AuthToken,
    Json(payload): Json<ExecuteRequest>,
) -> Json<ExecuteResponse> {
    let pool = &state.pool;
    let key = pool.acquire().await;
    
    if let Err(e) = key.try_use() {
        pool.release(key).await;
        return Json(ExecuteResponse {
            status: "error".to_string(),
            key_id: "none".to_string(),
            message: format!("Rate limit hit: {}", e),
        });
    }

    sleep(Duration::from_millis(50)).await;
    
    let key_id = key.id();
    pool.release(key).await;

    Json(ExecuteResponse {
        status: "success".to_string(),
        key_id,
        message: format!("Task '{}' completed safely", payload.task_name),
    })
}

async fn handle_stats(_token: AuthToken) -> &'static str {
    "Pool stats: Operational"
}

async fn handle_get_config(
    State(state): State<Arc<AppState>>,
    _token: AuthToken,
) -> Json<AppConfig> {
    let mut config = (**state.config.load()).clone();
    config.auth.secret = "[REDACTED]".to_string();
    Json(config)
}

async fn handle_patch_config(
    State(state): State<Arc<AppState>>,
    _token: AuthToken,
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
    _token: AuthToken,
    Json(request): Json<McpRequest>,
) -> Json<McpResponse> {
    let result = match request.method.as_str() {
        "list_pools" => Some(serde_json::to_value(state.mcp.list_pools().await).unwrap()),
        "get_config" => Some(state.mcp.get_config_resource().await),
        "update_description" => {
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
