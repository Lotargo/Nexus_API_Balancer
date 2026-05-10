use serde::{Deserialize, Serialize};
use crate::core::KeyPool;
use crate::config::AppConfig;
use std::sync::Arc;
use arc_swap::ArcSwap;

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateDescriptionArgs {
    pub pool_name: String,
    pub description: String,
}

pub struct BalancerMcpServer {
    pub pool: KeyPool,
    pub config: Arc<ArcSwap<AppConfig>>,
}

impl BalancerMcpServer {
    pub fn new(pool: KeyPool, config: Arc<ArcSwap<AppConfig>>) -> Self {
        Self { pool, config }
    }

    /// MCP Tool: List all pools with their descriptions to help agents identify them.
    pub async fn list_pools(&self) -> Vec<serde_json::Value> {
        let config = self.config.load();
        config.pools.iter().map(|p| {
            serde_json::json!({
                "name": p.name,
                "description": p.description.clone().unwrap_or_else(|| "No description provided".to_string()),
                "key_count": p.keys.len(),
                "capacity": p.capacity
            })
        }).collect()
    }

    /// MCP Tool: Allows an agent to add or update a description for a specific pool.
    pub async fn update_pool_description(&self, args: UpdateDescriptionArgs) -> Result<String, String> {
        let current_config = self.config.load();
        let mut new_config = (**current_config).clone();

        if let Some(pool) = new_config.pools.iter_mut().find(|p| p.name == args.pool_name) {
            pool.description = Some(args.description.clone());
            self.config.store(Arc::new(new_config));
            Ok(format!("Description for pool '{}' updated successfully", args.pool_name))
        } else {
            Err(format!("Pool '{}' not found", args.pool_name))
        }
    }

    /// MCP Resource: Provides the full configuration schema to the agent.
    pub async fn get_config_resource(&self) -> serde_json::Value {
        let config = self.config.load();
        let mut sanitized = (**config).clone();
        sanitized.auth.secret = "[REDACTED]".to_string();
        serde_json::to_value(sanitized).unwrap()
    }
}

// Basic MCP JSON-RPC structures for transport integration
#[derive(Debug, Deserialize)]
pub struct McpRequest {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct McpResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
}
