use std::fs;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

fn is_safe_name(name: &str) -> bool {
    let path = Path::new(name);
    // Reject absolute paths, parent dir components, and empty segments
    !path.has_root()
        && path.components().all(|c| {
            matches!(c, std::path::Component::Normal(_))
        })
}

#[derive(Clone)]
pub struct SecretStorage {
    base_path: PathBuf,
}

impl SecretStorage {
    pub fn new(path: &str) -> Self {
        Self {
            base_path: PathBuf::from(path),
        }
    }

    /// Loads a secret by name. Handles subdirectories if present in name.
    pub fn load_secret(&self, name: &str) -> Result<String> {
        if !is_safe_name(name) {
            return Err(anyhow::anyhow!("Path traversal detected in name: {:?}", name));
        }
        let path = self.base_path.join(name);
        let secret = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read secret file: {:?}", path))?;
        Ok(secret.trim_start_matches('\u{feff}').trim().to_string())
    }

    pub fn save_secret(&self, name: &str, secret: &str) -> Result<()> {
        if !is_safe_name(name) {
            return Err(anyhow::anyhow!("Path traversal detected in name: {:?}", name));
        }
        let path = self.base_path.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, secret)
            .with_context(|| format!("Failed to write secret file: {:?}", path))?;
        Ok(())
    }

    pub fn save_secret_for_client(&self, client_id: &str, provider: &str, secret: &str) -> Result<String> {
        if !is_safe_name(client_id) {
            return Err(anyhow::anyhow!("Path traversal detected in client_id: {:?}", client_id));
        }
        if !is_safe_name(provider) {
            return Err(anyhow::anyhow!("Path traversal detected in provider: {:?}", provider));
        }
        let dir = self.base_path.join(client_id);
        fs::create_dir_all(&dir)?;
        
        let filename = format!("{}_api_key.txt", provider.to_lowercase());
        let path = dir.join(&filename);
        
        // Parse keys: comma-separated or newline-separated
        let keys: Vec<&str> = if secret.contains(',') {
            secret.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect()
        } else {
            secret.lines().map(|s| s.trim()).filter(|s| !s.is_empty()).collect()
        };

        if keys.is_empty() {
             return Err(anyhow::anyhow!("Invalid key format. Use 'key1, key2' or 'key1'."));
        }

        // Append to existing file or create new
        let mut content = if path.exists() {
            fs::read_to_string(&path)?
        } else {
            String::new()
        };

        for key in keys {
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(key);
            content.push('\n');
        }

        fs::write(&path, content)?;
        
        // Return relative path for config
        Ok(format!("{}/{}", client_id, filename))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_client_isolated_storage() -> Result<()> {
        let dir = tempdir()?;
        let storage = SecretStorage::new(dir.path().to_str().unwrap());
        
        let client_1 = "client_1";
        let client_2 = "client_2";
        
        // Save for client 1
        let path1 = storage.save_secret_for_client(client_1, "gemini", "key-1")?;
        assert_eq!(path1, "client_1/gemini_api_key.txt");
        
        // Append for client 1
        storage.save_secret_for_client(client_1, "gemini", "key-2, key-3")?;
        
        // Load and verify
        let content1 = storage.load_secret(&path1)?;
        let lines: Vec<&str> = content1.lines().collect();
        assert_eq!(lines, vec!["key-1", "key-2", "key-3"]);
        
        // Save for client 2 (isolation check)
        let path2 = storage.save_secret_for_client(client_2, "gemini", "other-key")?;
        assert_eq!(path2, "client_2/gemini_api_key.txt");
        
        let content2 = storage.load_secret(&path2)?;
        assert_eq!(content2, "other-key");
        
        Ok(())
    }

    #[test]
    fn test_rejects_path_traversal_in_load() {
        let dir = tempdir().unwrap();
        let storage = SecretStorage::new(dir.path().to_str().unwrap());

        // Write a known valid file first
        storage.save_secret("good.txt", "safe").unwrap();

        // Loading a safe file should work
        assert!(storage.load_secret("good.txt").is_ok());

        // Path traversal attempts should fail
        assert!(storage.load_secret("../outside.txt").is_err());
        assert!(storage.load_secret("../../etc/passwd").is_err());
        assert!(storage.load_secret("subdir/../../../etc/passwd").is_err());
        assert!(storage.load_secret("..\\outside.txt").is_err());
    }

    #[test]
    fn test_rejects_path_traversal_in_save() {
        let dir = tempdir().unwrap();
        let storage = SecretStorage::new(dir.path().to_str().unwrap());

        // Saving to a valid path should work
        assert!(storage.save_secret("new_file.txt", "data").is_ok());

        // Path traversal attempts should fail
        assert!(storage.save_secret("../outside.txt", "data").is_err());
        assert!(storage.save_secret("../../etc/passwd", "data").is_err());
    }

    #[test]
    fn test_rejects_path_traversal_in_client_storage() {
        let dir = tempdir().unwrap();
        let storage = SecretStorage::new(dir.path().to_str().unwrap());

        // Normal client_id should work
        assert!(storage.save_secret_for_client("normal_client", "openai", "key-1").is_ok());

        // Traversal attempts should fail
        assert!(storage.save_secret_for_client("../outside_client", "openai", "key-1").is_err());
        assert!(storage.save_secret_for_client("../../etc/passwd", "openai", "key-1").is_err());
    }

    #[test]
    fn test_rejects_path_traversal_in_provider() {
        let dir = tempdir().unwrap();
        let storage = SecretStorage::new(dir.path().to_str().unwrap());

        // Normal provider should work
        assert!(storage.save_secret_for_client("client1", "gemini", "key-1").is_ok());

        // Traversal attempts in provider should fail
        assert!(storage.save_secret_for_client("client1", "../malicious", "key-1").is_err());
        assert!(storage.save_secret_for_client("client1", "../../../etc", "key-1").is_err());
        assert!(storage.save_secret_for_client("client1", "..\\..\\windows", "key-1").is_err());
    }
}
