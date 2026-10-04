// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The database-backed account service, against a real PostgreSQL (feature `db-tests`).
#![cfg(feature = "db-tests")]

use auth::AuthService;
use auth::token::{self, TokenKeys};
use chrono::{Duration, Utc};
use shared::{AccountService, AppError};
use sqlx::PgPool;
use uuid::Uuid;

const SECRET: &[u8] = b"integration-test-secret-0123456789abcdef";

fn service(pool: PgPool) -> AuthService {
    AuthService::new(pool, TokenKeys::new(SECRET, None), Duration::days(180))
}

fn message(error: AppError) -> String {
    match error {
        AppError::Unauthorized(message) => message,
        other => panic!("expected Unauthorized, got {other:?}"),
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn creating_an_account_persists_it_and_returns_a_working_token(pool: PgPool) {
    let service = service(pool.clone());

    let response = service.create_anonymous_account().await.unwrap();

    let stored: (Uuid, bool) = sqlx::query_as("SELECT id, is_active FROM accounts WHERE id = $1")
        .bind(response.account_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, (response.account_id, true));
    assert!(response.token.starts_with("baze_v2."));
    assert!(response.expires_at > response.created_at + Duration::days(179));

    let context = service.authenticate(&response.token).await.unwrap();
    assert_eq!(context.account_id, response.account_id);
    assert_eq!(context.created_at, response.created_at);
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_creation_time_comes_from_the_database_row(pool: PgPool) {
    let service = service(pool.clone());
    let response = service.create_anonymous_account().await.unwrap();

    let from_db: chrono::DateTime<Utc> =
        sqlx::query_scalar("SELECT created_at FROM accounts WHERE id = $1")
            .bind(response.account_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(response.created_at, from_db);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_deactivated_account_is_refused_at_once(pool: PgPool) {
    let service = service(pool.clone());
    let response = service.create_anonymous_account().await.unwrap();
    assert!(service.authenticate(&response.token).await.is_ok());

    sqlx::query("UPDATE accounts SET is_active = false WHERE id = $1")
        .bind(response.account_id)
        .execute(&pool)
        .await
        .unwrap();

    let banned = message(service.authenticate(&response.token).await.unwrap_err());
    let garbage = message(service.authenticate("baze_v2.nonsense").await.unwrap_err());
    assert_eq!(banned, garbage, "a ban must look like any other bad token");
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_valid_signature_for_an_account_that_does_not_exist_is_refused(pool: PgPool) {
    let service = service(pool);
    let keys = TokenKeys::new(SECRET, None);
    let (token, _) = token::issue(&keys, Uuid::new_v4(), Utc::now(), Duration::days(1));

    let error = message(service.authenticate(&token).await.unwrap_err());

    assert_eq!(error, "Invalid authorization token");
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_expired_token_is_refused_even_though_the_account_exists(pool: PgPool) {
    let service = service(pool);
    let response = service.create_anonymous_account().await.unwrap();
    let keys = TokenKeys::new(SECRET, None);
    let (expired, _) = token::issue(
        &keys,
        response.account_id,
        Utc::now() - Duration::days(10),
        Duration::days(1),
    );

    let error = message(service.authenticate(&expired).await.unwrap_err());

    assert_eq!(error, "Token expired");
}

#[sqlx::test(migrations = "../../migrations")]
async fn tokens_survive_a_secret_rotation_until_the_previous_secret_is_dropped(pool: PgPool) {
    let before = service(pool.clone());
    let response = before.create_anonymous_account().await.unwrap();

    let new_secret = b"the-secret-after-the-rotation-0123456789".to_vec();
    let rotated = AuthService::new(
        pool.clone(),
        TokenKeys::new(new_secret.clone(), Some(SECRET.to_vec())),
        Duration::days(180),
    );
    assert!(rotated.authenticate(&response.token).await.is_ok());
    let fresh = rotated.create_anonymous_account().await.unwrap();
    assert!(rotated.authenticate(&fresh.token).await.is_ok());

    let dropped = AuthService::new(pool, TokenKeys::new(new_secret, None), Duration::days(180));
    assert!(dropped.authenticate(&response.token).await.is_err());
    assert!(dropped.authenticate(&fresh.token).await.is_ok());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_dead_database_is_an_internal_error_not_an_authentication_failure(pool: PgPool) {
    let service = service(pool.clone());
    let response = service.create_anonymous_account().await.unwrap();
    pool.close().await;

    let error = service.authenticate(&response.token).await.unwrap_err();

    assert!(matches!(error, AppError::Internal(_)), "{error:?}");
}
