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
    pub http_client: reqwest::Client,
    pub db: crate::db::Database,
}

impl BalancerMcpServer {
    pub fn new(
        pools: std::collections::HashMap<String, KeyPool>, 
        config: Arc<ArcSwap<AppConfig>>,
        storage: crate::storage::SecretStorage,
        http_client: reqwest::Client,
        db: crate::db::Database,
    ) -> Self {
        Self { pools, config, storage, http_client, db }
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

    pub async fn import_key(
        &self, 
        client_id: &str,
        pool_name: &str, 
        mut key_cfg: crate::config::KeyConfig, 
        secret: String,
        provider: Option<String>,
        kv_cache: Option<bool>
    ) -> Result<String, String> {
        let mut new_config = (**self.config.load()).clone();
        let pool_idx = new_config.pools.iter().position(|p| p.name == pool_name);

        // 1. Determine provider and URL
        let (provider_name, target_url) = if let Some(idx) = pool_idx {
            (new_config.pools[idx].provider.clone(), new_config.pools[idx].target_url.clone())
        } else if let Some(p) = provider {
            if let Some(url) = AppConfig::get_standard_url(&p) {
                (p, url.to_string())
            } else {
                return Err(format!("Unsupported provider: {}", p));
            }
        } else {
            return Err(format!("Pool '{}' not found. Specify 'provider' to create it.", pool_name));
        };

        // 2. Parse and Validate Keys
        let keys: Vec<&str> = if secret.contains(',') {
            secret.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect()
        } else {
            secret.lines().map(|s| s.trim()).filter(|s| !s.is_empty()).collect()
        };

        if keys.is_empty() {
            return Err("Invalid key format. Use 'key1, key2' or 'key1'".to_string());
        }

        // Validate each key
        for key_str in &keys {
            crate::utils::verify_key(&self.http_client, &provider_name, &target_url, key_str).await
                .map_err(|e| format!("Key verification failed for one of the keys: {}", e))?;
        }

        // 3. Save secrets using client-specific directory
        let secret_path = self.storage.save_secret_for_client(client_id, &provider_name, &secret)
            .map_err(|e| format!("Failed to save secret: {}", e))?;
        
        key_cfg.secret_name = secret_path;

        // 4. Update config
        if let Some(idx) = pool_idx {
            // Check if we already have this secret_name in the pool to avoid duplicates in config
            if !new_config.pools[idx].keys.iter().any(|k| k.secret_name == key_cfg.secret_name) {
                new_config.pools[idx].keys.push(key_cfg.clone());
            }

            // Save KV Cache setting if provided
            if let Some(enabled) = kv_cache {
                if let Err(e) = self.db.set_pool_kv_cache(client_id, pool_name, enabled).await {
                    eprintln!("Warning: Failed to set KV cache in DB: {}", e);
                }
            }
            
            new_config.save("config.yaml").map_err(|e| format!("Failed to save config: {}", e))?;

            // 5. Update running pool
            if let Some(pool) = self.pools.get(pool_name) {
                for key_str in &keys {
                    let key = crate::core::ApiKey::new(
                        &key_cfg.id,
                        key_cfg.rps_limit,
                        key_cfg.rpd_limit,
                        key_cfg.tpm_limit,
                        key_cfg.tpd_limit,
                        key_cfg.max_request_tokens,
                        key_cfg.cooldown_on_limit.unwrap_or(false),
                        key_str.to_string(),
                        key_cfg.secret_type.clone(),
                        None,
                    );
                    for _ in 0..key_cfg.concurrency {
                        let _ = pool.add_key(key.clone());
                    }
                }
            }

            self.config.store(Arc::new(new_config));
            Ok(format!("Successfully imported {} keys into pool '{}'", keys.len(), pool_name))
        } else {
            // Auto-create pool if not exists
            let new_pool = crate::config::PoolConfig {
                name: pool_name.to_string(),
                description: Some(format!("Auto-created pool for {}", provider_name)),
                provider: provider_name.clone(),
                target_url: target_url.clone(),
                capacity: 20,
                keys: vec![key_cfg.clone()],
            };
            
            new_config.pools.push(new_pool);
            new_config.save("config.yaml").map_err(|e| format!("Failed to save config: {}", e))?;
            self.config.store(Arc::new(new_config));
            
            // Also need to register the relationship in DB if not admin? 
            // For now, let's just say restart is needed for auto-created pools
            Ok(format!("Pool '{}' created. Please restart to activate.", pool_name))
        }
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
