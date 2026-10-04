// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::http::error::HttpError;
use crate::http::extract::ClientIp;
use crate::router::AppState;
use axum::{extract::State, middleware::Next, response::Response};
use shared::AppError;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub name: &'static str,
    pub limit: u64,
    pub window: Duration,
}

pub const SIGNUP_BUDGET: Budget = Budget {
    name: "auth_signup",
    limit: 60,
    window: Duration::from_secs(900), // 60 cuentas cada 15 min por IP (acorde con ADR-0006 para mitigar falsos positivos en CGNAT)
};

pub const ACCOUNT_MUTATION_BUDGET: Budget = Budget {
    name: "account_mutation",
    limit: 30,
    window: Duration::from_secs(60), // 30 mutaciones por min por cuenta
};

pub fn per_ip_budget(rpm: u64) -> Budget {
    Budget {
        name: "per_ip",
        limit: rpm,
        window: Duration::from_secs(60),
    }
}

struct RateLimiterInner {
    records: HashMap<String, (Instant, u64)>,
    last_pruned: Instant,
}

#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<RateLimiterInner>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                records: HashMap::new(),
                last_pruned: Instant::now(),
            })),
        }
    }

    /// Verificación centralizada basada en el presupuesto asignado y la clave.
    pub fn check(&self, budget: &Budget, key: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let now = Instant::now();

        // Limpieza amortizada O(1): solo cada 60s si la tabla supera 10k entradas
        if inner.records.len() > 10_000
            && now.duration_since(inner.last_pruned) >= Duration::from_secs(60)
        {
            inner.records.retain(|_, (last_time, _)| {
                now.duration_since(*last_time) < Duration::from_secs(3600)
            });
            inner.last_pruned = now;
        }

        let full_key = format!("{}:{}", budget.name, key);
        let entry = inner.records.entry(full_key).or_insert((now, 0));
        if now.duration_since(entry.0) >= budget.window {
            *entry = (now, 1);
            true
        } else if entry.1 < budget.limit {
            entry.1 += 1;
            true
        } else {
            false
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

fn enforce_client_rate_limit(
    limiter: &RateLimiter,
    budget: &Budget,
    client_ip: IpAddr,
    error_message: &'static str,
) -> Result<(), HttpError> {
    let key = shared::client_network(client_ip).to_string();
    if !limiter.check(budget, &key) {
        return Err(HttpError::from(AppError::RateLimited(error_message.into())));
    }
    Ok(())
}

/// Middleware Tower que aplica presupuesto estricto de registro anónimo sobre la IP del cliente (política cliente).
pub async fn rate_limit_signup_middleware(
    State(state): State<Arc<AppState>>,
    client_ip: ClientIp,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response, HttpError> {
    enforce_client_rate_limit(
        &state.rate_limiter,
        &SIGNUP_BUDGET,
        client_ip.0,
        "Auth registration rate limit exceeded. Please retry later.",
    )?;
    Ok(next.run(request).await)
}

/// Middleware Tower que aplica presupuesto general de API sobre la IP del cliente (política cliente).
pub async fn rate_limit_api_middleware(
    State(state): State<Arc<AppState>>,
    client_ip: ClientIp,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response, HttpError> {
    let budget = per_ip_budget(state.config.rate_limit_rpm);
    enforce_client_rate_limit(
        &state.rate_limiter,
        &budget,
        client_ip.0,
        "Rate limit exceeded. Please retry later.",
    )?;
    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_allows_under_limit_and_blocks_over() {
        let limiter = RateLimiter::new();
        let budget = Budget {
            name: "test_limiter",
            limit: 2,
            window: Duration::from_secs(60),
        };
        let key = "client_test_1";

        assert!(limiter.check(&budget, key));
        assert!(limiter.check(&budget, key));
        assert!(!limiter.check(&budget, key)); // Superó límite

        // Diferente clave no es afectada
        let key2 = "client_test_2";
        assert!(limiter.check(&budget, key2));
    }

    #[test]
    fn test_rate_limiter_auth_strict_budget() {
        let limiter = RateLimiter::new();
        let ip_key = "192.168.1.10";

        for _ in 0..SIGNUP_BUDGET.limit {
            assert!(limiter.check(&SIGNUP_BUDGET, ip_key));
        }
        // La siguiente llamada dentro de la ventana falla
        assert!(!limiter.check(&SIGNUP_BUDGET, ip_key));
    }

    #[test]
    fn test_client_network_prefixes() {
        let v4: IpAddr = "192.168.1.100".parse().unwrap();
        assert_eq!(shared::client_network(v4).to_string(), "192.168.1.100/32");

        let v6: IpAddr = "2001:db8:85a3:0:1234:8a2e:370:7334".parse().unwrap();
        assert_eq!(shared::client_network(v6).to_string(), "2001:db8:85a3::/64");
    }
}
