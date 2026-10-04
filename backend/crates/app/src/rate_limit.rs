// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::http::error::HttpError;
use crate::http::extract::ClientIp;
use crate::router::AppState;
use axum::{extract::State, middleware::Next, response::Response};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
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

/// Keys are only swept once the table is this big...
const PRUNE_THRESHOLD: usize = 10_000;
/// ...and at most this often, so a flood of new keys cannot make every request pay for an O(n) sweep.
const PRUNE_INTERVAL: Duration = Duration::from_secs(60);
/// A counter untouched for this long is forgotten (longer than every budget window).
const RETENTION: Duration = Duration::from_secs(3600);
/// Hard cap on tracked keys. IPv6 clients are keyed per /64, so an attacker with a large prefix can
/// mint far more distinct keys than any table should hold.
pub const MAX_TRACKED_KEYS: usize = 100_000;
/// Clients that do not fit in the table share this one strict bucket instead of growing it.
const OVERFLOW_KEY: &str = "overflow";

#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<RateLimiterInner>>,
    max_keys: usize,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::with_max_keys(MAX_TRACKED_KEYS)
    }

    pub(crate) fn with_max_keys(max_keys: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                records: HashMap::new(),
                last_pruned: Instant::now(),
            })),
            max_keys,
        }
    }

    /// Verificación centralizada basada en el presupuesto asignado y la clave.
    pub fn check(&self, budget: &Budget, key: &str) -> bool {
        self.check_with_retry(budget, key).is_ok()
    }

    /// Like [`check`](Self::check) but tells a rejected caller how long to wait.
    pub fn check_with_retry(&self, budget: &Budget, key: &str) -> Result<(), Duration> {
        self.check_at(budget, key, Instant::now())
    }

    pub(crate) fn check_at(
        &self,
        budget: &Budget,
        key: &str,
        now: Instant,
    ) -> Result<(), Duration> {
        // A poisoned lock only means another request panicked; the counters are still usable.
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);

        let sweep_due = now.duration_since(inner.last_pruned) >= PRUNE_INTERVAL;
        if inner.records.len() > PRUNE_THRESHOLD && sweep_due {
            Self::prune(&mut inner, now);
        }

        let mut full_key = format!("{}:{}", budget.name, key);
        if !inner.records.contains_key(&full_key) && inner.records.len() >= self.max_keys {
            if sweep_due {
                Self::prune(&mut inner, now);
            }
            if inner.records.len() >= self.max_keys {
                full_key = format!("{}:{}", budget.name, OVERFLOW_KEY);
            }
        }

        let entry = inner.records.entry(full_key).or_insert((now, 0));
        if now.duration_since(entry.0) >= budget.window {
            *entry = (now, 1);
            Ok(())
        } else if entry.1 < budget.limit {
            entry.1 += 1;
            Ok(())
        } else {
            let waited = now.duration_since(entry.0);
            Err(budget
                .window
                .saturating_sub(waited)
                .max(Duration::from_secs(1)))
        }
    }

    fn prune(inner: &mut RateLimiterInner, now: Instant) {
        inner
            .records
            .retain(|_, (last_time, _)| now.duration_since(*last_time) < RETENTION);
        inner.last_pruned = now;
    }

    #[cfg(test)]
    fn tracked_keys(&self) -> usize {
        self.inner.lock().unwrap().records.len()
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
    limiter
        .check_with_retry(budget, &key)
        .map_err(|retry_after| HttpError::rate_limited(error_message, retry_after))
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

    #[test]
    fn rejected_callers_are_told_when_to_retry() {
        let limiter = RateLimiter::new();
        let budget = Budget {
            name: "retry",
            limit: 1,
            window: Duration::from_secs(60),
        };
        let t0 = Instant::now();
        assert!(limiter.check_at(&budget, "k", t0).is_ok());
        let wait = limiter
            .check_at(&budget, "k", t0 + Duration::from_secs(20))
            .unwrap_err();
        assert_eq!(wait, Duration::from_secs(40));
        assert!(
            limiter
                .check_at(&budget, "k", t0 + Duration::from_secs(61))
                .is_ok()
        );
    }

    #[test]
    fn retry_after_is_never_zero() {
        let limiter = RateLimiter::new();
        let budget = Budget {
            name: "retry_min",
            limit: 1,
            window: Duration::from_secs(60),
        };
        let t0 = Instant::now();
        limiter.check_at(&budget, "k", t0).unwrap();
        let wait = limiter
            .check_at(&budget, "k", t0 + Duration::from_millis(59_900))
            .unwrap_err();
        assert!(wait >= Duration::from_secs(1));
    }

    #[test]
    fn memory_stays_bounded_under_a_flood_of_distinct_keys() {
        let limiter = RateLimiter::with_max_keys(100);
        let budget = Budget {
            name: "flood",
            limit: 5,
            window: Duration::from_secs(60),
        };
        let t0 = Instant::now();
        for i in 0..10_000 {
            let _ = limiter.check_at(&budget, &format!("2001:db8:{i:x}::/64"), t0);
        }
        // The cap plus the single shared overflow bucket.
        assert!(
            limiter.tracked_keys() <= 101,
            "tracked {}",
            limiter.tracked_keys()
        );
    }

    #[test]
    fn overflow_clients_share_one_strict_bucket_and_known_clients_are_unaffected() {
        let limiter = RateLimiter::with_max_keys(2);
        let budget = Budget {
            name: "overflow",
            limit: 2,
            window: Duration::from_secs(60),
        };
        let t0 = Instant::now();
        assert!(limiter.check_at(&budget, "a", t0).is_ok());
        assert!(limiter.check_at(&budget, "b", t0).is_ok());

        // The table is full: new clients fall into the shared bucket (limit 2 for all of them together).
        assert!(limiter.check_at(&budget, "c", t0).is_ok());
        assert!(limiter.check_at(&budget, "d", t0).is_ok());
        assert!(limiter.check_at(&budget, "e", t0).is_err());

        // Clients that were already tracked keep their own allowance.
        assert!(limiter.check_at(&budget, "a", t0).is_ok());
        assert!(limiter.check_at(&budget, "a", t0).is_err());
    }

    #[test]
    fn stale_keys_are_swept_to_make_room() {
        let limiter = RateLimiter::with_max_keys(2);
        let budget = Budget {
            name: "sweep",
            limit: 1,
            window: Duration::from_secs(60),
        };
        let t0 = Instant::now();
        limiter.check_at(&budget, "a", t0).unwrap();
        limiter.check_at(&budget, "b", t0).unwrap();

        // More than RETENTION later both are forgotten, so a new client gets its own bucket again.
        let later = t0 + RETENTION + PRUNE_INTERVAL;
        assert!(limiter.check_at(&budget, "c", later).is_ok());
        assert!(limiter.check_at(&budget, "d", later).is_ok());
        assert!(limiter.check_at(&budget, "c", later).is_err());
    }

    #[test]
    fn a_full_table_does_not_trigger_a_sweep_per_request() {
        let limiter = RateLimiter::with_max_keys(50);
        let budget = Budget {
            name: "cost",
            limit: 1000,
            window: Duration::from_secs(60),
        };
        let t0 = Instant::now();
        for i in 0..50 {
            limiter.check_at(&budget, &format!("k{i}"), t0).unwrap();
        }
        let before = limiter.inner.lock().unwrap().last_pruned;
        for i in 0..1_000 {
            let _ = limiter.check_at(&budget, &format!("new{i}"), t0 + Duration::from_secs(1));
        }
        assert_eq!(
            limiter.inner.lock().unwrap().last_pruned,
            before,
            "no sweep within the interval"
        );
    }

    #[test]
    fn a_poisoned_lock_does_not_take_the_limiter_down() {
        let limiter = RateLimiter::new();
        let clone = limiter.clone();
        let _ = std::thread::spawn(move || {
            let _guard = clone.inner.lock().unwrap();
            panic!("poison the mutex");
        })
        .join();
        let budget = Budget {
            name: "poison",
            limit: 1,
            window: Duration::from_secs(60),
        };
        assert!(limiter.check(&budget, "k"));
    }
}
