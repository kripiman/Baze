// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Anonymous accounts. An account is a row in `accounts` plus a signed, expiring bearer token (see
//! [`token`]); the row is what makes accounts real: it records when the account was created and lets
//! an operator switch an abusive one off (`is_active = false`) without waiting for its token to expire.

pub mod token;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use shared::{AccountContext, AccountService, AppError, AuthResponse};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use token::{TokenError, TokenKeys};
use uuid::Uuid;

#[derive(Clone)]
pub struct AuthService {
    pool: PgPool,
    keys: Arc<TokenKeys>,
    token_ttl: Duration,
}

impl AuthService {
    pub fn new(pool: PgPool, keys: TokenKeys, token_ttl: Duration) -> Self {
        Self {
            pool,
            keys: Arc::new(keys),
            token_ttl,
        }
    }
}

/// Every reason a token can be refused looks the same to the caller, apart from plain expiry (which is
/// only reported once the signature has checked out), so errors cannot be used to probe tokens.
fn unauthorized(error: TokenError) -> AppError {
    match error {
        TokenError::Expired => AppError::Unauthorized("Token expired".into()),
        _ => invalid_token(),
    }
}

fn invalid_token() -> AppError {
    AppError::Unauthorized("Invalid authorization token".into())
}

fn store_error(error: sqlx::Error) -> AppError {
    AppError::Internal(format!("account store: {error}"))
}

#[async_trait]
impl AccountService for AuthService {
    async fn create_anonymous_account(&self) -> Result<AuthResponse, AppError> {
        let account_id = Uuid::new_v4();
        let created_at: DateTime<Utc> =
            sqlx::query_scalar("INSERT INTO accounts (id) VALUES ($1) RETURNING created_at")
                .bind(account_id)
                .fetch_one(&self.pool)
                .await
                .map_err(store_error)?;

        let (token, claims) = token::issue(&self.keys, account_id, Utc::now(), self.token_ttl);
        tracing::debug!(%account_id, "Created an anonymous account");

        Ok(AuthResponse {
            account_id,
            token,
            created_at,
            expires_at: claims.expires_at,
        })
    }

    async fn authenticate(&self, token: &str) -> Result<AccountContext, AppError> {
        let claims = token::verify(&self.keys, token, Utc::now()).map_err(unauthorized)?;

        // A valid signature proves the token was issued, not that the account is still welcome.
        let account = sqlx::query("SELECT created_at, is_active FROM accounts WHERE id = $1")
            .bind(claims.account_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;

        match account {
            Some(row) if row.get::<bool, _>("is_active") => Ok(AccountContext {
                account_id: claims.account_id,
                created_at: row.get("created_at"),
            }),
            // Unknown and deactivated accounts are indistinguishable from a bad token.
            _ => Err(invalid_token()),
        }
    }
}
