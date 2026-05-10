use std::fs;
use std::path::PathBuf;
use anyhow::{Context, Result};

pub struct SecretStorage {
    base_path: PathBuf,
}

impl SecretStorage {
    pub fn new(path: &str) -> Self {
        Self {
            base_path: PathBuf::from(path),
        }
    }

    /// Loads a secret by name.
    /// In a real production app, this would involve decryption or calling a Vault API.
    pub fn load_secret(&self, name: &str) -> Result<String> {
        let path = self.base_path.join(name);
        let secret = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read secret file: {:?}", path))?;
        Ok(secret.trim().to_string())
    }
}
