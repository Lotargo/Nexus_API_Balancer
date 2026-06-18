use std::sync::Arc;
use std::collections::HashMap;
use std::sync::RwLock;
use arc_swap::ArcSwap;
use serde_json::Value;
use crate::config::AppConfig;
use crate::db::{Database, ProviderModel};
use crate::storage::SecretStorage;

pub struct ModelRegistry {
    db: Database,
    config: Arc<ArcSwap<AppConfig>>,
    http_client: reqwest::Client,
    storage: SecretStorage,
    cache: Arc<RwLock<HashMap<String, Vec<(String, i32)>>>>,
}

impl ModelRegistry {
    pub fn new(
        db: Database,
        config: Arc<ArcSwap<AppConfig>>,
        http_client: reqwest::Client,
        storage: SecretStorage,
    ) -> Self {
        Self {
            db,
            config,
            http_client,
            storage,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn sync_all_providers(&self) -> Result<(), Vec<String>> {
        let config = self.config.load();
        let mut errors = Vec::new();

        for pool_cfg in &config.pools {
            if pool_cfg.skip_model_sync {
                continue;
            }

            if let Err(e) = self.sync_provider(pool_cfg).await {
                errors.push(format!("{}: {}", pool_cfg.name, e));
                eprintln!("[ModelRegistry] Warning: Failed to sync models for '{}': {}", pool_cfg.name, e);
            }
        }

        if let Err(e) = self.db.cleanup_stale_models().await {
            eprintln!("[ModelRegistry] Warning: Cleanup stale models failed: {}", e);
        }

        self.rebuild_cache();

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    async fn sync_provider(&self, pool_cfg: &crate::config::PoolConfig) -> Result<(), String> {
        if let Err(e) = self.db.mark_provider_stale(&pool_cfg.provider).await {
            return Err(format!("Failed to mark stale: {}", e));
        }

        let models = self.fetch_models(pool_cfg).await
            .map_err(|e| format!("Fetch failed: {}", e))?;

        let db_models: Vec<ProviderModel> = models.into_iter().map(|m| {
            let (owned_by, context_window, capabilities) = m.metadata.unwrap_or_default();
            ProviderModel {
                provider_name: pool_cfg.provider.clone(),
                pool_name: pool_cfg.name.clone(),
                model_id: m.id,
                owned_by,
                context_window,
                capabilities,
            }
        }).collect();

        let count = db_models.len();
        match self.db.upsert_provider_models_batch(&pool_cfg.provider, &pool_cfg.name, &db_models).await {
            Ok(n) => println!("[ModelRegistry] Discovered {} models from {}", n, pool_cfg.name),
            Err(e) => return Err(format!("DB upsert failed: {}", e)),
        }

        let _ = count;
        Ok(())
    }

    async fn fetch_models(&self, pool_cfg: &crate::config::PoolConfig) -> Result<Vec<RawModel>, String> {
        let base_url = pool_cfg.target_url.trim_end_matches('/');
        let is_google = pool_cfg.provider == "gemini" || pool_cfg.provider == "google";
        let is_gemini_openai = is_google && base_url.contains("/openai/");

        // Build the models endpoint URL
        let url = if let Some(ref ep) = pool_cfg.models_endpoint {
            format!("{}{}", base_url, ep)
        } else if is_gemini_openai {
            // Google OpenAI-compatible endpoint uses standard OpenAI format
            format!("{}/models", base_url)
        } else if is_google {
            // Gemini requires /v1beta/models (not just /models)
            format!("{}/v1beta/models", base_url)
        } else if base_url.ends_with("/v1") || base_url.ends_with("/v2") {
            // Provider URL already includes version prefix (e.g. https://api.mistral.ai/v1)
            format!("{}/models", base_url)
        } else {
            format!("{}/v1/models", base_url)
        };

        // Extract only the FIRST key from multi-key secret files
        let secret_raw = pool_cfg.keys.first()
            .and_then(|k| self.storage.load_secret(&k.secret_name).ok())
            .unwrap_or_default();
        let secret = secret_raw
            .lines()
            .map(|s| s.trim())
            .find(|s| !s.is_empty())
            .unwrap_or_default();

        if secret.is_empty() {
            return Err("No API key available for model discovery".to_string());
        }

        let mut req = self.http_client.get(&url);

        if is_gemini_openai {
            req = req.header("Authorization", format!("Bearer {}", secret));
        } else if is_google {
            // Gemini uses query param ?key= as primary auth
            let url_with_key = format!("{}?key={}", url, secret);
            req = self.http_client.get(&url_with_key);
        } else {
            req = req.header("Authorization", format!("Bearer {}", secret));
        }

        let resp = req.send().await.map_err(|e| format!("HTTP error: {}", e))?;
        let status = resp.status();
        let body: Value = resp.json().await.map_err(|e| format!("JSON parse error: {}", e))?;

        if !status.is_success() {
            return Err(format!("API returned status {}: {:?}", status, body));
        }

        Ok(parse_models_response(&pool_cfg.provider, &body))
    }

    /// O(1) lookup: returns the best pool name for a model based on priority
    pub fn resolve_model(&self, model_id: &str) -> Option<String> {
        let cache = self.cache.read().unwrap();
        cache.get(model_id)
            .and_then(|pools| pools.first())
            .map(|(pool_name, _)| pool_name.clone())
    }

    /// Resolve model filtering by allowed pools
    pub fn resolve_model_filtered(&self, model_id: &str, allowed_pools: Option<&Vec<String>>) -> Option<String> {
        let cache = self.cache.read().unwrap();
        cache.get(model_id).and_then(|pools| {
            pools.iter()
                .find(|(pool_name, _)| {
                    allowed_pools.map_or(true, |ap| ap.contains(pool_name))
                })
                .map(|(pool_name, _)| pool_name.clone())
        })
    }

    fn rebuild_cache(&self) {
        let config = self.config.load();
        let mut cache: HashMap<String, Vec<(String, i32)>> = HashMap::new();

        let rt = tokio::runtime::Handle::try_current();
        let models = if let Ok(handle) = rt {
            tokio::task::block_in_place(move || {
                handle.block_on(self.db.get_all_models())
            })
        } else {
            return;
        };

        let models = match models {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[ModelRegistry] Failed to load models for cache: {}", e);
                return;
            }
        };

        for model in models {
            let priority = config.pools.iter()
                .find(|p| p.name == model.pool_name)
                .map(|p| p.priority)
                .unwrap_or(0);

            cache.entry(model.model_id)
                .or_default()
                .push((model.pool_name, priority));
        }

        // Sort each entry by priority descending
        for pools in cache.values_mut() {
            pools.sort_by(|a, b| b.1.cmp(&a.1));
        }

        *self.cache.write().unwrap() = cache;
    }

    pub fn spawn_periodic_sync(self: &Arc<Self>) {
        let registry = Arc::clone(self);
        tokio::spawn(async move {
            // Initial sync at startup
            println!("[ModelRegistry] Starting initial model discovery...");
            if let Err(errors) = registry.sync_all_providers().await {
                for e in &errors {
                    eprintln!("[ModelRegistry] Warning: Initial sync had errors: {}", e);
                }
            }

            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(6 * 3600));
            // Skip the first tick (already did initial sync)
            interval.tick().await;
            loop {
                interval.tick().await;
                println!("[ModelRegistry] Starting periodic model rebase...");
                if let Err(errors) = registry.sync_all_providers().await {
                    for e in &errors {
                        eprintln!("[ModelRegistry] Rebase error: {}", e);
                    }
                }
            }
        });
    }
}

struct RawModel {
    id: String,
    metadata: Option<(Option<String>, Option<i64>, Option<String>)>,
}

fn parse_models_response(provider: &str, body: &Value) -> Vec<RawModel> {
    match provider {
        "gemini" | "google" if body.get("data").and_then(|d| d.as_array()).is_some() => {
            // Google OpenAI-compatible endpoint returns OpenAI format ({"data": [...]})
            parse_openai_compatible_models(body)
        }
        "gemini" | "google" => parse_google_models(body),
        _ => parse_openai_compatible_models(body),
    }
}

fn parse_openai_compatible_models(body: &Value) -> Vec<RawModel> {
    let mut models = Vec::new();

    if let Some(data) = body.get("data").and_then(|d| d.as_array()) {
        for item in data {
            let id = match item.get("id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => continue,
            };

            let owned_by = item.get("owned_by").and_then(|v| v.as_str()).map(|s| s.to_string());

            let context_window = item.get("context_window")
                .or_else(|| item.get("max_tokens"))
                .or_else(|| item.get("max_input_tokens"))
                .and_then(|v| v.as_i64());

            let capabilities = item.get("capabilities")
                .map(|v| v.to_string());

            models.push(RawModel {
                id,
                metadata: Some((owned_by, context_window, capabilities)),
            });
        }
    }

    models
}

fn parse_google_models(body: &Value) -> Vec<RawModel> {
    let mut models = Vec::new();

    if let Some(data) = body.get("models").and_then(|d| d.as_array()) {
        for item in data {
            let name = match item.get("name").and_then(|v| v.as_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            // Strip "models/" prefix if present
            let id = name.strip_prefix("models/").unwrap_or(&name).to_string();

            let context_window = item.get("inputTokenLimit")
                .or_else(|| item.get("outputTokenLimit"))
                .and_then(|v| v.as_i64());

            let capabilities = {
                let mut caps = Vec::new();
                if let Some(methods) = item.get("supportedGenerationMethods").and_then(|v| v.as_array()) {
                    for m in methods {
                        if let Some(method) = m.as_str() {
                            caps.push(method.to_string());
                        }
                    }
                }
                if caps.is_empty() {
                    None
                } else {
                    Some(serde_json::json!({"supportedGenerationMethods": caps}).to_string())
                }
            };

            models.push(RawModel {
                id,
                metadata: Some((None, context_window, capabilities)),
            });
        }
    }

    models
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_openai_models_response() {
        let body = json!({
            "data": [
                {"id": "gpt-4", "owned_by": "openai", "context_window": 8192},
                {"id": "gpt-3.5-turbo", "owned_by": "openai", "context_window": 4096}
            ]
        });

        let models = parse_openai_compatible_models(&body);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-4");
        assert_eq!(models[0].metadata.as_ref().unwrap().0.as_deref(), Some("openai"));
        assert_eq!(models[0].metadata.as_ref().unwrap().1, Some(8192));
        assert_eq!(models[1].id, "gpt-3.5-turbo");
    }

    #[test]
    fn parses_openai_models_response_empty() {
        let body = json!({"data": []});
        let models = parse_openai_compatible_models(&body);
        assert!(models.is_empty());
    }

    #[test]
    fn parses_openai_models_response_no_data_field() {
        let body = json!({"error": "not found"});
        let models = parse_openai_compatible_models(&body);
        assert!(models.is_empty());
    }

    #[test]
    fn parses_gemini_models_response() {
        let body = json!({
            "models": [
                {
                    "name": "models/gemini-2.0-flash",
                    "displayName": "Gemini 2.0 Flash",
                    "inputTokenLimit": 1048576,
                    "supportedGenerationMethods": ["generateContent", "streamGenerateContent"]
                },
                {
                    "name": "models/gemini-1.5-pro",
                    "displayName": "Gemini 1.5 Pro",
                    "inputTokenLimit": 2097152
                }
            ]
        });

        let models = parse_google_models(&body);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gemini-2.0-flash");
        assert_eq!(models[0].metadata.as_ref().unwrap().1, Some(1048576));
        assert!(models[0].metadata.as_ref().unwrap().2.is_some());
        assert_eq!(models[1].id, "gemini-1.5-pro");
        assert_eq!(models[1].metadata.as_ref().unwrap().1, Some(2097152));
    }

    #[test]
    fn parses_gemini_models_response_empty() {
        let body = json!({"models": []});
        let models = parse_google_models(&body);
        assert!(models.is_empty());
    }

    #[test]
    fn parses_gemini_model_without_prefix() {
        let body = json!({
            "models": [
                {"name": "gemini-2.0-flash", "displayName": "Gemini 2.0 Flash"}
            ]
        });
        let models = parse_google_models(&body);
        assert_eq!(models[0].id, "gemini-2.0-flash");
    }

    #[test]
    fn parses_models_dispatches_correctly() {
        let openai_body = json!({"data": [{"id": "gpt-4", "owned_by": "openai"}]});
        let gemini_body = json!({"models": [{"name": "models/gemini-2.0-flash"}]});

        let openai_models = parse_models_response("openai", &openai_body);
        assert_eq!(openai_models.len(), 1);
        assert_eq!(openai_models[0].id, "gpt-4");

        let gemini_models = parse_models_response("gemini", &gemini_body);
        assert_eq!(gemini_models.len(), 1);
        assert_eq!(gemini_models[0].id, "gemini-2.0-flash");

        let groq_models = parse_models_response("groq", &openai_body);
        assert_eq!(groq_models.len(), 1);
        assert_eq!(groq_models[0].id, "gpt-4");
    }
}
