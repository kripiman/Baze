// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use shared::AppError;
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuthResponse {
    pub account_id: Uuid,
    pub token: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct AuthService {
    #[allow(dead_code)]
    pool: PgPool,
    jwt_secret: String,
}

impl AuthService {
    pub fn new(pool: PgPool, jwt_secret: String) -> Self {
        Self { pool, jwt_secret }
    }

    fn compute_signature(&self, account_id: &Uuid) -> String {
        let mut mac = HmacSha256::new_from_slice(self.jwt_secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(account_id.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    pub async fn create_anonymous_account(&self) -> Result<AuthResponse, AppError> {
        // TODO(verify): Implementar inserción SQL en la tabla `accounts`
        let account_id = Uuid::new_v4();
        let now = Utc::now();
        let sig = self.compute_signature(&account_id);
        let token = format!("baze_anon_{}.{}", account_id, sig);

        tracing::info!(account_id = %account_id, "Emitted new signed anonymous account");

        Ok(AuthResponse {
            account_id,
            token,
            created_at: now,
        })
    }

    pub async fn validate_token(&self, token: &str) -> Result<Uuid, AppError> {
        let stripped = token
            .strip_prefix("baze_anon_")
            .ok_or_else(|| AppError::Unauthorized("Invalid authorization token format".into()))?;

        let mut parts = stripped.split('.');
        let id_str = parts
            .next()
            .ok_or_else(|| AppError::Unauthorized("Missing account ID in token".into()))?;
        let sig_str = parts.next().ok_or_else(|| {
            AppError::Unauthorized("Missing cryptographic signature in token".into())
        })?;

        if parts.next().is_some() {
            return Err(AppError::Unauthorized("Malformed token structure".into()));
        }

        let account_id = Uuid::parse_str(id_str)
            .map_err(|_| AppError::Unauthorized("Invalid account ID format in token".into()))?;

        let sig_bytes = hex::decode(sig_str)
            .map_err(|_| AppError::Unauthorized("Invalid hex signature format in token".into()))?;

        // Verificación criptográfica en tiempo constante mediante Mac::verify_slice
        let mut mac = HmacSha256::new_from_slice(self.jwt_secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(account_id.as_bytes());

        if mac.verify_slice(&sig_bytes).is_err() {
            tracing::warn!(account_id = %account_id, "Token signature verification failed");
            return Err(AppError::Unauthorized("Invalid token signature".into()));
        }

        Ok(account_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    #[tokio::test]
    async fn test_token_creation_and_validation() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://localhost/dummy")
            .unwrap();
        let secret = "very_secure_test_secret_key_at_least_32_bytes_long".to_string();
        let service = AuthService::new(pool, secret);

        let auth_res = service.create_anonymous_account().await.unwrap();
        assert!(auth_res.token.starts_with("baze_anon_"));

        // Valid token passes
        let validated_id = service.validate_token(&auth_res.token).await.unwrap();
        assert_eq!(validated_id, auth_res.account_id);

        // Forged signature fails
        let forged_token = format!(
            "baze_anon_{}.00112233445566778899aabbccddeeff",
            auth_res.account_id
        );
        assert!(service.validate_token(&forged_token).await.is_err());

        // Tampered account id fails
        let other_id = Uuid::new_v4();
        let parts: Vec<&str> = auth_res.token.split('.').collect();
        let tampered_token = format!("baze_anon_{}.{}", other_id, parts[1]);
        assert!(service.validate_token(&tampered_token).await.is_err());

        // Invalid hex signature fails
        let invalid_hex_token =
            format!("baze_anon_{}.not_valid_hex_signature!", auth_res.account_id);
        assert!(service.validate_token(&invalid_hex_token).await.is_err());
    }
}
