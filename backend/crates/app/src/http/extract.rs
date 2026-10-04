// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::http::error::HttpError;
use crate::rate_limit::ACCOUNT_MUTATION_BUDGET;
use crate::router::AppState;
use axum::{
    extract::{ConnectInfo, FromRef, FromRequestParts},
    http::{header, request::Parts},
};
use shared::{AccountContext, AppError};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

/// Extractor de IP del cliente seguro contra suplantación (anti-spoofing).
/// Solo confía en X-Real-IP si la conexión física proviene de un proxy de confianza (loopback o red interna).
pub struct ClientIp(pub IpAddr);

impl<S> FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
    Arc<AppState>: FromRef<S>,
{
    type Rejection = HttpError;

    async fn from_request_parts(parts: &mut Parts, state_ref: &S) -> Result<Self, Self::Rejection> {
        let state = Arc::<AppState>::from_ref(state_ref);
        let canonical_peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip().to_canonical());

        let is_trusted_proxy = match canonical_peer {
            Some(ip) => state.config.is_trusted_proxy(&ip),
            None => false,
        };

        if is_trusted_proxy
            && let Some(xri) = parts.headers.get("x-real-ip").and_then(|h| h.to_str().ok())
            && let Ok(parsed_ip) = xri.trim().parse::<IpAddr>()
        {
            return Ok(ClientIp(parsed_ip.to_canonical()));
        }

        if let Some(ip) = canonical_peer {
            return Ok(ClientIp(ip));
        }

        // Fallback seguro para pruebas unitarias sin ConnectInfo
        Ok(ClientIp(IpAddr::V4(Ipv4Addr::LOCALHOST)))
    }
}

/// The credentials of an `Authorization: Bearer <token>` header. The scheme is case-insensitive
/// (RFC 9110 section 11.1) and may be followed by extra spaces; the token itself is untouched.
fn bearer_token(header_value: &str) -> Option<&str> {
    let (scheme, token) = header_value.split_once(' ')?;
    let token = token.trim_start_matches(' ');
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// Extractor de cuenta anónima autenticada por token Bearer HMAC.
/// Aplica límite por cuenta para operaciones de mutación.
pub struct AuthenticatedAccount(pub AccountContext);

impl<S> FromRequestParts<S> for AuthenticatedAccount
where
    S: Send + Sync,
    Arc<AppState>: FromRef<S>,
{
    type Rejection = HttpError;

    async fn from_request_parts(parts: &mut Parts, state_ref: &S) -> Result<Self, Self::Rejection> {
        let state = Arc::<AppState>::from_ref(state_ref);

        let auth_header = parts
            .headers
            .get(header::AUTHORIZATION)
            .ok_or_else(|| {
                HttpError::from(AppError::Unauthorized(
                    "Missing Authorization header".into(),
                ))
            })?
            .to_str()
            .map_err(|_| {
                HttpError::from(AppError::Unauthorized(
                    "Invalid Authorization header encoding".into(),
                ))
            })?;

        let token = bearer_token(auth_header).ok_or_else(|| {
            HttpError::from(AppError::Unauthorized(
                "Authorization scheme must be Bearer".into(),
            ))
        })?;

        let account = state.auth_service.authenticate(token).await?;

        // Límite de mutaciones por cuenta
        state
            .rate_limiter
            .check_with_retry(&ACCOUNT_MUTATION_BUDGET, &account.account_id.to_string())
            .map_err(|retry_after| {
                HttpError::rate_limited(
                    "Account mutation rate limit exceeded. Please retry later.",
                    retry_after,
                )
            })?;

        Ok(AuthenticatedAccount(account))
    }
}

#[cfg(test)]
mod tests {
    use super::bearer_token;

    #[test]
    fn bearer_scheme_is_case_insensitive() {
        for header in [
            "Bearer abc",
            "bearer abc",
            "BEARER abc",
            "BeArEr abc",
            "Bearer   abc",
        ] {
            assert_eq!(bearer_token(header), Some("abc"), "{header}");
        }
    }

    #[test]
    fn other_schemes_and_empty_tokens_are_rejected() {
        for header in [
            "Basic abc",
            "Bearerabc",
            "abc",
            "",
            "Bearer",
            "Bearer ",
            "Token abc",
        ] {
            assert_eq!(bearer_token(header), None, "{header}");
        }
    }

    #[test]
    fn the_token_is_not_altered() {
        assert_eq!(bearer_token("Bearer baze_anon_X.Y"), Some("baze_anon_X.Y"));
    }
}
