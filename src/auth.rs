use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Validation, Algorithm, Header};
use serde::{Deserialize, Serialize};
use anyhow::{anyhow, Result};
use crate::config::AuthConfig;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub iss: String,
    pub aud: String,
    pub role: Option<String>,
}

pub struct AuthManager {
    config: AuthConfig,
    decoding_key: DecodingKey,
}

impl AuthManager {
    pub fn new(config: AuthConfig) -> Self {
        let decoding_key = DecodingKey::from_secret(config.secret.as_bytes());
        Self {
            config,
            decoding_key,
        }
    }

    /// Validates a JWT token according to OAuth 2.1 standards.
    pub fn validate_token(&self, token: &str) -> Result<Claims> {
        if !self.config.enabled {
            return Ok(Claims {
                sub: "anonymous".to_string(),
                exp: 0,
                iss: "".to_string(),
                aud: "".to_string(),
                role: None,
            });
        }

        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[&self.config.issuer]);
        validation.set_audience(&[&self.config.audience]);
        validation.validate_exp = true;

        let token_data = decode::<Claims>(
            token,
            &self.decoding_key,
            &validation,
        ).map_err(|e| anyhow!("Invalid token: {}", e))?;

        Ok(token_data.claims)
    }

    pub fn generate_token(&self, sub: &str, role: Option<String>) -> Result<String> {
        let expiration = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::days(365))
            .expect("valid timestamp")
            .timestamp() as usize;

        let claims = Claims {
            sub: sub.to_string(),
            exp: expiration,
            iss: self.config.issuer.clone(),
            aud: self.config.audience.clone(),
            role,
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.config.secret.as_bytes()),
        ).map_err(|e| anyhow!("Token generation failed: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;

    fn make_auth_manager() -> AuthManager {
        let config = AuthConfig {
            enabled: true,
            public_registration: false,
            master_key: None,
            admin_key: None,
            secret: "test-secret-key".to_string(),
            issuer: "test-issuer".to_string(),
            audience: "test-audience".to_string(),
        };
        AuthManager::new(config)
    }

    #[test]
    fn test_generate_and_validate_token() {
        let auth = make_auth_manager();
        let token = auth.generate_token("test-user", Some("client".to_string())).unwrap();
        let claims = auth.validate_token(&token).unwrap();
        assert_eq!(claims.sub, "test-user");
        assert_eq!(claims.role.as_deref(), Some("client"));
        assert_eq!(claims.iss, "test-issuer");
        assert_eq!(claims.aud, "test-audience");
    }

    #[test]
    fn test_validate_token_rejects_wrong_key() {
        let auth = make_auth_manager();
        let token = auth.generate_token("test-user", None).unwrap();

        let bad_auth = AuthManager::new(AuthConfig {
            enabled: true,
            secret: "different-secret".to_string(),
            ..make_auth_manager().config
        });
        assert!(bad_auth.validate_token(&token).is_err());
    }

    #[test]
    fn test_validate_token_rejects_invalid() {
        let auth = make_auth_manager();
        assert!(auth.validate_token("invalid-token").is_err());
    }

    #[test]
    fn test_disabled_auth_returns_anonymous() {
        let auth = AuthManager::new(AuthConfig {
            enabled: false,
            ..make_auth_manager().config
        });
        let claims = auth.validate_token("anything").unwrap();
        assert_eq!(claims.sub, "anonymous");
        assert_eq!(claims.role, None);
    }

    #[test]
    fn test_generate_token_with_admin_role() {
        let auth = make_auth_manager();
        let token = auth.generate_token("admin-user", Some("admin".to_string())).unwrap();
        let claims = auth.validate_token(&token).unwrap();
        assert_eq!(claims.sub, "admin-user");
        assert_eq!(claims.role.as_deref(), Some("admin"));
    }
}
