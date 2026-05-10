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
    pub secret: String,
    pub issuer: String,
    pub audience: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct KeyConfig {
    pub id: String,
    pub limit: u32,
    pub concurrency: usize,
    pub secret_name: String,
    pub secret_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct PoolConfig {
    pub name: String,
    pub description: Option<String>,
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
    pub fn load(path: &str) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: AppConfig = serde_yaml::from_str(&content)?;
        Ok(config)
    }
}
