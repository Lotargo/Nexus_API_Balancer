use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use anyhow::Result;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone)]
pub struct Database {
    pub pool: SqlitePool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub client_id: Option<String>,
    pub key_id: Option<String>,
    pub pool_id: Option<String>,
    pub status: String,
    pub latency_ms: Option<i64>,
    pub error_message: Option<String>,
    pub request_ip: Option<String>,
}

impl Database {
    pub async fn new(db_url: &str) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(db_url)
            .await?;

        // Run migrations
        sqlx::migrate!("./migrations").run(&pool).await?;

        Ok(Self { pool })
    }

    pub async fn log_request(&self, entry: LogEntry) -> Result<()> {
        sqlx::query(
            "INSERT INTO request_logs (client_id, key_id, pool_id, status, latency_ms, error_message, request_ip)
             VALUES (?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(entry.client_id)
        .bind(entry.key_id)
        .bind(entry.pool_id)
        .bind(entry.status)
        .bind(entry.latency_ms)
        .bind(entry.error_message)
        .bind(entry.request_ip)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_stats(&self) -> Result<serde_json::Value> {
        let total_requests: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM request_logs")
            .fetch_one(&self.pool)
            .await?;

        let success_rate: f64 = sqlx::query_scalar(
            "SELECT CAST(COUNT(*) AS FLOAT) / (SELECT COUNT(*) FROM request_logs) FROM request_logs WHERE status = 'success'"
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0.0);

        Ok(serde_json::json!({
            "total_requests": total_requests,
            "success_rate": success_rate,
        }))
    }

    pub async fn get_allowed_pools(&self, client_id: &str) -> Result<Vec<String>> {
        let pools: Vec<String> = sqlx::query_scalar(
            "SELECT pool_id FROM client_pools WHERE client_id = ?"
        )
        .bind(client_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(pools)
    }
}
