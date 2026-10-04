// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Limits that keep one client from degrading the service for everyone else.

mod common;

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{
    get, json_request, point, send, signup, test_app, test_app_with, test_app_with_geocoder,
    test_config,
};
use serde_json::json;
use shared::{AppError, GeocodingItem, GeocodingProvider};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;

const CITY_BBOX: &str = "min_lon=-70.7&min_lat=-33.5&max_lon=-70.6&max_lat=-33.4";
const WORLD_BBOX: &str = "min_lon=-180&min_lat=-90&max_lon=180&max_lat=90";

// ---------------------------------------------------------------- request bodies

#[tokio::test]
async fn oversized_body_is_413_not_400() {
    let app = test_app();
    let token = signup(&app).await;
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/hazards")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(vec![b'x'; 20 * 1024]))
        .unwrap();

    let (status, body) = send(&app, request).await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn a_realistic_report_fits_comfortably_under_the_limit() {
    let app = test_app();
    let token = signup(&app).await;
    let description = "a".repeat(400);
    let request = json_request(
        "POST",
        "/api/v1/hazards",
        Some(&token),
        &json!({"category": "pothole", "description": description, "location": point()}),
    );

    let (status, body) = send(&app, request).await;

    assert_eq!(status, StatusCode::CREATED, "{body}");
}

// ---------------------------------------------------------------- API documentation

#[tokio::test]
async fn docs_are_not_served_when_disabled() {
    let mut config = test_config();
    config.enable_api_docs = false;
    let app = test_app_with(config);

    for path in ["/swagger-ui/", "/api-docs/openapi.json"] {
        let (status, _) = send(&app, get(path)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn docs_are_served_when_enabled() {
    let mut config = test_config();
    config.enable_api_docs = true;
    let app = test_app_with(config);

    let (status, body) = send(&app, get("/api-docs/openapi.json")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["info"]["title"], "Baze API");
}

// ---------------------------------------------------------------- Authorization header

#[tokio::test]
async fn bearer_scheme_is_case_insensitive() {
    let app = test_app();
    let token = signup(&app).await;

    for scheme in ["Bearer", "bearer", "BEARER"] {
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/hazards")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("{scheme} {token}"))
            .body(Body::from(
                json!({"category": "glass", "location": point()}).to_string(),
            ))
            .unwrap();
        let (status, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::CREATED, "{scheme}: {body}");
    }
}

// ---------------------------------------------------------------- bounding boxes

#[tokio::test]
async fn listing_rejects_a_world_sized_box() {
    let app = test_app();

    let (status, body) = send(&app, get(&format!("/api/v1/hazards?{WORLD_BBOX}"))).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("too large"));
}

#[tokio::test]
async fn listing_accepts_a_city_sized_box() {
    let app = test_app();

    let (status, body) = send(&app, get(&format!("/api/v1/hazards?{CITY_BBOX}"))).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn the_stream_allows_a_route_sized_box_but_not_a_continent() {
    let app = test_app();
    let route = "min_lon=-71.6&min_lat=-33.7&max_lon=-70.4&max_lat=-33.0"; // Santiago - Valparaiso, 1.2 x 0.7
    let continent = "min_lon=-80&min_lat=-60&max_lon=-60&max_lat=-10";

    let ok = app
        .clone()
        .oneshot(get(&format!("/api/v1/realtime/sse?{route}")))
        .await
        .unwrap();
    let (rejected, _) = send(&app, get(&format!("/api/v1/realtime/sse?{continent}"))).await;

    assert_eq!(ok.status(), StatusCode::OK);
    assert_eq!(rejected, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------- rate limiting

#[tokio::test]
async fn rejected_requests_say_when_to_retry() {
    let app = test_app();
    let limit = test_config().rate_limit_rpm;

    let mut last = None;
    for _ in 0..=limit {
        last = Some(
            app.clone()
                .oneshot(get("/api/v1/geocoding/search?q=a"))
                .await
                .unwrap(),
        );
    }
    let response = last.unwrap();

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry: u64 = response
        .headers()
        .get(header::RETRY_AFTER)
        .expect("Retry-After header")
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry), "retry-after {retry}");
}

#[tokio::test]
async fn changing_the_authorization_header_does_not_buy_a_fresh_budget() {
    let app = test_app();
    let limit = test_config().rate_limit_rpm as usize;

    let mut throttled = 0;
    for i in 0..limit + 40 {
        let request = Request::builder()
            .uri("/api/v1/geocoding/search?q=a")
            .header(header::AUTHORIZATION, format!("Bearer attacker-{i}"))
            .body(Body::empty())
            .unwrap();
        if app.clone().oneshot(request).await.unwrap().status() == StatusCode::TOO_MANY_REQUESTS {
            throttled += 1;
        }
    }

    assert_eq!(throttled, 40);
}

// ---------------------------------------------------------------- request deadline

struct SlowGeocoder(Duration);

#[async_trait]
impl GeocodingProvider for SlowGeocoder {
    async fn search_address(
        &self,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<GeocodingItem>, AppError> {
        tokio::time::sleep(self.0).await;
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn a_request_that_takes_too_long_gets_408() {
    let mut config = test_config();
    config.request_timeout_secs = 1;
    let app = test_app_with_geocoder(config, Arc::new(SlowGeocoder(Duration::from_secs(30))));

    let started = Instant::now();
    let (status, _) = send(&app, get("/api/v1/geocoding/search?q=a")).await;

    assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_request_within_the_deadline_is_unaffected() {
    let mut config = test_config();
    config.request_timeout_secs = 5;
    let app = test_app_with_geocoder(config, Arc::new(SlowGeocoder(Duration::from_millis(50))));

    let (status, body) = send(&app, get("/api/v1/geocoding/search?q=a")).await;

    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn in_flight_requests_are_capped_globally() {
    let mut config = test_config();
    config.max_concurrent_requests = 2;
    config.request_timeout_secs = 10;
    config.rate_limit_rpm = 1000;
    let app = test_app_with_geocoder(config, Arc::new(SlowGeocoder(Duration::from_millis(600))));

    // Four 600 ms requests through a cap of 2: the second pair has to queue behind the first,
    // so the batch cannot finish faster than two waves.
    let started = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..4 {
        let app = app.clone();
        handles.push(tokio::spawn(async move {
            app.oneshot(get("/api/v1/geocoding/search?q=a"))
                .await
                .unwrap()
                .status()
        }));
    }
    let mut statuses = Vec::new();
    for handle in handles {
        statuses.push(handle.await.unwrap());
    }

    assert!(
        started.elapsed() >= Duration::from_millis(1100),
        "{:?}",
        started.elapsed()
    );
    assert!(
        statuses.iter().all(|s| *s == StatusCode::OK),
        "{statuses:?}"
    );
}
