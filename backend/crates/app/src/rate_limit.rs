// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone)]
pub struct RateLimiter {
    rpm: u64,
    records: Arc<Mutex<HashMap<String, (Instant, u64)>>>,
}

impl RateLimiter {
    pub fn new(rpm: u64) -> Self {
        Self {
            rpm,
            records: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn check(&self, key: &str) -> Result<(), ()> {
        let mut map = self.records.lock().unwrap();
        let now = Instant::now();

        // Evitar crecimiento ilimitado de memoria limpiando registros antiguos si la tabla crece
        if map.len() > 10_000 {
            map.retain(|_, (last_time, _)| now.duration_since(*last_time).as_secs() < 60);
        }

        let entry = map.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0).as_secs() >= 60 {
            *entry = (now, 1);
            Ok(())
        } else if entry.1 < self.rpm {
            entry.1 += 1;
            Ok(())
        } else {
            Err(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_allows_under_limit_and_blocks_over() {
        let limiter = RateLimiter::new(2); // 2 requests per minute
        let key = "client_test_1";

        assert!(limiter.check(key).is_ok());
        assert!(limiter.check(key).is_ok());
        assert!(limiter.check(key).is_err()); // Exceeded limit!

        // Different key is unaffected
        let key2 = "client_test_2";
        assert!(limiter.check(key2).is_ok());
    }
}
