mod core;
mod config;
mod storage;
mod api;
mod auth;
mod mcp;
mod db;

use anyhow::Result;
use crate::config::AppConfig;
use crate::storage::SecretStorage;
use crate::core::{ApiKey, KeyPool};
use crate::auth::AuthManager;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use std::sync::Arc;
use arc_swap::ArcSwap;
use crate::db::Database;

#[tokio::main]
async fn main() -> Result<()> {
    // 0. Load Environment Variables
    dotenvy::dotenv().ok();

    // 1. Load Config
    let config = AppConfig::load("config.yaml")?;
    println!("Loaded config from config.yaml");
    
    let shared_config = Arc::new(ArcSwap::from(Arc::new(config.clone())));
    
    // 1.5 Initialize Database
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:nexus.db".to_string());
    let db = Database::new(&db_url).await?;
    println!("Database initialized and migrated.");

    // 2. Initialize Secret Storage
    let storage = SecretStorage::new("secrets");

    // 3. Initialize Pools
    let pool_config = &config.pools[0];
    let pool = KeyPool::new(pool_config.capacity);

    for key_cfg in &pool_config.keys {
        let secret = storage.load_secret(&key_cfg.secret_name)?;
        let key = ApiKey::new(
            &key_cfg.id,
            key_cfg.limit,
            secret,
            key_cfg.secret_type.clone(),
            None,
        );

        for _ in 0..key_cfg.concurrency {
            pool.add_key(key.clone()).await;
        }
    }

    println!("Initialized pool '{}' with {} slots", pool_config.name, pool_config.keys.len());

    // 4. Initialize Auth
    let auth_manager = AuthManager::new(config.auth.clone());

    // 5. Start REST API
    let app = api::create_router(pool, auth_manager, shared_config, db);
    let addr = SocketAddr::from(([127, 0, 0, 1], config.server.port));
    
    println!("Starting REST API & MCP Server on http://{}", addr);
    
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}