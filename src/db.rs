use sqlx::{sqlite::{SqlitePoolOptions, SqliteConnectOptions}, SqlitePool};
use std::str::FromStr;
use std::collections::HashMap;
use anyhow::Result;
use serde::{Serialize, Deserialize};
use utoipa::ToSchema;

#[derive(Debug, Clone)]
pub struct Database {
    pub pool: SqlitePool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, Default)]
pub struct LogEntry {
    pub client_id: Option<String>,
    pub key_id: Option<String>,
    pub pool_id: Option<String>,
    pub status: String,
    pub latency_ms: Option<i64>,
    pub tokens_used: u32,
    pub error_message: Option<String>,
    pub request_ip: Option<String>,
}

impl Database {
    pub async fn new(db_url: &str) -> Result<Self> {
        let mut opts = SqliteConnectOptions::from_str(db_url)?
            .create_if_missing(true);
        
        if db_url.contains(":memory:") {
            opts = opts.shared_cache(true);
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .after_connect(|conn, _meta| Box::pin(async move {
                sqlx::query("PRAGMA foreign_keys = OFF").execute(conn).await?;
                Ok(())
            }))
            .connect_with(opts)
            .await?;

        // Run migrations
        sqlx::migrate!("./migrations").run(&pool).await?;
        Self::ensure_schema(&pool).await?;

        Ok(Self { pool })
    }

    async fn ensure_schema(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS request_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                client_id TEXT,
                key_id TEXT,
                pool_id TEXT,
                status TEXT NOT NULL,
                latency_ms INTEGER,
                tokens_used INTEGER NOT NULL DEFAULT 0,
                error_message TEXT,
                request_ip TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )"
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS clients (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                auth_token TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )"
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS client_pools (
                client_id TEXT NOT NULL,
                pool_id TEXT NOT NULL,
                kv_cache BOOLEAN NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (client_id, pool_id)
            )"
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn set_pool_kv_cache(&self, client_id: &str, pool_id: &str, enabled: bool) -> Result<()> {
        sqlx::query(
            "INSERT INTO client_pools (client_id, pool_id, kv_cache) 
             VALUES (?, ?, ?)
             ON CONFLICT(client_id, pool_id) DO UPDATE SET kv_cache = excluded.kv_cache"
        )
        .bind(client_id)
        .bind(pool_id)
        .bind(enabled)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn log_request(&self, entry: LogEntry) -> Result<()> {
        sqlx::query(
            "INSERT INTO request_logs (client_id, key_id, pool_id, status, latency_ms, tokens_used, error_message, request_ip)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(entry.client_id)
        .bind(entry.key_id)
        .bind(entry.pool_id)
        .bind(entry.status)
        .bind(entry.latency_ms)
        .bind(entry.tokens_used)
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

        let total_tokens: i64 = sqlx::query_scalar("SELECT SUM(tokens_used) FROM request_logs")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);

        let success_rate: f64 = sqlx::query_scalar(
            "SELECT CAST(COUNT(*) AS FLOAT) / (SELECT COUNT(*) FROM request_logs) FROM request_logs WHERE status = 'success'"
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0.0);

        Ok(serde_json::json!({
            "total_requests": total_requests,
            "total_tokens": total_tokens,
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

    pub async fn get_allowed_pools_ext(&self, client_id: &str) -> Result<HashMap<String, bool>> {
        let rows: Vec<(String, bool)> = sqlx::query_as(
            "SELECT pool_id, kv_cache FROM client_pools WHERE client_id = ?"
        )
        .bind(client_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().collect())
    }

    pub async fn register_client(&self, id: &str, name: &str, token: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO clients (id, name, auth_token, status) VALUES (?, ?, ?, 'active')"
        )
        .bind(id)
        .bind(name)
        .bind(token)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
