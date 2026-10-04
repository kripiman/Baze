// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shared helpers for the HTTP-level tests. They drive the real router in-process with
//! `tower::ServiceExt::oneshot`, so no socket and no database are needed (the Postgres pool is lazy
//! and the services under test do not touch it yet).
#![allow(dead_code)]

use auth::AuthService;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use baze_app::config::AppConfig;
use baze_app::rate_limit::RateLimiter;
use baze_app::router::{AppState, create_router};
use geocoding::PhotonGeocodingService;
use hazards::HazardService;
use http_body_util::BodyExt;
use realtime::RealtimeService;
use routing::ValhallaRoutingService;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;

pub fn test_config() -> AppConfig {
    AppConfig::from_lookup(|key| (key == "ENVIRONMENT").then(|| "development".to_string()))
        .expect("development defaults must be valid")
}

pub fn test_app() -> Router {
    let config = test_config();
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://localhost/dummy")
        .expect("lazy pool");
    let realtime_service = RealtimeService::new(16);
    let hazard_service = Arc::new(HazardService::new(
        pool.clone(),
        config.confirmation_threshold,
        config.default_ttl_hours,
        Arc::new(realtime_service.clone()),
    ));
    let state = AppState {
        auth_service: AuthService::new(pool, config.jwt_secret.clone()),
        routing_service: ValhallaRoutingService::new(
            config.valhalla_url.clone(),
            hazard_service.clone(),
        )
        .expect("routing client"),
        geocoding_service: Arc::new(
            PhotonGeocodingService::new(config.photon_url.clone()).expect("geocoding client"),
        ),
        rate_limiter: RateLimiter::new(),
        hazard_service,
        realtime_service,
        config,
    };
    create_router(Arc::new(state))
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
