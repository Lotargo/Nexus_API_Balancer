use serde::{Deserialize, Serialize};
use std::fs;
use anyhow::Result;
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
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
            "cerebras" => Some("https://api.cerebras.ai/v1"),
            "sambanova" => Some("https://api.sambanova.ai/v1"),
            "cohere" => Some("https://api.cohere.com/v2"),
            "mistral" => Some("https://api.mistral.ai/v1"),
            "deepseek" => Some("https://api.deepseek.com"),
            _ => None,
        }
    }

    pub fn get_supported_providers() -> Vec<&'static str> {
        vec!["openai", "gemini", "grok", "cerebras", "sambanova", "cohere", "mistral", "deepseek"]
    }

    pub fn load(path: &str) -> Result<Self> {
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
        let content = serde_yaml::to_string(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}
