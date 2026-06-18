use serde::{Deserialize, Serialize};
use std::fs;
use anyhow::Result;
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_cors_origin")]
    pub cors_allowed_origin: String,
}

fn default_cors_origin() -> String {
    "http://localhost:3317".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct AuthConfig {
    pub enabled: bool,
    pub public_registration: bool,
    pub master_key: Option<String>,
    pub admin_key: Option<String>,
    pub secret: String,
    pub issuer: String,
    pub audience: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct KeyConfig {
    pub id: String,
    pub rps_limit: Option<u32>,
    pub rpd_limit: Option<u32>,
    pub tpm_limit: Option<u32>,
    pub tpd_limit: Option<u32>,
    pub max_request_tokens: Option<u32>,
    pub cooldown_on_limit: Option<bool>,
    pub concurrency: usize,
    pub secret_name: String,
    pub secret_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct PoolConfig {
    pub name: String,
    pub description: Option<String>,
    pub provider: String,
    pub target_url: String,
    pub capacity: usize,
    pub keys: Vec<KeyConfig>,
    /// Priority when same model exists across multiple pools (higher = preferred). Default: 0
    #[serde(default)]
    pub priority: i32,
    /// Custom endpoint for model listing. Default depends on provider.
    pub models_endpoint: Option<String>,
    /// Skip auto-discovery for this pool. Default: false
    #[serde(default)]
    pub skip_model_sync: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub pools: Vec<PoolConfig>,
}

impl AppConfig {
    pub fn get_standard_url(provider: &str) -> Option<&'static str> {
        match provider.to_lowercase().as_str() {
            "openai" => Some("https://api.openai.com/v1"),
            "gemini" | "google" => Some("https://generativelanguage.googleapis.com"),
            "grok" | "xai" => Some("https://api.x.ai/v1"),
            "groq" => Some("https://api.groq.com/openai/v1"),
            "cerebras" => Some("https://api.cerebras.ai/v1"),
            "sambanova" => Some("https://api.sambanova.ai/v1"),
            "cohere" => Some("https://api.cohere.com/v2"),
            "mistral" => Some("https://api.mistral.ai/v1"),
            "deepseek" => Some("https://api.deepseek.com"),
            "anthropic" | "claude" => Some("https://api.anthropic.com/v1"),
            _ => None,
        }
    }

    pub fn get_supported_providers() -> Vec<&'static str> {
        vec!["openai", "gemini", "grok", "groq", "cerebras", "sambanova", "cohere", "mistral", "deepseek", "anthropic"]
    }

    pub fn load(path: &str) -> Result<Self> {
        // Validate path to prevent Path Traversal
        let path_obj = std::path::Path::new(path);
        if path_obj.components().any(|x| x == std::path::Component::ParentDir) {
            return Err(anyhow::anyhow!("Invalid path: Path traversal detected"));
        }

        let content = fs::read_to_string(path)?;
        let mut config: AppConfig = serde_yaml::from_str(&content)?;

        // Override with environment variables if present
        if let Ok(host) = std::env::var("HOST") {
            config.server.host = host;
        }
        if let Ok(port_str) = std::env::var("PORT") {
            if let Ok(port) = port_str.parse::<u16>() {
                config.server.port = port;
            }
        }

        Ok(config)
    }

    pub fn save(&self, path: &str) -> Result<()> {
        // Validate path to prevent Path Traversal
        let path_obj = std::path::Path::new(path);
        if path_obj.components().any(|x| x == std::path::Component::ParentDir) {
            return Err(anyhow::anyhow!("Invalid path: Path traversal detected"));
        }

        let content = serde_yaml::to_string(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_standard_url_known_providers() {
        assert_eq!(AppConfig::get_standard_url("openai"), Some("https://api.openai.com/v1"));
        assert_eq!(AppConfig::get_standard_url("gemini"), Some("https://generativelanguage.googleapis.com"));
        assert_eq!(AppConfig::get_standard_url("google"), Some("https://generativelanguage.googleapis.com"));
        assert_eq!(AppConfig::get_standard_url("anthropic"), Some("https://api.anthropic.com/v1"));
        assert_eq!(AppConfig::get_standard_url("claude"), Some("https://api.anthropic.com/v1"));
        assert_eq!(AppConfig::get_standard_url("grok"), Some("https://api.x.ai/v1"));
        assert_eq!(AppConfig::get_standard_url("xai"), Some("https://api.x.ai/v1"));
        assert_eq!(AppConfig::get_standard_url("mistral"), Some("https://api.mistral.ai/v1"));
        assert_eq!(AppConfig::get_standard_url("deepseek"), Some("https://api.deepseek.com"));
    }

    #[test]
    fn test_get_standard_url_unknown_provider() {
        assert_eq!(AppConfig::get_standard_url("nonexistent"), None);
        assert_eq!(AppConfig::get_standard_url(""), None);
    }

    #[test]
    fn test_get_standard_url_case_insensitive() {
        assert_eq!(AppConfig::get_standard_url("OpenAI"), Some("https://api.openai.com/v1"));
        assert_eq!(AppConfig::get_standard_url("ANTHROPIC"), Some("https://api.anthropic.com/v1"));
    }

    #[test]
    fn test_get_supported_providers_includes_anthropic() {
        let providers = AppConfig::get_supported_providers();
        assert!(providers.contains(&"anthropic"));
        assert!(providers.contains(&"openai"));
        assert!(providers.contains(&"gemini"));
        assert!(providers.contains(&"mistral"));
    }

    #[test]
    fn test_load_rejects_path_traversal() {
        assert!(AppConfig::load("../outside.yaml").is_err());
        assert!(AppConfig::load("../../etc/passwd").is_err());
    }

    #[test]
    fn test_save_rejects_path_traversal() {
        let config = AppConfig {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 3317,
                cors_allowed_origin: "http://localhost:3317".to_string(),
            },
            auth: AuthConfig {
                enabled: false,
                public_registration: false,
                master_key: None,
                admin_key: None,
                secret: "test".to_string(),
                issuer: "test".to_string(),
                audience: "test".to_string(),
            },
            pools: vec![],
        };
        assert!(config.save("../outside.yaml").is_err());
        assert!(config.save("../../etc/passwd").is_err());
    }

    #[test]
    fn test_default_cors_origin() {
        assert_eq!(default_cors_origin(), "http://localhost:3317");
    }
}
