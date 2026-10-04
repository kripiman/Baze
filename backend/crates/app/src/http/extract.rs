// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::http::error::HttpError;
use crate::rate_limit::ACCOUNT_MUTATION_BUDGET;
use crate::router::AppState;
use axum::{
    extract::{ConnectInfo, FromRef, FromRequestParts},
    http::{header, request::Parts},
};
use shared::AppError;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use uuid::Uuid;

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

/// Extractor de cuenta anónima autenticada por token Bearer HMAC.
/// Aplica límite por cuenta para operaciones de mutación.
pub struct AuthenticatedAccount(pub Uuid);

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

        let token = auth_header.strip_prefix("Bearer ").ok_or_else(|| {
            HttpError::from(AppError::Unauthorized(
                "Authorization scheme must be Bearer".into(),
            ))
        })?;

        let account_id = state
            .auth_service
            .validate_token(token)
            .await
            .map_err(HttpError::from)?;

        // Límite de mutaciones por cuenta
        if !state
            .rate_limiter
            .check(&ACCOUNT_MUTATION_BUDGET, &account_id.to_string())
        {
            return Err(HttpError::from(AppError::RateLimited(
                "Account mutation rate limit exceeded. Please retry later.".into(),
            )));
        }

        Ok(AuthenticatedAccount(account_id))
    }
}
