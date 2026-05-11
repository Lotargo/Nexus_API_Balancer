use anyhow::Result;
use nexus_balancer::config::AppConfig;
use nexus_balancer::db::Database;
use nexus_balancer::run_server;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let config = AppConfig::load("config.yaml")?;
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:nexus.db".to_string());
    let db = Database::new(&db_url).await?;
    run_server(config, db, "secrets").await
}