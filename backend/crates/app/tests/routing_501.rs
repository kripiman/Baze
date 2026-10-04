// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Until the Valhalla and Photon clients exist the API must say so. A made-up route returned with
//! HTTP 200 is dangerous: a cyclist would follow it.

mod common;

use axum::http::StatusCode;
use common::{get, json_request, point, send, test_app};
use serde_json::json;

#[tokio::test]
async fn routing_answers_501_instead_of_a_fabricated_route() {
    let app = test_app();
    let request = json_request(
        "POST",
        "/api/v1/routing/route",
        None,
        &json!({
            "origin": point(),
            "destination": {"type": "Point", "coordinates": [-71.55, -33.02]},
        }),
    );

    let (status, body) = send(&app, request).await;

    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
    assert!(body["error"].as_str().unwrap().contains("not available"));
    assert!(
        body.get("geometry").is_none(),
        "no route data may be returned: {body}"
    );
}

#[tokio::test]
async fn geocoding_answers_501_instead_of_an_empty_result() {
    let app = test_app();

    let (status, body) = send(&app, get("/api/v1/geocoding/search?q=Plaza%20de%20Armas")).await;

    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn invalid_routing_input_is_still_rejected_before_reaching_the_engine() {
    let app = test_app();
    let request = json_request(
        "POST",
        "/api/v1/routing/route",
        None,
        &json!({"origin": point(), "destination": point()}),
    );

    let (status, _) = send(&app, request).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}
