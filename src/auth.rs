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
