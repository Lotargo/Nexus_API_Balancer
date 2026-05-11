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
    pub pools: std::collections::HashMap<String, KeyPool>,
    pub config: Arc<ArcSwap<AppConfig>>,
    pub storage: crate::storage::SecretStorage,
}

impl BalancerMcpServer {
    pub fn new(
        pools: std::collections::HashMap<String, KeyPool>, 
        config: Arc<ArcSwap<AppConfig>>,
        storage: crate::storage::SecretStorage,
    ) -> Self {
        Self { pools, config, storage }
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
        if sanitized.auth.master_key.is_some() {
            sanitized.auth.master_key = Some("[REDACTED]".to_string());
        }
        serde_json::to_value(sanitized).unwrap()
    }

    /// MCP Tool: Export a key with its secret by ID.
    pub async fn export_key(&self, pool_name: &str, key_id: &str) -> Result<serde_json::Value, String> {
        let config = self.config.load();
        let pool = config.pools.iter().find(|p| p.name == pool_name)
            .ok_or_else(|| format!("Pool '{}' not found", pool_name))?;
        
        let key_cfg = pool.keys.iter().find(|k| k.id == key_id)
            .ok_or_else(|| format!("Key '{}' not found in pool '{}'", key_id, pool_name))?;
            
        let secret = self.storage.load_secret(&key_cfg.secret_name)
            .map_err(|e| format!("Failed to load secret: {}", e))?;
            
        Ok(serde_json::json!({
            "key": key_cfg,
            "secret": secret
        }))
    }

    /// MCP Tool: Import a new key into a pool.
    pub async fn import_key(&self, pool_name: &str, key_cfg: crate::config::KeyConfig, secret: String) -> Result<String, String> {
        // 1. Save secret
        self.storage.save_secret(&key_cfg.secret_name, &secret)
            .map_err(|e| format!("Failed to save secret: {}", e))?;

        // 2. Update config
        let mut new_config = (**self.config.load()).clone();
        let pool_idx = new_config.pools.iter().position(|p| p.name == pool_name)
            .ok_or_else(|| format!("Pool '{}' not found", pool_name))?;
        
        new_config.pools[pool_idx].keys.push(key_cfg.clone());
        
        // 3. Persist to disk
        new_config.save("config.yaml")
            .map_err(|e| format!("Failed to save config: {}", e))?;

        // 4. Update running pool
        if let Some(pool) = self.pools.get(pool_name) {
            let key = crate::core::ApiKey::new(
                &key_cfg.id,
                key_cfg.rps_limit,
                key_cfg.rpd_limit,
                key_cfg.tpm_limit,
                key_cfg.tpd_limit,
                key_cfg.max_request_tokens,
                key_cfg.cooldown_on_limit.unwrap_or(false),
                secret,
                key_cfg.secret_type.clone(),
                None,
            );
            for _ in 0..key_cfg.concurrency {
                let _ = pool.add_key(key.clone());
            }
        }

        self.config.store(Arc::new(new_config));
        Ok(format!("Key '{}' imported successfully into pool '{}'", key_cfg.id, pool_name))
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
