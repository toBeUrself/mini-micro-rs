use std::sync::Arc;

use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct JwtManager {
    secret: Arc<str>, // JWT 签名密钥
    ttl: Duration,    // token 有效期，比如 7 天
}

#[derive(Debug, Clone, Serialize)]
pub struct IssuedToken {
    pub token: String,
    pub expires_in: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Business user id. We keep user state out of the token and reload it from
    /// storage before proxying so phone/openid binding changes take effect.
    pub sub: String, // subject，这里放业务用户 id。
    pub iat: usize, // issued at，签发时间。
    pub exp: usize, // expires at，过期时间。
}

#[derive(Debug, Error)]
pub enum JwtError {
    #[error("failed to encode JWT: {0}")]
    Encode(#[source] jsonwebtoken::errors::Error),
    #[error("invalid JWT: {0}")]
    Decode(#[source] jsonwebtoken::errors::Error),
    #[error("JWT subject is not a valid UUID: {0}")]
    InvalidSubject(#[source] uuid::Error),
}

impl JwtManager {
    pub fn new(secret: impl Into<String>, ttl_seconds: i64) -> Self {
        Self {
            secret: Arc::from(secret.into()),
            ttl: Duration::seconds(ttl_seconds),
        }
    }

    // issue：签发 token
    pub fn issue(&self, user_id: Uuid) -> Result<IssuedToken, JwtError> {
        let now = Utc::now();
        let expires_at = now + self.ttl;
        let claims = Claims {
            sub: user_id.to_string(),
            iat: now.timestamp() as usize,
            exp: expires_at.timestamp() as usize,
        };

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(self.secret.as_bytes()),
        )
        .map_err(JwtError::Encode)?;

        Ok(IssuedToken {
            token,
            expires_in: self.ttl.num_seconds(),
        })
    }

    // 验证 token
    pub fn verify(&self, token: &str) -> Result<Uuid, JwtError> {
        let mut validation = Validation::new(Algorithm::HS256);
        // Keep expiry strict; clients should refresh via WeChat login instead
        // of relying on server-side leeway.
        validation.leeway = 0;
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &validation,
        )
        .map_err(JwtError::Decode)?;

        Uuid::parse_str(&token_data.claims.sub).map_err(JwtError::InvalidSubject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issues_and_verifies_token() {
        let manager = JwtManager::new("secret", 60);
        let user_id = Uuid::new_v4();

        let issued = manager.issue(user_id).expect("token should be issued");
        let verified = manager.verify(&issued.token).expect("token should verify");

        assert_eq!(verified, user_id);
        assert_eq!(issued.expires_in, 60);
    }

    #[test]
    fn rejects_wrong_signature() {
        let user_id = Uuid::new_v4();
        let issued = JwtManager::new("secret-a", 60)
            .issue(user_id)
            .expect("token should be issued");

        let error = JwtManager::new("secret-b", 60)
            .verify(&issued.token)
            .expect_err("wrong signature should fail");

        assert!(matches!(error, JwtError::Decode(_)));
    }

    #[test]
    fn rejects_expired_token() {
        let user_id = Uuid::new_v4();
        let issued = JwtManager::new("secret", -120)
            .issue(user_id)
            .expect("token should be issued");

        let error = JwtManager::new("secret", 60)
            .verify(&issued.token)
            .expect_err("expired token should fail");

        assert!(matches!(error, JwtError::Decode(_)));
    }
}
