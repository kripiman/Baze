// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Input validation as the client sees it.

mod common;

use axum::http::StatusCode;
use common::{json_request, point, send, signup, test_app};
use serde_json::{Value, json};

async fn post_description(description: &str) -> (StatusCode, Value) {
    let app = test_app();
    let token = signup(&app).await;
    let request = json_request(
        "POST",
        "/api/v1/hazards",
        Some(&token),
        &json!({"category": "glass", "description": description, "location": point()}),
    );
    send(&app, request).await
}

#[tokio::test]
async fn the_limit_counts_characters_so_accented_text_is_not_penalised() {
    let (status, body) = post_description(&"ñ".repeat(300)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, _) = post_description(&"ñ".repeat(500)).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = post_description(&"ñ".repeat(501)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("500 characters"));
}

#[tokio::test]
async fn control_characters_are_rejected() {
    for bad in ["\u{0}x", "a\u{1b}b", "tab\tseparated"] {
        let (status, body) = post_description(bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad:?}: {body}");
    }
}

#[tokio::test]
async fn bidirectional_overrides_are_rejected() {
    let (status, body) = post_description("safe\u{202e}evil").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn the_stored_description_is_trimmed_and_blank_becomes_null() {
    let (status, body) = post_description("  Vidrios en la ciclovía  ").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["description"], "Vidrios en la ciclovía");

    let (status, body) = post_description("   ").await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(body["description"].is_null(), "{body}");
}

#[tokio::test]
async fn the_hazard_type_is_derived_from_the_category() {
    let app = test_app();
    let token = signup(&app).await;
    for (category, expected) in [("glass", "warning"), ("road_closed", "blocking")] {
        let request = json_request(
            "POST",
            "/api/v1/hazards",
            Some(&token),
            // A client-supplied hazard_type is ignored: the server decides from the category.
            &json!({"category": category, "hazard_type": "blocking", "location": point()}),
        );
        let (status, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["hazard_type"], expected, "{category}");
    }
}

#[tokio::test]
async fn the_public_payload_never_names_the_creator() {
    let app = test_app();
    let token = signup(&app).await;
    let request = json_request(
        "POST",
        "/api/v1/hazards",
        Some(&token),
        &json!({"category": "glass", "location": point()}),
    );
    let (status, created) = send(&app, request).await;
    assert_eq!(status, StatusCode::CREATED);

    let (_, listing) = send(
        &app,
        common::get("/api/v1/hazards?min_lon=-70.7&min_lat=-33.5&max_lon=-70.6&max_lat=-33.4"),
    )
    .await;

    for payload in [created.to_string(), listing.to_string()] {
        assert!(!payload.contains("creator"), "{payload}");
        assert!(!payload.contains("account"), "{payload}");
    }
}
