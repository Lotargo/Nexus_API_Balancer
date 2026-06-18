use sqlx::{sqlite::{SqlitePoolOptions, SqliteConnectOptions}, SqlitePool, Row};
use std::str::FromStr;
use std::collections::HashMap;
use anyhow::Result;
use serde::{Serialize, Deserialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModel {
    pub provider_name: String,
    pub pool_name: String,
    pub model_id: String,
    pub owned_by: Option<String>,
    pub context_window: Option<i64>,
    pub capabilities: Option<String>,
}

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
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(5));
        
        if db_url.contains(":memory:") {
            opts = opts.shared_cache(true);
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .after_connect(|conn, _meta| Box::pin(async move {
                sqlx::query("PRAGMA foreign_keys = ON").execute(&mut *conn).await?;
                sqlx::query("PRAGMA busy_timeout = 5000").execute(&mut *conn).await?;
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
                client_id TEXT NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
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

    pub async fn upsert_provider_model(&self, model: &ProviderModel) -> Result<()> {
        sqlx::query(
            "INSERT INTO provider_models (provider_name, pool_name, model_id, owned_by, context_window, capabilities, is_stale)
             VALUES (?, ?, ?, ?, ?, ?, 0)
             ON CONFLICT(provider_name, model_id) DO UPDATE SET
                pool_name = excluded.pool_name,
                owned_by = excluded.owned_by,
                context_window = excluded.context_window,
                capabilities = excluded.capabilities,
                is_stale = 0,
                fetched_at = CURRENT_TIMESTAMP"
        )
        .bind(&model.provider_name)
        .bind(&model.pool_name)
        .bind(&model.model_id)
        .bind(&model.owned_by)
        .bind(model.context_window)
        .bind(&model.capabilities)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_provider_stale(&self, provider_name: &str) -> Result<()> {
        sqlx::query("UPDATE provider_models SET is_stale = 1 WHERE provider_name = ?")
            .bind(provider_name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn cleanup_stale_models(&self) -> Result<u64> {
        let result = sqlx::query(
            "DELETE FROM provider_models WHERE is_stale = 1 AND fetched_at < datetime('now', '-24 hours')"
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn find_pools_for_model(&self, model_id: &str) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query(
            "SELECT pool_name, provider_name FROM provider_models WHERE model_id = ? AND is_stale = 0"
        )
        .bind(model_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    pub async fn get_models_by_provider(&self, provider_name: &str) -> Result<Vec<ProviderModel>> {
        let rows = sqlx::query(
            "SELECT provider_name, pool_name, model_id, owned_by, context_window, capabilities
             FROM provider_models WHERE provider_name = ? AND is_stale = 0"
        )
        .bind(provider_name)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| ProviderModel {
            provider_name: r.get(0),
            pool_name: r.get(1),
            model_id: r.get(2),
            owned_by: r.get(3),
            context_window: r.get(4),
            capabilities: r.get(5),
        }).collect())
    }

    pub async fn get_all_models(&self) -> Result<Vec<ProviderModel>> {
        let rows = sqlx::query(
            "SELECT provider_name, pool_name, model_id, owned_by, context_window, capabilities
             FROM provider_models WHERE is_stale = 0
             ORDER BY provider_name, model_id"
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| ProviderModel {
            provider_name: r.get(0),
            pool_name: r.get(1),
            model_id: r.get(2),
            owned_by: r.get(3),
            context_window: r.get(4),
            capabilities: r.get(5),
        }).collect())
    }

    pub async fn upsert_provider_models_batch(&self, _provider_name: &str, _pool_name: &str, models: &[ProviderModel]) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let mut count = 0;
        for model in models {
            sqlx::query(
                "INSERT INTO provider_models (provider_name, pool_name, model_id, owned_by, context_window, capabilities, is_stale)
                 VALUES (?, ?, ?, ?, ?, ?, 0)
                 ON CONFLICT(provider_name, model_id) DO UPDATE SET
                    pool_name = excluded.pool_name,
                    owned_by = excluded.owned_by,
                    context_window = excluded.context_window,
                    capabilities = excluded.capabilities,
                    is_stale = 0,
                    fetched_at = CURRENT_TIMESTAMP"
            )
            .bind(&model.provider_name)
            .bind(&model.pool_name)
            .bind(&model.model_id)
            .bind(&model.owned_by)
            .bind(model.context_window)
            .bind(&model.capabilities)
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        tx.commit().await?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqliteConnectOptions;
    use std::str::FromStr;

    async fn make_db() -> Database {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap()
            .create_if_missing(true)
            .shared_cache(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .after_connect(|conn, _meta| Box::pin(async move {
                sqlx::query("PRAGMA foreign_keys = ON").execute(&mut *conn).await?;
                sqlx::query("PRAGMA busy_timeout = 5000").execute(&mut *conn).await?;
                Ok(())
            }))
            .connect_with(opts)
            .await
            .unwrap();

        let db = Database { pool };

        // Create tables manually (without running migrations)
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS clients (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                auth_token TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&db.pool).await.unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS client_pools (
                client_id TEXT NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
                pool_id TEXT NOT NULL,
                kv_cache BOOLEAN NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (client_id, pool_id)
            )"
        ).execute(&db.pool).await.unwrap();

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
        ).execute(&db.pool).await.unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS provider_models (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                provider_name TEXT NOT NULL,
                pool_name TEXT NOT NULL,
                model_id TEXT NOT NULL,
                owned_by TEXT,
                context_window INTEGER,
                capabilities TEXT,
                is_stale BOOLEAN NOT NULL DEFAULT 0,
                fetched_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(provider_name, model_id)
            )"
        ).execute(&db.pool).await.unwrap();

        db
    }

    #[tokio::test]
    async fn test_register_and_get_client() {
        let db = make_db().await;
        db.register_client("client-1", "Test Client", "token-1").await.unwrap();

        let pools = db.get_allowed_pools("client-1").await.unwrap();
        assert!(pools.is_empty());
    }

    #[tokio::test]
    async fn test_set_and_get_pool_kv_cache() {
        let db = make_db().await;
        db.register_client("client-1", "Test Client", "token-1").await.unwrap();

        db.set_pool_kv_cache("client-1", "pool-1", true).await.unwrap();

        let pools = db.get_allowed_pools("client-1").await.unwrap();
        assert_eq!(pools, vec!["pool-1"]);

        let pools_ext = db.get_allowed_pools_ext("client-1").await.unwrap();
        assert_eq!(pools_ext.get("pool-1"), Some(&true));
    }

    #[tokio::test]
    async fn test_log_and_get_stats() {
        let db = make_db().await;
        db.register_client("client-1", "Test Client", "token-1").await.unwrap();

        db.log_request(LogEntry {
            client_id: Some("client-1".to_string()),
            key_id: Some("key-1".to_string()),
            pool_id: Some("pool-1".to_string()),
            status: "success".to_string(),
            latency_ms: Some(100),
            tokens_used: 50,
            error_message: None,
            request_ip: None,
        }).await.unwrap();

        let stats = db.get_stats().await.unwrap();
        assert_eq!(stats["total_requests"], 1);
        assert!(stats["total_tokens"].as_i64().unwrap_or(0) > 0);
    }

    #[tokio::test]
    async fn test_upsert_and_get_provider_models() {
        let db = make_db().await;

        let model = ProviderModel {
            provider_name: "test-provider".to_string(),
            pool_name: "test-pool".to_string(),
            model_id: "test-model-1".to_string(),
            owned_by: Some("test".to_string()),
            context_window: Some(4096),
            capabilities: None,
        };

        db.upsert_provider_model(&model).await.unwrap();

        let models = db.get_all_models().await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "test-model-1");
        assert_eq!(models[0].pool_name, "test-pool");

        let found = db.find_pools_for_model("test-model-1").await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "test-pool");
    }

    #[tokio::test]
    async fn test_mark_stale_and_cleanup() {
        let db = make_db().await;

        let model = ProviderModel {
            provider_name: "test-provider".to_string(),
            pool_name: "test-pool".to_string(),
            model_id: "test-model-1".to_string(),
            owned_by: None,
            context_window: None,
            capabilities: None,
        };

        db.upsert_provider_model(&model).await.unwrap();
        db.mark_provider_stale("test-provider").await.unwrap();

        // get_all_models filters by is_stale = 0, so stale models are excluded
        let models = db.get_all_models().await.unwrap();
        assert_eq!(models.len(), 0);

        // get_models_by_provider also filters stale
        let provider_models = db.get_models_by_provider("test-provider").await.unwrap();
        assert_eq!(provider_models.len(), 0);
    }

    #[tokio::test]
    async fn test_foreign_key_enforced() {
        let db = make_db().await;

        // client_pools requires a valid client_id reference
        let result = sqlx::query(
            "INSERT INTO client_pools (client_id, pool_id) VALUES (?, ?)"
        )
        .bind("nonexistent-client")
        .bind("pool-1")
        .execute(&db.pool)
        .await;

        // Should fail due to FK constraint
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_upsert_provider_models_batch() {
        let db = make_db().await;

        let models = vec![
            ProviderModel {
                provider_name: "provider-1".to_string(),
                pool_name: "pool-1".to_string(),
                model_id: "model-a".to_string(),
                owned_by: None,
                context_window: None,
                capabilities: None,
            },
            ProviderModel {
                provider_name: "provider-1".to_string(),
                pool_name: "pool-1".to_string(),
                model_id: "model-b".to_string(),
                owned_by: None,
                context_window: None,
                capabilities: None,
            },
        ];

        let count = db.upsert_provider_models_batch("provider-1", "pool-1", &models).await.unwrap();
        assert_eq!(count, 2);

        let all = db.get_all_models().await.unwrap();
        assert_eq!(all.len(), 2);
    }
}
