pub mod core;
pub mod config;
pub mod storage;
pub mod api;
pub mod auth;
pub mod mcp;
pub mod db;
pub mod utils;
pub mod mcp_client;
pub mod model_registry;

use anyhow::Result;
use crate::config::AppConfig;
use crate::storage::SecretStorage;
use crate::core::{ApiKey, KeyPool};
use crate::auth::AuthManager;
use crate::db::Database;
use crate::model_registry::ModelRegistry;
use std::collections::HashMap;
use std::sync::Arc;
use arc_swap::ArcSwap;
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};
use crate::api::ApiDoc;

pub async fn run_server(config: AppConfig, db: Database, storage_path: &str) -> Result<()> {
    let shared_config = Arc::new(ArcSwap::from(Arc::new(config.clone())));
    let storage = SecretStorage::new(storage_path);
    let http_client = reqwest::Client::new();
    let mut pools = HashMap::new();

    for pool_cfg in &config.pools {
        // Automatically calculate required capacity to prevent capacity exceeded errors
        let mut required_capacity = 0;
        for key_cfg in &pool_cfg.keys {
            let secret_count = storage.load_secret(&key_cfg.secret_name)
                .map(|content| content.lines().map(|s| s.trim()).filter(|s| !s.is_empty()).count())
                .unwrap_or(1);
            required_capacity += secret_count * key_cfg.concurrency;
        }
        let final_capacity = std::cmp::max(pool_cfg.capacity, required_capacity);

        let pool = KeyPool::new(final_capacity);
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

    // Initialize Model Registry
    let model_registry = Arc::new(ModelRegistry::new(
        db.clone(),
        shared_config.clone(),
        http_client.clone(),
        storage.clone(),
    ));

    // Count discovered models for startup banner (initial sync handled by spawn_periodic_sync)
    let model_count = db.get_all_models().await.map(|m| m.len()).unwrap_or(0);
    let provider_count = {
        let mut providers: Vec<String> = config.pools.iter()
            .filter(|p| !p.skip_model_sync)
            .map(|p| p.provider.clone())
            .collect();
        providers.sort();
        providers.dedup();
        providers.len()
    };

    // Spawn periodic rebase every 6 hours
    model_registry.spawn_periodic_sync();

    let auth_manager = AuthManager::new(config.auth.clone());
    let cors_origin = config.server.cors_allowed_origin.clone();
    let app = api::create_router(pools, auth_manager, shared_config, db, storage, model_registry)
        .merge(Scalar::with_url("/scalar", ApiDoc::openapi()))
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::AllowOrigin::predicate(
                    move |origin: &axum::http::HeaderValue, _: &axum::http::request::Parts| {
                        origin
                            .to_str()
                            .map(|o| o == cors_origin || o == "http://localhost:3317")
                            .unwrap_or(false)
                    },
                ))
        );
    
    let addr_str = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr_str).await?;

    // Update the banner to show model count
    let green = "\x1b[32m";
    let reset = "\x1b[0m";
    println!("   {}Models:  {} discovered across {} providers{}", green, model_count, provider_count, reset);

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
        })
        .await?;

    Ok(())
}
