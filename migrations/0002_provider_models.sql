CREATE TABLE IF NOT EXISTS provider_models (
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
);

CREATE INDEX IF NOT EXISTS idx_provider_models_model_id ON provider_models(model_id);
CREATE INDEX IF NOT EXISTS idx_provider_models_provider ON provider_models(provider_name);
