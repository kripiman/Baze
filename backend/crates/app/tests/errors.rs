// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every failure is a JSON body `{"error": ...}` with a meaningful status code.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{json_request, point, send, signup, test_app};
use serde_json::json;

#[tokio::test]
async fn missing_authorization_is_401_json() {
    let app = test_app();
    let request = json_request(
        "POST",
        "/api/v1/hazards",
        None,
        &json!({"category": "glass", "location": point()}),
    );

    let (status, body) = send(&app, request).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"].is_string(), "{body}");
}

#[tokio::test]
async fn a_garbage_token_is_401() {
    let app = test_app();
    let request = json_request(
        "POST",
        "/api/v1/hazards",
        Some("baze_anon_not-a-real-token"),
        &json!({"category": "glass", "location": point()}),
    );

    let (status, _) = send(&app, request).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn malformed_json_is_400_json() {
    let app = test_app();
    let token = signup(&app).await;
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/hazards")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from("{not json"))
        .unwrap();

    let (status, body) = send(&app, request).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].is_string(), "{body}");
}

#[tokio::test]
async fn wrong_content_type_is_415_json() {
    let app = test_app();
    let token = signup(&app).await;
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/hazards")
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from("category=glass"))
        .unwrap();

    let (status, body) = send(&app, request).await;

    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert!(body["error"].is_string(), "{body}");
}

#[tokio::test]
async fn out_of_range_coordinates_are_400() {
    let app = test_app();
    let token = signup(&app).await;
    let request = json_request(
        "POST",
        "/api/v1/hazards",
        Some(&token),
        &json!({"category": "glass", "location": {"type": "Point", "coordinates": [500.0, 0.0]}}),
    );

    let (status, body) = send(&app, request).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn unknown_category_is_rejected() {
    let app = test_app();
    let token = signup(&app).await;
    let request = json_request(
        "POST",
        "/api/v1/hazards",
        Some(&token),
        &json!({"category": "alien_invasion", "location": point()}),
    );

    let (status, _) = send(&app, request).await;

    assert!(status.is_client_error(), "{status}");
}

#[tokio::test]
async fn voting_on_an_unknown_hazard_is_404_json() {
    let app = test_app();
    let token = signup(&app).await;
    let request = json_request(
        "POST",
        "/api/v1/hazards/00000000-0000-4000-8000-000000000000/vote",
        Some(&token),
        &json!({"vote": 1}),
    );

    let (status, body) = send(&app, request).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].is_string(), "{body}");
}
