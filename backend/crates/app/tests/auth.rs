// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Authentication as the client experiences it: what a token looks like, and every way of
//! presenting one that must be refused.

mod common;

use auth::token::{self, TokenKeys};
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use common::{
    FakeAccounts, get, json_request, point, send, signup, test_app, test_app_with_accounts,
    test_config,
};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

fn report() -> Value {
    json!({"category": "glass", "location": point()})
}

async fn post_report(app: &axum::Router, token: &str) -> (StatusCode, Value) {
    send(
        app,
        json_request("POST", "/api/v1/hazards", Some(token), &report()),
    )
    .await
}

#[tokio::test]
async fn signup_returns_a_v2_token_that_expires() {
    let app = test_app();
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/api/v1/auth/anonymous")
        .body(axum::body::Body::empty())
        .unwrap();

    let (status, body) = send(&app, request).await;

    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert!(body["token"].as_str().unwrap().starts_with("baze_v2."));
    assert!(Uuid::parse_str(body["account_id"].as_str().unwrap()).is_ok());
    let created: chrono::DateTime<Utc> = body["created_at"].as_str().unwrap().parse().unwrap();
    let expires: chrono::DateTime<Utc> = body["expires_at"].as_str().unwrap().parse().unwrap();
    assert!(
        expires > created + Duration::days(100),
        "{created} -> {expires}"
    );
}

#[tokio::test]
async fn a_valid_token_is_accepted() {
    let app = test_app();
    let token = signup(&app).await;
    let (status, body) = post_report(&app, &token).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[tokio::test]
async fn only_the_canonical_spelling_of_a_token_is_accepted() {
    let app = test_app();
    let token = signup(&app).await;
    let rest = token.strip_prefix("baze_v2.").unwrap();
    let (payload, mac) = rest.split_once('.').unwrap();

    let variants = [
        format!("baze_v2.{}.{mac}", payload.to_uppercase()),
        format!("baze_v2.{payload}.{}", mac.to_uppercase()),
        format!("BAZE_V2.{payload}.{mac}"),
        format!("baze_v2.{payload}.{mac}.x"),
        format!("baze_v2.{payload}"),
    ];
    for variant in variants {
        let (status, _) = post_report(&app, &variant).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{variant}");
    }
}

#[tokio::test]
async fn legacy_v1_tokens_are_refused_even_when_forged_with_the_real_secret() {
    use hmac::{Hmac, Mac};
    let config = test_config();
    let app = test_app();
    let account = Uuid::new_v4();
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(config.jwt_secret.as_bytes()).unwrap();
    mac.update(account.as_bytes());
    let v1 = format!(
        "baze_anon_{account}.{}",
        hex::encode(mac.finalize().into_bytes())
    );

    let (status, _) = post_report(&app, &v1).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_token_signed_with_another_secret_is_refused() {
    let app = test_app();
    let other = TokenKeys::new(b"not-the-secret-this-server-uses-0123456".to_vec(), None);
    let (token, _) = token::issue(&other, Uuid::new_v4(), Utc::now(), Duration::days(1));

    let (status, _) = post_report(&app, &token).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_expired_token_says_so_but_a_forged_one_does_not() {
    let config = test_config();
    let accounts = Arc::new(FakeAccounts::new(&config.jwt_secret));
    let app = test_app_with_accounts(config.clone(), accounts.clone());
    let keys = TokenKeys::new(config.jwt_secret.as_bytes(), None);
    let (account, _) = accounts.add_account(Utc::now() - Duration::days(400));

    // Issued 200 days ago for 30 days: authentic, but over.
    let (expired, _) = token::issue(
        &keys,
        account,
        Utc::now() - Duration::days(200),
        Duration::days(30),
    );
    let (status, body) = post_report(&app, &expired).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "Token expired");

    // The same shape signed by someone else is just invalid: expiry is not an oracle.
    let other = TokenKeys::new(b"attacker-controlled-secret-0123456789".to_vec(), None);
    let (forged, _) = token::issue(
        &other,
        account,
        Utc::now() - Duration::days(200),
        Duration::days(30),
    );
    let (status, body) = post_report(&app, &forged).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "Invalid authorization token");
}

#[tokio::test]
async fn a_deactivated_account_stops_working_immediately_and_looks_like_a_bad_token() {
    let config = test_config();
    let accounts = Arc::new(FakeAccounts::new(&config.jwt_secret));
    let app = test_app_with_accounts(config, accounts.clone());
    let (account, token) = accounts.add_account(Utc::now() - Duration::days(1));
    assert_eq!(post_report(&app, &token).await.0, StatusCode::CREATED);

    accounts.deactivate(account);
    let (status, banned) = post_report(&app, &token).await;
    let (_, garbage) = post_report(&app, "baze_v2.nonsense").await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Indistinguishable from any other invalid token, so a banned user learns nothing about the ban.
    assert_eq!(banned["error"], garbage["error"]);
}

#[tokio::test]
async fn an_unknown_account_with_a_valid_signature_is_refused() {
    let config = test_config();
    let app = test_app();
    let keys = TokenKeys::new(config.jwt_secret.as_bytes(), None);
    let (token, _) = token::issue(&keys, Uuid::new_v4(), Utc::now(), Duration::days(1));

    let (status, _) = post_report(&app, &token).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn reading_hazards_needs_no_token() {
    let app = test_app();
    let (status, _) = send(
        &app,
        get("/api/v1/hazards?min_lon=-70.7&min_lat=-33.5&max_lon=-70.6&max_lat=-33.4"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
