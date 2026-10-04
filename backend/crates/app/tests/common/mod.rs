// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shared helpers for the HTTP-level tests. They drive the real router in-process with
//! `tower::ServiceExt::oneshot`, so no socket and no database are needed (the Postgres pool is lazy
//! and the services under test do not touch it; accounts are an in-memory fake).
#![allow(dead_code)]

use async_trait::async_trait;
use auth::token::{self, TokenKeys};
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use baze_app::config::AppConfig;
use baze_app::rate_limit::RateLimiter;
use baze_app::router::{AppState, build_app};
use chrono::{DateTime, Duration, Utc};
use geocoding::PhotonGeocodingService;
use hazards::HazardService;
use http_body_util::BodyExt;
use realtime::RealtimeService;
use routing::ValhallaRoutingService;
use serde_json::{Value, json};
use shared::{AccountContext, AccountService, AppError, AuthResponse, GeocodingProvider};
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

/// In-memory stand-in for the database-backed account service. It issues and verifies the real
/// tokens (the same `auth::token` code), so HTTP tests exercise authentication faithfully without a
/// database. The real service is covered by the `db-tests` of the auth crate.
pub struct FakeAccounts {
    keys: TokenKeys,
    ttl: Duration,
    accounts: Mutex<HashMap<Uuid, (DateTime<Utc>, bool)>>,
}

impl FakeAccounts {
    pub fn new(secret: &str) -> Self {
        Self {
            keys: TokenKeys::new(secret.as_bytes(), None),
            ttl: Duration::days(180),
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Registers an account created at `created_at` and returns its id and a valid token.
    pub fn add_account(&self, created_at: DateTime<Utc>) -> (Uuid, String) {
        let account_id = Uuid::new_v4();
        self.accounts
            .lock()
            .unwrap()
            .insert(account_id, (created_at, true));
        let (token, _) = token::issue(&self.keys, account_id, Utc::now(), self.ttl);
        (account_id, token)
    }

    /// What an operator does to an abusive account: its token stops working at once.
    pub fn deactivate(&self, account_id: Uuid) {
        if let Some(entry) = self.accounts.lock().unwrap().get_mut(&account_id) {
            entry.1 = false;
        }
    }
}

#[async_trait]
impl AccountService for FakeAccounts {
    async fn create_anonymous_account(&self) -> Result<AuthResponse, AppError> {
        let now = Utc::now();
        let (account_id, token) = self.add_account(now);
        let claims = token::verify(&self.keys, &token, now).expect("a token just issued verifies");
        Ok(AuthResponse {
            account_id,
            token,
            created_at: now,
            expires_at: claims.expires_at,
        })
    }

    async fn authenticate(&self, token: &str) -> Result<AccountContext, AppError> {
        let invalid = || AppError::Unauthorized("Invalid authorization token".into());
        let claims = token::verify(&self.keys, token, Utc::now()).map_err(|error| match error {
            token::TokenError::Expired => AppError::Unauthorized("Token expired".into()),
            _ => invalid(),
        })?;
        match self.accounts.lock().unwrap().get(&claims.account_id) {
            Some((created_at, true)) => Ok(AccountContext {
                account_id: claims.account_id,
                created_at: *created_at,
            }),
            _ => Err(invalid()),
        }
    }
}

pub fn test_accounts(config: &AppConfig) -> Arc<FakeAccounts> {
    Arc::new(FakeAccounts::new(&config.jwt_secret))
}

pub fn test_config() -> AppConfig {
    AppConfig::from_lookup(|key| (key == "ENVIRONMENT").then(|| "development".to_string()))
        .expect("development defaults must be valid")
}

pub fn test_app() -> Router {
    test_app_with(test_config())
}

pub fn test_app_with(config: AppConfig) -> Router {
    let geocoder =
        Arc::new(PhotonGeocodingService::new(config.photon_url.clone()).expect("geocoding client"));
    test_app_with_geocoder(config, geocoder)
}

/// The geocoder is the one collaborator that is trivial to replace, which makes it the handle for
/// simulating a slow or failing backing service.
pub fn test_app_with_geocoder(config: AppConfig, geocoder: Arc<dyn GeocodingProvider>) -> Router {
    let accounts = test_accounts(&config);
    build_test_app(config, accounts, geocoder)
}

/// For tests that need to create accounts of a given age or deactivate them.
pub fn test_app_with_accounts(config: AppConfig, accounts: Arc<FakeAccounts>) -> Router {
    let geocoder =
        Arc::new(PhotonGeocodingService::new(config.photon_url.clone()).expect("geocoding client"));
    build_test_app(config, accounts, geocoder)
}

fn build_test_app(
    config: AppConfig,
    accounts: Arc<FakeAccounts>,
    geocoder: Arc<dyn GeocodingProvider>,
) -> Router {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://localhost/dummy")
        .expect("lazy pool");
    let realtime_service = RealtimeService::new(16);
    let hazard_service = Arc::new(HazardService::new(
        pool,
        config.confirmation_threshold,
        config.default_ttl_hours,
        Arc::new(realtime_service.clone()),
    ));
    let state = AppState {
        auth_service: accounts,
        routing_service: ValhallaRoutingService::new(
            config.valhalla_url.clone(),
            hazard_service.clone(),
        )
        .expect("routing client"),
        geocoding_service: geocoder,
        rate_limiter: RateLimiter::new(),
        hazard_service,
        realtime_service,
        config,
    };
    build_app(Arc::new(state))
}

pub fn json_request(method: &str, uri: &str, token: Option<&str>, body: &Value) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

pub fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

/// Sends one request and returns the status and the JSON body (`Value::Null` when the body is empty).
pub async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.expect("infallible");
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, body)
}

pub async fn signup(app: &Router) -> String {
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/anonymous")
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(app, request).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["token"]
        .as_str()
        .expect("token in response")
        .to_string()
}

pub fn point() -> Value {
    json!({"type": "Point", "coordinates": [-70.65, -33.45]})
}
