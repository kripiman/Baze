// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use ipnet::IpNet;
use sqlx::postgres::PgConnectOptions;
use std::collections::{HashMap, HashSet};
use std::env;
use std::net::IpAddr;
use std::str::FromStr;

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
    /// Whether Swagger UI and `/api-docs/openapi.json` are served.
    pub enable_api_docs: bool,
    pub request_timeout_secs: u64,
    pub max_concurrent_requests: usize,
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

/// Fragments that betray a value copied from documentation or a template.
const WEAK_SECRET_MARKERS: &[&str] = &[
    "change_me",
    "changeme",
    "replace",
    "example",
    "placeholder",
    "secret",
    "password",
    "dev_insecure",
    "default",
];
const MIN_SECRET_BYTES: usize = 32;
const MIN_DISTINCT_CHARS: usize = 10;
const MIN_ENTROPY_BITS_PER_CHAR: f64 = 3.0;

fn shannon_entropy_bits_per_char(value: &str) -> f64 {
    let mut counts: HashMap<char, usize> = HashMap::new();
    let mut total = 0usize;
    for c in value.chars() {
        *counts.entry(c).or_insert(0) += 1;
        total += 1;
    }
    if total == 0 {
        return 0.0;
    }
    counts
        .values()
        .map(|&n| {
            let p = n as f64 / total as f64;
            -p * p.log2()
        })
        .sum()
}

/// True when the value is one short pattern repeated (`abab…`, `0123456789` ×6).
fn is_repeating_pattern(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=bytes.len() / 2).any(|period| {
        let chunks = bytes.chunks_exact(period);
        chunks.remainder().is_empty() && chunks.clone().all(|chunk| chunk == &bytes[..period])
    })
}

/// Why a signing secret must not be used in production, or `None` if it looks like random key material.
fn jwt_secret_weakness(secret: &str) -> Option<String> {
    if secret.len() < MIN_SECRET_BYTES {
        return Some(format!("it is shorter than {MIN_SECRET_BYTES} bytes"));
    }
    let lowered = secret.to_ascii_lowercase();
    if let Some(marker) = WEAK_SECRET_MARKERS.iter().find(|m| lowered.contains(**m)) {
        return Some(format!("it contains the placeholder marker '{marker}'"));
    }
    if secret.chars().collect::<HashSet<_>>().len() < MIN_DISTINCT_CHARS {
        return Some(format!(
            "it uses fewer than {MIN_DISTINCT_CHARS} distinct characters"
        ));
    }
    if is_repeating_pattern(secret) {
        return Some("it is a short pattern repeated over and over".to_string());
    }
    if shannon_entropy_bits_per_char(secret) < MIN_ENTROPY_BITS_PER_CHAR {
        return Some("its entropy is too low".to_string());
    }
    None
}

/// A full or abbreviated lowercase hex git commit hash (7 to 40 characters).
fn is_commit_hash(value: &str) -> bool {
    (7..=40).contains(&value.len())
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Backend-internal endpoints are reached over the private Docker network: plain `http`, a host,
/// and nothing else (no credentials, query string or fragment that could be smuggled in).
fn validate_internal_http_url(name: &str, value: &str) -> Result<(), String> {
    let url = url::Url::parse(value).map_err(|e| format!("{name} is not a valid URL: {e}"))?;
    if url.scheme() != "http" {
        return Err(format!("{name} must use the http scheme"));
    }
    if url.host_str().is_none() {
        return Err(format!("{name} must include a host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{name} must not embed credentials"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(format!(
            "{name} must not contain a query string or fragment"
        ));
    }
    Ok(())
}

impl AppConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|k| env::var(k).ok())
    }

    pub fn from_lookup<F>(lookup: F) -> Result<Self, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let env_str =
            lookup_trimmed(&lookup, "ENVIRONMENT").unwrap_or_else(|| "production".to_string());
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
                if is_prod && PgConnectOptions::from_str(&url).is_err() {
                    return Err("DATABASE_URL is not a valid PostgreSQL connection string".into());
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
                if is_prod && let Some(reason) = jwt_secret_weakness(&secret) {
                    return Err(format!(
                        "JWT_SECRET in production must be a secure random secret of at least 32 bytes (rejected: {reason}). Generate one with `openssl rand -hex 32`"
                    ));
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

        let valhalla_url = lookup_trimmed(&lookup, "VALHALLA_URL")
            .unwrap_or_else(|| "http://localhost:8002".to_string());
        let photon_url = lookup_trimmed(&lookup, "PHOTON_URL")
            .unwrap_or_else(|| "http://localhost:2322".to_string());
        validate_internal_http_url("VALHALLA_URL", &valhalla_url)?;
        validate_internal_http_url("PHOTON_URL", &photon_url)?;

        let rate_limit_rpm = parse_lookup_var(&lookup, "RATE_LIMIT_REQUESTS_PER_MINUTE", 60u64)?;
        if !(1..=10_000).contains(&rate_limit_rpm) {
            return Err("RATE_LIMIT_REQUESTS_PER_MINUTE must be between 1 and 10000".into());
        }

        let confirmation_threshold =
            parse_lookup_var(&lookup, "HAZARD_CONFIRMATION_THRESHOLD", 3i32)?;
        // A threshold of 1 would let the reporter alone confirm a blocking hazard.
        if !(2..=50).contains(&confirmation_threshold) {
            return Err("HAZARD_CONFIRMATION_THRESHOLD must be between 2 and 50".into());
        }

        let default_ttl_hours = parse_lookup_var(&lookup, "HAZARD_DEFAULT_TTL_HOURS", 24i64)?;
        if !(1..=8760).contains(&default_ttl_hours) {
            return Err(
                "HAZARD_DEFAULT_TTL_HOURS must be between 1 and 8760 hours (1 year)".into(),
            );
        }

        let source_repo_url = lookup_trimmed(&lookup, "SOURCE_REPO_URL")
            .unwrap_or_else(|| "https://github.com/kripiman/Baze".to_string());
        if !source_repo_url.starts_with("https://") {
            return Err("SOURCE_REPO_URL must be an https URL".into());
        }
        // AGPL section 13: /source must point at the code that is actually deployed.
        let git_commit_hash =
            lookup_trimmed(&lookup, "GIT_COMMIT_HASH").unwrap_or_else(|| "dev".to_string());
        if is_prod && !is_commit_hash(&git_commit_hash) {
            return Err(
                "GIT_COMMIT_HASH in production must be the lowercase hex commit hash the image was built from (7 to 40 characters)"
                    .into(),
            );
        }
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

        // The contract lives in contracts/openapi.json; serving the UI in production only widens the surface.
        let enable_api_docs = parse_lookup_var(&lookup, "ENABLE_API_DOCS", is_dev)?;

        let request_timeout_secs = parse_lookup_var(&lookup, "REQUEST_TIMEOUT_SECONDS", 15u64)?;
        if !(1..=120).contains(&request_timeout_secs) {
            return Err("REQUEST_TIMEOUT_SECONDS must be between 1 and 120".into());
        }

        let max_concurrent_requests =
            parse_lookup_var(&lookup, "MAX_CONCURRENT_REQUESTS", 512usize)?;
        if !(1..=100_000).contains(&max_concurrent_requests) {
            return Err("MAX_CONCURRENT_REQUESTS must be between 1 and 100000".into());
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
            enable_api_docs,
            request_timeout_secs,
            max_concurrent_requests,
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
    use uuid::Uuid;

    const GOOD_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    // Fixtures are generated per test run so that no key-like literal lives in the repository.
    fn strong_secret() -> String {
        format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
    }

    fn real_db() -> String {
        format!(
            "postgres://baze_app:{}@db:5432/baze_db",
            Uuid::new_v4().simple()
        )
    }

    /// Minimal valid production environment; each test overrides what it wants to break.
    fn prod_vars<'a>(extra: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        let secret = strong_secret();
        let database_url = real_db();
        move |key| {
            if let Some((_, v)) = extra.iter().find(|(k, _)| *k == key) {
                return Some(v.to_string());
            }
            match key {
                "ENVIRONMENT" => Some("production".into()),
                "DATABASE_URL" => Some(database_url.clone()),
                "JWT_SECRET" => Some(secret.clone()),
                "GIT_COMMIT_HASH" => Some(GOOD_COMMIT.into()),
                _ => None,
            }
        }
    }

    fn mock_env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        }
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
        assert!(
            config
                .err()
                .unwrap()
                .contains("default placeholder password")
        );

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
            (
                "DATABASE_URL",
                "postgres://user:real_prod_password@db:5432/db",
            ),
            ("JWT_SECRET", &strong_secret()),
            ("GIT_COMMIT_HASH", GOOD_COMMIT),
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
        assert!(
            config
                .err()
                .unwrap()
                .contains("RATE_LIMIT_REQUESTS_PER_MINUTE")
        );
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
        assert!(
            config
                .err()
                .unwrap()
                .contains("Invalid IP or CIDR in TRUSTED_PROXIES")
        );
    }

    /// Values of `KEY=VALUE` lines in an env template, ignoring comments and blank lines.
    fn parse_env_template(text: &str) -> HashMap<String, String> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect()
    }

    #[test]
    fn env_example_never_boots_in_production() {
        // Whoever copies infra/.env.example verbatim must not end up with a running, forgeable service.
        let mut vars = parse_env_template(include_str!("../../../../infra/.env.example"));
        vars.insert("ENVIRONMENT".into(), "production".into());
        let result = AppConfig::from_lookup(|k| vars.get(k).cloned());
        assert!(
            result.is_err(),
            "the unmodified .env.example must not boot in production"
        );
    }

    #[test]
    fn env_example_ships_no_secret_values() {
        let vars = parse_env_template(include_str!("../../../../infra/.env.example"));
        for key in [
            "JWT_SECRET",
            "POSTGRES_PASSWORD",
            "APP_DB_PASSWORD",
            "DATABASE_URL",
            "MIGRATION_DATABASE_URL",
        ] {
            assert_eq!(
                vars.get(key).map(String::as_str),
                Some(""),
                "{key} must be empty in the template"
            );
        }
    }

    #[test]
    fn rejects_placeholder_jwt_secrets() {
        let weak = [
            "change_me_to_a_random_32_bytes_secret_key_in_production", // the old .env.example value
            "super_secret_key_at_least_32_bytes_long_here",
            "dev_insecure_jwt_secret_must_be_32_bytes_long_min",
            "REPLACE_THIS_WITH_A_LONG_RANDOM_STRING_0123456789",
            "this-is-an-example-key-0123456789abcdefghijklmnop",
            "placeholder0123456789abcdef0123456789abcdef0123456",
            "MyPassword0123456789abcdefghijklmnopqrstuvwxyz",
            "changeme0123456789abcdef0123456789abcdef01234567",
            "default-key-0123456789-abcdefghijklmnopqrstuvwxyz",
            "short",
        ];
        for secret in weak {
            let err = AppConfig::from_lookup(prod_vars(&[("JWT_SECRET", secret)]))
                .expect_err(&format!("{secret} must be rejected"));
            assert!(err.contains("JWT_SECRET"), "{err}");
        }
    }

    #[test]
    fn rejects_low_entropy_secret() {
        for secret in ["a".repeat(64), "ab".repeat(32), "0123456789".repeat(6)] {
            assert!(
                AppConfig::from_lookup(prod_vars(&[("JWT_SECRET", &secret)])).is_err(),
                "{secret} must be rejected"
            );
        }
    }

    #[test]
    fn accepts_random_hex_and_mixed_secrets() {
        let hex = strong_secret();
        let mixed = format!("{}-{}", Uuid::new_v4(), Uuid::new_v4().simple());
        for secret in [hex, mixed] {
            AppConfig::from_lookup(prod_vars(&[("JWT_SECRET", &secret)]))
                .unwrap_or_else(|e| panic!("{secret} should be accepted: {e}"));
        }
    }

    #[test]
    fn weak_secret_is_tolerated_only_in_development() {
        let config = AppConfig::from_lookup(mock_env(&[
            ("ENVIRONMENT", "development"),
            ("JWT_SECRET", "short"),
        ]))
        .unwrap();
        assert_eq!(config.jwt_secret, "short");
    }

    #[test]
    fn rejects_out_of_range_numbers() {
        let bad = [
            ("RATE_LIMIT_REQUESTS_PER_MINUTE", "0"),
            ("RATE_LIMIT_REQUESTS_PER_MINUTE", "10001"),
            ("HAZARD_CONFIRMATION_THRESHOLD", "0"),
            ("HAZARD_CONFIRMATION_THRESHOLD", "1"),
            ("HAZARD_CONFIRMATION_THRESHOLD", "51"),
            ("HAZARD_DEFAULT_TTL_HOURS", "0"),
            ("PORT", "0"),
            ("PORT", "70000"),
        ];
        for (key, value) in bad {
            let err = AppConfig::from_lookup(prod_vars(&[(key, value)]))
                .expect_err(&format!("{key}={value} must be rejected"));
            assert!(err.contains(key), "{err}");
        }
    }

    #[test]
    fn ttl_extreme_is_rejected_instead_of_panicking_later() {
        let err = AppConfig::from_lookup(prod_vars(&[(
            "HAZARD_DEFAULT_TTL_HOURS",
            "9223372036854775807",
        )]))
        .unwrap_err();
        assert!(err.contains("HAZARD_DEFAULT_TTL_HOURS"), "{err}");
    }

    #[test]
    fn non_numeric_is_an_error_not_a_silent_default() {
        for key in [
            "RATE_LIMIT_REQUESTS_PER_MINUTE",
            "HAZARD_CONFIRMATION_THRESHOLD",
            "HAZARD_DEFAULT_TTL_HOURS",
            "PORT",
        ] {
            let err = AppConfig::from_lookup(prod_vars(&[(key, "abc")])).unwrap_err();
            assert!(err.contains(key), "{err}");
        }
    }

    #[test]
    fn prod_requires_hex_commit() {
        for bad in [
            "dev",
            "development",
            "main",
            "ABCDEF1234567",
            "abc123",
            "xyz1234567",
            &"a".repeat(41),
        ] {
            let err = AppConfig::from_lookup(prod_vars(&[("GIT_COMMIT_HASH", bad)])).unwrap_err();
            assert!(err.contains("GIT_COMMIT_HASH"), "{bad}: {err}");
        }
        let missing = AppConfig::from_lookup(|k| {
            if k == "GIT_COMMIT_HASH" {
                None
            } else {
                prod_vars(&[])(k)
            }
        });
        assert!(missing.unwrap_err().contains("GIT_COMMIT_HASH"));
        for good in ["abcdef1", GOOD_COMMIT] {
            AppConfig::from_lookup(prod_vars(&[("GIT_COMMIT_HASH", good)])).unwrap();
        }
    }

    #[test]
    fn dev_commit_hash_defaults_to_dev() {
        let config = AppConfig::from_lookup(mock_env(&[("ENVIRONMENT", "development")])).unwrap();
        assert_eq!(config.git_commit_hash, "dev");
    }

    #[test]
    fn internal_service_urls_must_be_plain_http_without_extras() {
        let bad = [
            ("VALHALLA_URL", "https://valhalla:8002"),
            ("VALHALLA_URL", "ftp://valhalla"),
            ("VALHALLA_URL", "not a url"),
            ("VALHALLA_URL", "http://user:pw@valhalla:8002"),
            ("PHOTON_URL", "http://photon:2322/?q=1"),
            ("PHOTON_URL", "http://photon:2322/#frag"),
        ];
        for (key, value) in bad {
            let err = AppConfig::from_lookup(prod_vars(&[(key, value)])).expect_err(value);
            assert!(err.contains(key), "{err}");
        }
        AppConfig::from_lookup(prod_vars(&[("VALHALLA_URL", "http://valhalla:8002/")])).unwrap();
    }

    #[test]
    fn source_repo_url_must_be_https() {
        assert!(
            AppConfig::from_lookup(prod_vars(&[("SOURCE_REPO_URL", "http://example.org/repo")]))
                .is_err()
        );
    }

    #[test]
    fn production_database_url_must_parse() {
        let err = AppConfig::from_lookup(prod_vars(&[("DATABASE_URL", "not-a-connection-string")]))
            .unwrap_err();
        assert!(err.contains("DATABASE_URL"), "{err}");
    }

    #[test]
    fn api_docs_are_off_in_production_and_on_in_development_by_default() {
        let prod = AppConfig::from_lookup(prod_vars(&[])).unwrap();
        assert!(!prod.enable_api_docs);
        let dev = AppConfig::from_lookup(mock_env(&[("ENVIRONMENT", "development")])).unwrap();
        assert!(dev.enable_api_docs);
    }

    #[test]
    fn api_docs_can_be_switched_explicitly() {
        let on = AppConfig::from_lookup(prod_vars(&[("ENABLE_API_DOCS", "true")])).unwrap();
        assert!(on.enable_api_docs);
        let err = AppConfig::from_lookup(prod_vars(&[("ENABLE_API_DOCS", "maybe")])).unwrap_err();
        assert!(err.contains("ENABLE_API_DOCS"), "{err}");
    }

    #[test]
    fn server_limits_have_safe_defaults_and_bounds() {
        let config = AppConfig::from_lookup(prod_vars(&[])).unwrap();
        assert_eq!(config.request_timeout_secs, 15);
        assert_eq!(config.max_concurrent_requests, 512);

        for (key, value) in [
            ("REQUEST_TIMEOUT_SECONDS", "0"),
            ("REQUEST_TIMEOUT_SECONDS", "121"),
            ("MAX_CONCURRENT_REQUESTS", "0"),
            ("MAX_CONCURRENT_REQUESTS", "100001"),
        ] {
            let err = AppConfig::from_lookup(prod_vars(&[(key, value)])).expect_err(value);
            assert!(err.contains(key), "{err}");
        }
    }
}
