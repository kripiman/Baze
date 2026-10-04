// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Schema, constraints and role checks against a real PostgreSQL + PostGIS server.
//! Enabled with `--features db-tests` (CI does; locally point DATABASE_URL at a throwaway server).
#![cfg(feature = "db-tests")]

use baze_app::db::{self, MIGRATOR};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use uuid::Uuid;

async fn account(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("INSERT INTO accounts DEFAULT VALUES RETURNING id")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn insert_hazard(
    pool: &PgPool,
    creator: Uuid,
    category: &str,
    hazard_type: &str,
    description: Option<&str>,
    ttl_hours: i32,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO hazards (creator_account_id, category, hazard_type, description, geom, expires_at) \
         VALUES ($1, $2, $3, $4, ST_SetSRID(ST_MakePoint(-70.65, -33.45), 4326), now() + make_interval(hours => $5)) \
         RETURNING id",
    )
    .bind(creator)
    .bind(category)
    .bind(hazard_type)
    .bind(description)
    .bind(ttl_hours)
    .fetch_one(pool)
    .await
}

fn is_check_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|e| e.code())
        .is_some_and(|code| code == "23514")
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|e| e.code())
        .is_some_and(|code| code == "23505")
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn migrations_create_the_expected_schema(pool: PgPool) {
    for table in ["accounts", "hazards", "hazard_votes"] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1)",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(exists, "table {table} is missing");
    }
    let nullable: String = sqlx::query_scalar(
        "SELECT is_nullable FROM information_schema.columns WHERE table_name = 'hazard_votes' AND column_name = 'voter_net'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(nullable, "NO");
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn migrating_twice_changes_nothing(pool: PgPool) {
    db::migrate(&pool).await.unwrap();
    db::migrate(&pool).await.unwrap();
    assert!(db::assert_schema_current(&pool).await.is_ok());
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn a_coherent_report_is_accepted(pool: PgPool) {
    let creator = account(&pool).await;
    insert_hazard(&pool, creator, "glass", "warning", Some("Vidrios"), 24)
        .await
        .unwrap();
    insert_hazard(&pool, creator, "road_closed", "blocking", None, 24)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn incoherent_reports_are_rejected_by_the_database(pool: PgPool) {
    let creator = account(&pool).await;

    // Category and type must agree; the server derives the type, the database enforces it.
    let err = insert_hazard(&pool, creator, "glass", "blocking", None, 24)
        .await
        .unwrap_err();
    assert!(is_check_violation(&err), "{err}");
    let err = insert_hazard(&pool, creator, "road_closed", "warning", None, 24)
        .await
        .unwrap_err();
    assert!(is_check_violation(&err), "{err}");

    // Unknown category and a report that expires before it was created.
    let err = insert_hazard(&pool, creator, "alien_invasion", "warning", None, 24)
        .await
        .unwrap_err();
    assert!(is_check_violation(&err), "{err}");
    let err = insert_hazard(&pool, creator, "glass", "warning", None, -1)
        .await
        .unwrap_err();
    assert!(is_check_violation(&err), "{err}");
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn the_description_limit_counts_characters(pool: PgPool) {
    let creator = account(&pool).await;
    // 500 two-byte characters are 1000 bytes and must fit; 501 must not.
    insert_hazard(
        &pool,
        creator,
        "glass",
        "warning",
        Some(&"ñ".repeat(500)),
        24,
    )
    .await
    .unwrap();
    let err = insert_hazard(
        &pool,
        creator,
        "glass",
        "warning",
        Some(&"ñ".repeat(501)),
        24,
    )
    .await
    .unwrap_err();
    assert!(is_check_violation(&err), "{err}");
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn counters_cannot_go_negative(pool: PgPool) {
    let creator = account(&pool).await;
    let hazard = insert_hazard(&pool, creator, "glass", "warning", None, 24)
        .await
        .unwrap();
    let err = sqlx::query("UPDATE hazards SET downvotes = -1 WHERE id = $1")
        .bind(hazard)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(is_check_violation(&err), "{err}");
}

async fn ballot(
    pool: &PgPool,
    hazard: Uuid,
    account: Uuid,
    net: &[u8],
    counts: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO hazard_votes (hazard_id, account_id, vote_type, counts, voter_net) VALUES ($1, $2, 1, $3, $4)",
    )
    .bind(hazard)
    .bind(account)
    .bind(counts)
    .bind(net)
    .execute(pool)
    .await
    .map(|_| ())
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn only_one_counted_ballot_per_network_and_report(pool: PgPool) {
    let creator = account(&pool).await;
    let hazard = insert_hazard(&pool, creator, "road_closed", "blocking", None, 24)
        .await
        .unwrap();
    let (a, b, c) = (
        account(&pool).await,
        account(&pool).await,
        account(&pool).await,
    );
    let net_one = [1u8; 16];
    let net_two = [2u8; 16];

    ballot(&pool, hazard, a, &net_one, true).await.unwrap();

    // A second account on the same network cannot also count...
    let err = ballot(&pool, hazard, b, &net_one, true).await.unwrap_err();
    assert!(is_unique_violation(&err), "{err}");
    // ...but its ballot may be recorded as not counting, and another network counts normally.
    ballot(&pool, hazard, b, &net_one, false).await.unwrap();
    ballot(&pool, hazard, c, &net_two, true).await.unwrap();
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn the_same_network_may_count_on_different_reports(pool: PgPool) {
    let creator = account(&pool).await;
    let first = insert_hazard(&pool, creator, "road_closed", "blocking", None, 24)
        .await
        .unwrap();
    let second = insert_hazard(&pool, creator, "flood", "blocking", None, 24)
        .await
        .unwrap();
    let voter = account(&pool).await;
    ballot(&pool, first, voter, &[7u8; 16], true).await.unwrap();
    ballot(&pool, second, voter, &[7u8; 16], true)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn an_account_votes_once_per_report(pool: PgPool) {
    let creator = account(&pool).await;
    let hazard = insert_hazard(&pool, creator, "road_closed", "blocking", None, 24)
        .await
        .unwrap();
    let voter = account(&pool).await;
    ballot(&pool, hazard, voter, &[1u8; 16], true)
        .await
        .unwrap();
    let err = ballot(&pool, hazard, voter, &[2u8; 16], true)
        .await
        .unwrap_err();
    assert!(is_unique_violation(&err), "{err}");
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn a_network_tag_is_exactly_sixteen_bytes_never_an_address(pool: PgPool) {
    let creator = account(&pool).await;
    let hazard = insert_hazard(&pool, creator, "road_closed", "blocking", None, 24)
        .await
        .unwrap();
    let voter = account(&pool).await;
    // An IPv4 address (4 bytes) or an IPv6 one rendered as text cannot be stored as the tag.
    for bad in [&[192u8, 168, 1, 0][..], b"192.168.1.0/24", &[0u8; 17]] {
        let err = ballot(&pool, hazard, voter, bad, true).await.unwrap_err();
        assert!(is_check_violation(&err), "{bad:?}: {err}");
    }
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn deleting_a_report_removes_its_ballots(pool: PgPool) {
    let creator = account(&pool).await;
    let hazard = insert_hazard(&pool, creator, "road_closed", "blocking", None, 24)
        .await
        .unwrap();
    ballot(&pool, hazard, creator, &[1u8; 16], true)
        .await
        .unwrap();

    sqlx::query("DELETE FROM hazards WHERE id = $1")
        .bind(hazard)
        .execute(&pool)
        .await
        .unwrap();

    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM hazard_votes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn the_corridor_index_is_used_for_distance_queries(pool: PgPool) {
    let creator = account(&pool).await;
    for _ in 0..50 {
        insert_hazard(&pool, creator, "pothole", "warning", None, 24)
            .await
            .unwrap();
    }
    sqlx::query("ANALYZE hazards").execute(&pool).await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL enable_seqscan = off")
        .execute(&mut *tx)
        .await
        .unwrap();
    let plan: Vec<String> = sqlx::query(
        "EXPLAIN SELECT id FROM hazards WHERE ST_DWithin(geom::geography, \
         ST_SetSRID(ST_MakePoint(-70.65, -33.45), 4326)::geography, 15)",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.get::<String, _>(0))
    .collect();
    assert!(
        plan.iter().any(|line| line.contains("idx_hazards_geog")),
        "the geography index is not used: {plan:#?}"
    );
}

// ---------------------------------------------------------------- roles and schema checks

/// Connects as a freshly created role with the given attributes.
async fn pool_as_new_role(admin: &PgPool, attributes: &str) -> (PgPool, String) {
    let name = format!("t_{}", Uuid::new_v4().simple());
    sqlx::query(&format!(
        "CREATE ROLE {name} LOGIN PASSWORD 'pw' {attributes}"
    ))
    .execute(admin)
    .await
    .unwrap();
    let options = (*admin.connect_options())
        .clone()
        .username(&name)
        .password("pw");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    (pool, name)
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn a_superuser_is_refused(pool: PgPool) {
    let superuser: bool =
        sqlx::query_scalar("SELECT rolsuper FROM pg_roles WHERE rolname = current_user")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(superuser, "this test needs DATABASE_URL to use a superuser");

    let err = db::assert_least_privilege(&pool).await.unwrap_err();
    assert!(err.contains("SUPERUSER"), "{err}");
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn a_plain_role_is_accepted_and_each_excess_privilege_is_named(pool: PgPool) {
    let (plain, plain_name) = pool_as_new_role(&pool, "NOSUPERUSER NOCREATEDB NOCREATEROLE").await;
    assert!(db::assert_least_privilege(&plain).await.is_ok());
    plain.close().await;

    for (attributes, label) in [
        ("NOSUPERUSER CREATEDB", "CREATEDB"),
        ("NOSUPERUSER CREATEROLE", "CREATEROLE"),
        ("NOSUPERUSER REPLICATION", "REPLICATION"),
    ] {
        let (risky, name) = pool_as_new_role(&pool, attributes).await;
        let err = db::assert_least_privilege(&risky).await.unwrap_err();
        assert!(err.contains(label), "{label}: {err}");
        risky.close().await;
        sqlx::query(&format!("DROP ROLE {name}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query(&format!("DROP ROLE {plain_name}"))
        .execute(&pool)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn the_schema_check_passes_after_migrating(pool: PgPool) {
    db::assert_schema_current(&pool).await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn the_schema_check_fails_when_nothing_was_migrated(pool: PgPool) {
    let err = db::assert_schema_current(&pool).await.unwrap_err();
    assert!(err.contains("migrate"), "{err}");
}

#[sqlx::test(migrations = false)]
async fn the_schema_check_names_the_first_missing_migration(pool: PgPool) {
    // Apply everything but the newest migration by hand.
    sqlx::query(
        "CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY, description TEXT NOT NULL, \
         installed_on TIMESTAMPTZ NOT NULL DEFAULT now(), success BOOLEAN NOT NULL, \
         checksum BYTEA NOT NULL, execution_time BIGINT NOT NULL)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let all: Vec<_> = MIGRATOR.iter().collect();
    for migration in &all[..all.len() - 1] {
        sqlx::query("INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES ($1, $2, true, $3, 0)")
            .bind(migration.version)
            .bind(migration.description.as_ref())
            .bind(migration.checksum.as_ref())
            .execute(&pool)
            .await
            .unwrap();
    }

    let err = db::assert_schema_current(&pool).await.unwrap_err();
    assert!(
        err.contains(&all.last().unwrap().version.to_string()),
        "{err}"
    );
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn the_schema_check_detects_an_altered_migration(pool: PgPool) {
    sqlx::query("UPDATE _sqlx_migrations SET checksum = decode(repeat('ab', 48), 'hex') WHERE version = (SELECT min(version) FROM _sqlx_migrations)")
        .execute(&pool)
        .await
        .unwrap();
    let err = db::assert_schema_current(&pool).await.unwrap_err();
    assert!(err.contains("differs"), "{err}");
}
