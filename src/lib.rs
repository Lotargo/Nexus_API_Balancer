pub mod core;
pub mod config;
pub mod storage;
pub mod api;
pub mod auth;
pub mod mcp;
pub mod db;
pub mod utils;

use anyhow::Result;
use crate::config::AppConfig;
use crate::storage::SecretStorage;
use crate::core::{ApiKey, KeyPool};
use crate::auth::AuthManager;
use crate::db::Database;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use arc_swap::ArcSwap;
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};
use crate::api::ApiDoc;

pub async fn run_server(config: AppConfig, db: Database, storage_path: &str) -> Result<()> {
    let shared_config = Arc::new(ArcSwap::from(Arc::new(config.clone())));
    let storage = SecretStorage::new(storage_path);
    let mut pools = HashMap::new();

    for pool_cfg in &config.pools {
        let pool = KeyPool::new(pool_cfg.capacity);
        for key_cfg in &pool_cfg.keys {
            let secret_content = storage.load_secret(&key_cfg.secret_name)?;
            // Support multiple keys per file (one per line)
            let individual_secrets: Vec<&str> = secret_content.lines()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            let individual_secrets_count = individual_secrets.len();

            for (idx, secret) in individual_secrets.into_iter().enumerate() {
                let unique_id = if idx == 0 && individual_secrets_count == 1 {
                    key_cfg.id.clone()
                } else {
                    format!("{}#{}", key_cfg.id, idx + 1)
                };
                
                let key = ApiKey::new(
                    &unique_id,
                    key_cfg.rps_limit,
                    key_cfg.rpd_limit,
                    key_cfg.tpm_limit,
                    key_cfg.tpd_limit,
                    key_cfg.max_request_tokens,
                    key_cfg.cooldown_on_limit.unwrap_or(false),
                    secret.to_string(),
                    key_cfg.secret_type.clone(),
                    None,
                );
                for _ in 0..key_cfg.concurrency {
                    if let Err(e) = pool.add_key(key.clone()) {
                        eprintln!("Warning: Pool '{}' reached capacity during init: {}", pool_cfg.name, e);
                        break;
                    }
                }
            }
        }
        pools.insert(pool_cfg.name.clone(), pool);
    }

    let auth_manager = AuthManager::new(config.auth.clone());
    let app = api::create_router(pools, auth_manager, shared_config, db, storage)
        .merge(Scalar::with_url("/scalar", ApiDoc::openapi()));
    
    let addr = SocketAddr::from(([127, 0, 0, 1], config.server.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
        })
        .await?;

    Ok(())
}
