// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use ipnet::IpNet;
use std::env;
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Development,
    Production,
}

impl Environment {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
        }
    }
}

impl std::fmt::Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub environment: Environment,
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub valhalla_url: String,
    pub photon_url: String,
    pub jwt_secret: String,
    pub rate_limit_rpm: u64,
    pub confirmation_threshold: i32,
    pub default_ttl_hours: i64,
    pub source_repo_url: String,
    pub git_commit_hash: String,
    pub trusted_proxies: Vec<IpNet>,
}

fn lookup_trimmed<F: Fn(&str) -> Option<String>>(lookup: &F, key: &str) -> Option<String> {
    lookup(key).and_then(|val| {
        let trimmed = val.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn parse_lookup_var<T: std::str::FromStr, F: Fn(&str) -> Option<String>>(
    lookup: &F,
    key: &str,
    default: T,
) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    match lookup_trimmed(lookup, key) {
        Some(val) => val
            .parse::<T>()
            .map_err(|e| format!("Invalid value for environment variable '{}': {}", key, e)),
        None => Ok(default),
    }
}

impl AppConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|k| env::var(k).ok())
    }

    pub fn from_lookup<F>(lookup: F) -> Result<Self, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let env_str = lookup_trimmed(&lookup, "ENVIRONMENT").unwrap_or_else(|| "production".to_string());
        let environment = if env_str.eq_ignore_ascii_case("development") {
            Environment::Development
        } else {
            Environment::Production
        };

        let is_dev = environment == Environment::Development;
        let is_prod = !is_dev;

        let host = lookup_trimmed(&lookup, "HOST").unwrap_or_else(|| "0.0.0.0".to_string());
        let port = parse_lookup_var(&lookup, "PORT", 8080u16)?;
        if port == 0 {
            return Err("PORT must be greater than 0".into());
        }

        // Validación estricta en producción para evitar 'fail-open' con secretos por defecto
        let database_url = match lookup_trimmed(&lookup, "DATABASE_URL") {
            Some(url) => {
                if is_prod && url.contains("baze_secure_password_replace_in_production") {
                    return Err(
                        "DATABASE_URL in production environment cannot use default placeholder password"
                            .into(),
                    );
                }
                url
            }
            None => {
                if is_prod {
                    return Err("DATABASE_URL must be explicitly provided in production".into());
                }
                "postgres://baze_user:baze_secure_password_replace_in_production@localhost:5432/baze_db".to_string()
            }
        };

        let jwt_secret = match lookup_trimmed(&lookup, "JWT_SECRET") {
            Some(secret) => {
                if is_prod && (secret.contains("dev_insecure") || secret.len() < 32) {
                    return Err(
                        "JWT_SECRET in production must be a secure random secret of at least 32 bytes"
                            .into(),
                    );
                }
                secret
            }
            None => {
                if is_prod {
                    return Err("JWT_SECRET must be explicitly provided in production".into());
                }
                "dev_insecure_jwt_secret_must_be_32_bytes_long_min".to_string()
            }
        };

        let valhalla_url =
            lookup_trimmed(&lookup, "VALHALLA_URL").unwrap_or_else(|| "http://localhost:8002".to_string());
        let photon_url =
            lookup_trimmed(&lookup, "PHOTON_URL").unwrap_or_else(|| "http://localhost:2322".to_string());

        let rate_limit_rpm = parse_lookup_var(&lookup, "RATE_LIMIT_REQUESTS_PER_MINUTE", 60u64)?;
        if rate_limit_rpm == 0 {
            return Err("RATE_LIMIT_REQUESTS_PER_MINUTE must be at least 1".into());
        }

        let confirmation_threshold = parse_lookup_var(&lookup, "HAZARD_CONFIRMATION_THRESHOLD", 3i32)?;
        if confirmation_threshold < 1 {
            return Err("HAZARD_CONFIRMATION_THRESHOLD must be at least 1".into());
        }

        let default_ttl_hours = parse_lookup_var(&lookup, "HAZARD_DEFAULT_TTL_HOURS", 24i64)?;
        if !(1..=8760).contains(&default_ttl_hours) {
            return Err("HAZARD_DEFAULT_TTL_HOURS must be between 1 and 8760 hours (1 year)".into());
        }

        let source_repo_url = lookup_trimmed(&lookup, "SOURCE_REPO_URL")
            .unwrap_or_else(|| "https://github.com/kripiman/Baze".to_string());
        let git_commit_hash =
            lookup_trimmed(&lookup, "GIT_COMMIT_HASH").unwrap_or_else(|| "dev".to_string());
        let trusted_proxies_str = lookup_trimmed(&lookup, "TRUSTED_PROXIES")
            .unwrap_or_else(|| "127.0.0.1,::1,172.16.0.0/12".to_string());
        let mut trusted_proxies = Vec::new();
        for s in trusted_proxies_str.split(',') {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                continue;
            }
            let net = trimmed
                .parse::<IpNet>()
                .or_else(|_| trimmed.parse::<IpAddr>().map(IpNet::from))
                .map_err(|_| format!("Invalid IP or CIDR in TRUSTED_PROXIES: '{trimmed}'"))?;
            trusted_proxies.push(net);
        }

        Ok(Self {
            environment,
            host,
            port,
            database_url,
            valhalla_url,
            photon_url,
            jwt_secret,
            rate_limit_rpm,
            confirmation_threshold,
            default_ttl_hours,
            source_repo_url,
            git_commit_hash,
            trusted_proxies,
        })
    }

    pub fn is_development(&self) -> bool {
        self.environment == Environment::Development
    }

    pub fn is_trusted_proxy(&self, ip: &IpAddr) -> bool {
        self.trusted_proxies.iter().any(|net| net.contains(ip))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string())
    }

    #[test]
    fn test_unset_environment_defaults_to_production_and_fails_closed() {
        let config = AppConfig::from_lookup(mock_env(&[]));
        assert!(config.is_err());
        let err = config.err().unwrap();
        assert!(err.contains("must be explicitly provided in production"));
    }

    #[test]
    fn test_explicit_development_permits_defaults() {
        let config = AppConfig::from_lookup(mock_env(&[("ENVIRONMENT", "development")])).unwrap();
        assert!(config.is_development());
        assert_eq!(config.environment, Environment::Development);
        assert!(config.jwt_secret.contains("dev_insecure"));
    }

    #[test]
    fn test_production_rejects_placeholder_secrets() {
        let config = AppConfig::from_lookup(mock_env(&[
            ("ENVIRONMENT", "production"),
            (
                "DATABASE_URL",
                "postgres://user:baze_secure_password_replace_in_production@db:5432/db",
            ),
            ("JWT_SECRET", "super_secret_key_at_least_32_bytes_long_here"),
        ]));
        assert!(config.is_err());
        assert!(config.err().unwrap().contains("default placeholder password"));

        let config2 = AppConfig::from_lookup(mock_env(&[
            ("ENVIRONMENT", "production"),
            ("DATABASE_URL", "postgres://user:real_prod_pw@db:5432/db"),
            ("JWT_SECRET", "short_secret"),
        ]));
        assert!(config2.is_err());
        assert!(config2.err().unwrap().contains("at least 32 bytes"));
    }

    #[test]
    fn test_production_valid_configuration() {
        let config = AppConfig::from_lookup(mock_env(&[
            ("ENVIRONMENT", "production"),
            ("DATABASE_URL", "postgres://user:real_prod_password@db:5432/db"),
            ("JWT_SECRET", "super_secure_random_production_key_32_bytes"),
        ]))
        .unwrap();
        assert_eq!(config.environment, Environment::Production);
        assert!(!config.is_development());
    }

    #[test]
    fn test_invalid_number_fails_fast_without_silent_fallback() {
        let config = AppConfig::from_lookup(mock_env(&[
            ("ENVIRONMENT", "development"),
            ("RATE_LIMIT_REQUESTS_PER_MINUTE", "not_a_number"),
        ]));
        assert!(config.is_err());
        assert!(config.err().unwrap().contains("RATE_LIMIT_REQUESTS_PER_MINUTE"));
    }

    #[test]
    fn test_out_of_range_config_rejected() {
        let config = AppConfig::from_lookup(mock_env(&[
            ("ENVIRONMENT", "development"),
            ("RATE_LIMIT_REQUESTS_PER_MINUTE", "0"),
        ]));
        assert!(config.is_err());

        let config2 = AppConfig::from_lookup(mock_env(&[
            ("ENVIRONMENT", "development"),
            ("RATE_LIMIT_REQUESTS_PER_MINUTE", "60"),
            ("HAZARD_DEFAULT_TTL_HOURS", "90000"), // > 8760
        ]));
        assert!(config2.is_err());
    }

    #[test]
    fn test_trusted_proxies_cidr_matching() {
        let config = AppConfig::from_lookup(mock_env(&[
            ("ENVIRONMENT", "development"),
            (
                "TRUSTED_PROXIES",
                "127.0.0.1, ::1, 172.20.0.0/16, 10.0.1.0/24, fd00::/8, 192.168.1.50",
            ),
        ]))
        .unwrap();

        // Loopback confiable si está en la lista explícita
        assert!(config.is_trusted_proxy(&"127.0.0.1".parse().unwrap()));
        assert!(config.is_trusted_proxy(&"::1".parse().unwrap()));

        // Rangos CIDR IPv4 arbitrarios
        assert!(config.is_trusted_proxy(&"172.20.5.1".parse().unwrap()));
        assert!(!config.is_trusted_proxy(&"172.21.5.1".parse().unwrap()));

        assert!(config.is_trusted_proxy(&"10.0.1.42".parse().unwrap()));
        assert!(!config.is_trusted_proxy(&"10.0.2.1".parse().unwrap()));

        // Rango CIDR IPv6
        assert!(config.is_trusted_proxy(&"fd00::abcd".parse().unwrap()));
        assert!(!config.is_trusted_proxy(&"fe80::1".parse().unwrap()));

        // IP fija sin prefijo
        assert!(config.is_trusted_proxy(&"192.168.1.50".parse().unwrap()));
        assert!(!config.is_trusted_proxy(&"192.168.1.51".parse().unwrap()));
    }

    #[test]
    fn test_trusted_proxies_default_and_custom_exclusion() {
        // Por defecto incluye loopback
        let default_config =
            AppConfig::from_lookup(mock_env(&[("ENVIRONMENT", "development")])).unwrap();
        assert!(default_config.is_trusted_proxy(&"127.0.0.1".parse().unwrap()));
        assert!(default_config.is_trusted_proxy(&"::1".parse().unwrap()));

        // Si se sobreescribe sin loopback, loopback NO es confiable (no hay bypass hardcodeado)
        let custom_config = AppConfig::from_lookup(mock_env(&[
            ("ENVIRONMENT", "development"),
            ("TRUSTED_PROXIES", "10.0.0.1"),
        ]))
        .unwrap();
        assert!(!custom_config.is_trusted_proxy(&"127.0.0.1".parse().unwrap()));
        assert!(!custom_config.is_trusted_proxy(&"::1".parse().unwrap()));
        assert!(custom_config.is_trusted_proxy(&"10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn test_trusted_proxies_invalid_cidr_fails_fast() {
        let config = AppConfig::from_lookup(mock_env(&[
            ("ENVIRONMENT", "development"),
            ("TRUSTED_PROXIES", "172.20.0.0/16, not_a_valid_cidr/99"),
        ]));
        assert!(config.is_err());
        assert!(config
            .err()
            .unwrap()
            .contains("Invalid IP or CIDR in TRUSTED_PROXIES"));
    }
}
