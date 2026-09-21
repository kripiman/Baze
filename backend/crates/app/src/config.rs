// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::env;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub environment: String,
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
}

impl AppConfig {
    pub fn from_env() -> Result<Self, String> {
        let environment = env::var("ENVIRONMENT").unwrap_or_else(|_| "production".to_string());
        let is_dev = environment.eq_ignore_ascii_case("development");
        let is_prod = !is_dev;

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .unwrap_or_else(|_| "8080".to_string())
            .parse::<u16>()
            .map_err(|e| format!("Invalid PORT: {}", e))?;

        // Validación estricta en producción para evitar 'fail-open' con secretos por defecto
        let database_url = match env::var("DATABASE_URL") {
            Ok(url) => {
                if is_prod && url.contains("baze_secure_password_replace_in_production") {
                    return Err(
                        "DATABASE_URL in production environment cannot use default placeholder password"
                            .into(),
                    );
                }
                url
            }
            Err(_) => {
                if is_prod {
                    return Err("DATABASE_URL must be explicitly provided in production".into());
                }
                "postgres://baze_user:baze_secure_password_replace_in_production@localhost:5432/baze_db".to_string()
            }
        };

        let jwt_secret = match env::var("JWT_SECRET") {
            Ok(secret) => {
                if is_prod && (secret.contains("dev_insecure") || secret.len() < 32) {
                    return Err(
                        "JWT_SECRET in production must be a secure random secret of at least 32 bytes"
                            .into(),
                    );
                }
                secret
            }
            Err(_) => {
                if is_prod {
                    return Err("JWT_SECRET must be explicitly provided in production".into());
                }
                "dev_insecure_jwt_secret_must_be_32_bytes_long_min".to_string()
            }
        };

        let valhalla_url =
            env::var("VALHALLA_URL").unwrap_or_else(|_| "http://localhost:8002".to_string());
        let photon_url =
            env::var("PHOTON_URL").unwrap_or_else(|_| "http://localhost:2322".to_string());

        let rate_limit_rpm = env::var("RATE_LIMIT_REQUESTS_PER_MINUTE")
            .unwrap_or_else(|_| "60".to_string())
            .parse::<u64>()
            .unwrap_or(60);
        let confirmation_threshold = env::var("HAZARD_CONFIRMATION_THRESHOLD")
            .unwrap_or_else(|_| "3".to_string())
            .parse::<i32>()
            .unwrap_or(3);
        let default_ttl_hours = env::var("HAZARD_DEFAULT_TTL_HOURS")
            .unwrap_or_else(|_| "24".to_string())
            .parse::<i64>()
            .unwrap_or(24);
        let source_repo_url = env::var("SOURCE_REPO_URL")
            .unwrap_or_else(|_| "https://github.com/kripiman/Baze".to_string());
        let git_commit_hash = env::var("GIT_COMMIT_HASH").unwrap_or_else(|_| "dev".to_string());

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
        })
    }

    pub fn is_development(&self) -> bool {
        self.environment.eq_ignore_ascii_case("development")
    }

    pub fn is_production(&self) -> bool {
        !self.is_development()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Mutex para evitar colisiones en tests que modifican variables de entorno
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_unset_environment_defaults_to_production_and_fails_closed() {
        let _guard = ENV_MUTEX.lock().unwrap();
        unsafe {
            env::remove_var("ENVIRONMENT");
            env::remove_var("DATABASE_URL");
            env::remove_var("JWT_SECRET");
        }

        let config = AppConfig::from_env();
        assert!(config.is_err());
        let err = config.err().unwrap();
        assert!(err.contains("must be explicitly provided in production"));
    }

    #[test]
    fn test_explicit_development_permits_defaults() {
        let _guard = ENV_MUTEX.lock().unwrap();
        unsafe {
            env::set_var("ENVIRONMENT", "development");
            env::remove_var("DATABASE_URL");
            env::remove_var("JWT_SECRET");
        }

        let config = AppConfig::from_env().unwrap();
        assert!(config.is_development());
        assert!(!config.is_production());
        assert_eq!(config.environment, "development");
        assert!(config.jwt_secret.contains("dev_insecure"));
    }

    #[test]
    fn test_production_rejects_placeholder_secrets() {
        let _guard = ENV_MUTEX.lock().unwrap();
        unsafe {
            env::set_var("ENVIRONMENT", "production");
            env::set_var("DATABASE_URL", "postgres://user:baze_secure_password_replace_in_production@db:5432/db");
            env::set_var("JWT_SECRET", "super_secret_key_at_least_32_bytes_long_here");
        }

        let config = AppConfig::from_env();
        assert!(config.is_err());
        assert!(config.err().unwrap().contains("default placeholder password"));

        unsafe {
            env::set_var("DATABASE_URL", "postgres://user:real_prod_pw@db:5432/db");
            env::set_var("JWT_SECRET", "short_secret");
        }
        let config2 = AppConfig::from_env();
        assert!(config2.is_err());
        assert!(config2.err().unwrap().contains("at least 32 bytes"));
    }

    #[test]
    fn test_production_valid_configuration() {
        let _guard = ENV_MUTEX.lock().unwrap();
        unsafe {
            env::set_var("ENVIRONMENT", "production");
            env::set_var("DATABASE_URL", "postgres://user:real_prod_password@db:5432/db");
            env::set_var("JWT_SECRET", "super_secure_random_production_key_32_bytes");
        }

        let config = AppConfig::from_env().unwrap();
        assert!(config.is_production());
        assert!(!config.is_development());
    }
}
