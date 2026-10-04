// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The hazard store against a real PostgreSQL + PostGIS (feature `db-tests`).
//!
//! Each test gets its own database with the migrations applied (`sqlx::test`).

use super::*;
use shared::GeoJsonPoint;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashSet;
use std::sync::Mutex;

const SECRET: &[u8] = b"hazard-store-test-secret-0123456789abcdef";

#[derive(Default)]
struct Recorder(Mutex<Vec<Hazard>>);

impl HazardNotifier for Recorder {
    fn broadcast_hazard(&self, hazard: &Hazard) {
        self.0.lock().unwrap().push(hazard.clone());
    }
}

fn service(pool: &PgPool, threshold: i32) -> HazardService {
    HazardService::new(
        pool.clone(),
        threshold,
        24,
        SECRET,
        Arc::new(Recorder::default()),
    )
}

async fn account(pool: &PgPool) -> AccountContext {
    let account_id = Uuid::new_v4();
    let created_at =
        sqlx::query_scalar("INSERT INTO accounts (id) VALUES ($1) RETURNING created_at")
            .bind(account_id)
            .fetch_one(pool)
            .await
            .unwrap();
    AccountContext {
        account_id,
        created_at,
    }
}

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

fn report(category: HazardCategory) -> CreateHazardRequest {
    CreateHazardRequest {
        category,
        description: None,
        location: GeoJsonPoint::new(-70.65, -33.45),
    }
}

fn blocking_report() -> CreateHazardRequest {
    report(HazardCategory::RoadClosed)
}

fn city_bbox() -> BoundingBox {
    BoundingBox {
        min_lon: -70.7,
        min_lat: -33.5,
        max_lon: -70.6,
        max_lat: -33.4,
    }
}

/// Writes a report straight into the table, bypassing the service: for states the service never
/// produces (already expired) or that would take many votes to reach.
async fn insert_hazard(
    pool: &PgPool,
    creator: Uuid,
    category: &str,
    status: &str,
    (lon, lat): (f64, f64),
    expires_in_hours: i32,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO hazards (id, creator_account_id, category, hazard_type, status, upvotes, downvotes, \
                              geom, created_at, expires_at) \
         VALUES ($1, $2, $3, \
                 CASE WHEN $3 IN ('glass', 'pothole', 'debris') THEN 'warning' ELSE 'blocking' END, \
                 $4, 1, 0, ST_SetSRID(ST_MakePoint($5, $6), 4326), \
                 now() - interval '2 hours', now() + make_interval(hours => $7))",
    )
    .bind(id)
    .bind(creator)
    .bind(category)
    .bind(status)
    .bind(lon)
    .bind(lat)
    .bind(expires_in_hours)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// The stored counters must always equal a recount of the ballots, and no network may have two
/// counted ballots on one report.
async fn assert_consistent(pool: &PgPool, hazard_id: Uuid) -> Hazard {
    let (up, down): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE vote_type = 1), count(*) FILTER (WHERE vote_type = -1) \
         FROM hazard_votes WHERE hazard_id = $1 AND counts",
    )
    .bind(hazard_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let row: HazardRow = sqlx::query_as(&format!("SELECT {COLUMNS} FROM hazards WHERE id = $1"))
        .bind(hazard_id)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        (i64::from(row.upvotes), i64::from(row.downvotes)),
        (up, down)
    );

    let (duplicated,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM (SELECT 1 FROM hazard_votes WHERE hazard_id = $1 AND counts \
         GROUP BY voter_net HAVING count(*) > 1) d",
    )
    .bind(hazard_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(duplicated, 0);
    row.into_hazard().unwrap()
}

// ------------------------------------------------------------------ creating and voting

#[sqlx::test(migrations = "../../migrations")]
async fn test_create_warning_is_confirmed(pool: PgPool) {
    let service = service(&pool, 3);
    let creator = account(&pool).await;

    let hazard = service
        .create_hazard(&creator, &report(HazardCategory::Glass), ip("192.168.1.10"))
        .await
        .unwrap();

    assert_eq!(hazard.status, HazardStatus::Confirmed);
    assert_eq!((hazard.upvotes, hazard.downvotes), (1, 0));
    assert_eq!(hazard.hazard_type, HazardType::Warning);
    assert_eq!(hazard.location.coordinates, [-70.65, -33.45]);
    assert!(hazard.expires_at > hazard.created_at + chrono::Duration::hours(23));
    assert_consistent(&pool, hazard.id).await;
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_create_blocking_is_unconfirmed_initially(pool: PgPool) {
    let service = service(&pool, 3);
    let creator = account(&pool).await;
    let req = CreateHazardRequest {
        category: HazardCategory::Construction,
        description: Some("Street works".into()),
        location: GeoJsonPoint::new(-70.65, -33.45),
    };

    let hazard = service
        .create_hazard(&creator, &req, ip("192.168.1.10"))
        .await
        .unwrap();

    assert_eq!(hazard.status, HazardStatus::Unconfirmed);
    assert_eq!(hazard.upvotes, 1);
    assert_eq!(hazard.description.as_deref(), Some("Street works"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_create_blocking_with_threshold_one_is_confirmed(pool: PgPool) {
    let service = service(&pool, 1);
    let creator = account(&pool).await;

    let hazard = service
        .create_hazard(&creator, &blocking_report(), ip("192.168.1.10"))
        .await
        .unwrap();

    assert_eq!(hazard.status, HazardStatus::Confirmed);
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_vote_blocking_hazard_confirmation_threshold_lifecycle(pool: PgPool) {
    let service = service(&pool, 3);
    let hazard = service
        .create_hazard(
            &account(&pool).await,
            &blocking_report(),
            ip("192.168.1.10"),
        )
        .await
        .unwrap();
    assert_eq!(hazard.status, HazardStatus::Unconfirmed);

    // Net 2 < 3.
    let voter_2 = account(&pool).await;
    let updated = service
        .vote_hazard(hazard.id, &voter_2, Vote::Up, ip("192.168.2.10"))
        .await
        .unwrap();
    assert_eq!((updated.upvotes, updated.downvotes), (2, 0));
    assert_eq!(updated.status, HazardStatus::Unconfirmed);

    // Net 3 >= 3.
    let updated = service
        .vote_hazard(
            hazard.id,
            &account(&pool).await,
            Vote::Up,
            ip("192.168.3.10"),
        )
        .await
        .unwrap();
    assert_eq!((updated.upvotes, updated.downvotes), (3, 0));
    assert_eq!(updated.status, HazardStatus::Confirmed);

    // A downvote brings the balance to 2: unconfirmed again.
    let voter_4 = account(&pool).await;
    let updated = service
        .vote_hazard(hazard.id, &voter_4, Vote::Down, ip("192.168.4.10"))
        .await
        .unwrap();
    assert_eq!((updated.upvotes, updated.downvotes), (3, 1));
    assert_eq!(updated.status, HazardStatus::Unconfirmed);

    // Changing that vote to Up makes it 4: confirmed.
    let updated = service
        .vote_hazard(hazard.id, &voter_4, Vote::Up, ip("192.168.4.10"))
        .await
        .unwrap();
    assert_eq!((updated.upvotes, updated.downvotes), (4, 0));
    assert_eq!(updated.status, HazardStatus::Confirmed);
    assert_consistent(&pool, hazard.id).await;
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_sybil_multiple_accounts_same_subnet_cannot_confirm_hazard(pool: PgPool) {
    let service = service(&pool, 3);
    let hazard = service
        .create_hazard(
            &account(&pool).await,
            &blocking_report(),
            ip("192.168.1.10"),
        )
        .await
        .unwrap();

    // Two more accounts from the creator's own /24 add nothing.
    for host in ["192.168.1.50", "192.168.1.99"] {
        let updated = service
            .vote_hazard(hazard.id, &account(&pool).await, Vote::Up, ip(host))
            .await
            .unwrap();
        assert_eq!(updated.upvotes, 1, "{host}");
        assert_eq!(updated.status, HazardStatus::Unconfirmed);
    }

    // Independent networks do.
    let v4 = service
        .vote_hazard(hazard.id, &account(&pool).await, Vote::Up, ip("10.0.1.10"))
        .await
        .unwrap();
    assert_eq!(v4.upvotes, 2);
    assert_eq!(v4.status, HazardStatus::Unconfirmed);

    let v5 = service
        .vote_hazard(hazard.id, &account(&pool).await, Vote::Up, ip("10.0.2.10"))
        .await
        .unwrap();
    assert_eq!(v5.upvotes, 3);
    assert_eq!(v5.status, HazardStatus::Confirmed);
    assert_consistent(&pool, hazard.id).await;
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_single_account_multiple_subnets_cannot_confirm_hazard(pool: PgPool) {
    let service = service(&pool, 3);
    let creator = account(&pool).await;
    let hazard = service
        .create_hazard(&creator, &blocking_report(), ip("192.168.1.10"))
        .await
        .unwrap();

    // The same account rotating networks never adds a second counted vote.
    for subnet in 2..=3 {
        let v = service
            .vote_hazard(
                hazard.id,
                &creator,
                Vote::Up,
                ip(&format!("192.168.{subnet}.10")),
            )
            .await
            .unwrap();
        assert_eq!((v.upvotes, v.downvotes), (1, 0), "subnet {subnet}");
        assert_eq!(v.status, HazardStatus::Unconfirmed);
    }

    // It can only change the vote it already had.
    let v3 = service
        .vote_hazard(hazard.id, &creator, Vote::Down, ip("192.168.4.10"))
        .await
        .unwrap();
    assert_eq!((v3.upvotes, v3.downvotes), (0, 1));
    let v4 = service
        .vote_hazard(hazard.id, &creator, Vote::Up, ip("192.168.5.10"))
        .await
        .unwrap();
    assert_eq!((v4.upvotes, v4.downvotes), (1, 0));

    // Legitimate accounts in other networks do confirm.
    let v_b = service
        .vote_hazard(
            hazard.id,
            &account(&pool).await,
            Vote::Up,
            ip("192.168.2.10"),
        )
        .await
        .unwrap();
    assert_eq!(v_b.upvotes, 2);
    let v_c = service
        .vote_hazard(
            hazard.id,
            &account(&pool).await,
            Vote::Up,
            ip("192.168.3.10"),
        )
        .await
        .unwrap();
    assert_eq!(v_c.upvotes, 3);
    assert_eq!(v_c.status, HazardStatus::Confirmed);
    assert_consistent(&pool, hazard.id).await;
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_ipv6_mobile_64_subnets_isolation(pool: PgPool) {
    let service = service(&pool, 3);
    let hazard = service
        .create_hazard(
            &account(&pool).await,
            &blocking_report(),
            ip("2001:db8:85a3:0::1"),
        )
        .await
        .unwrap();

    let same_64 = service
        .vote_hazard(
            hazard.id,
            &account(&pool).await,
            Vote::Up,
            ip("2001:db8:85a3:0:ffff::2"),
        )
        .await
        .unwrap();
    assert_eq!(same_64.upvotes, 1);

    let other_64 = service
        .vote_hazard(
            hazard.id,
            &account(&pool).await,
            Vote::Up,
            ip("2001:db8:85a3:1::1"),
        )
        .await
        .unwrap();
    assert_eq!(other_64.upvotes, 2);

    let third = service
        .vote_hazard(
            hazard.id,
            &account(&pool).await,
            Vote::Up,
            ip("2001:db8:85a3:2::1"),
        )
        .await
        .unwrap();
    assert_eq!(third.upvotes, 3);
    assert_eq!(third.status, HazardStatus::Confirmed);
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_vote_idempotence(pool: PgPool) {
    let service = service(&pool, 3);
    let hazard = service
        .create_hazard(
            &account(&pool).await,
            &report(HazardCategory::Flood),
            ip("192.168.1.10"),
        )
        .await
        .unwrap();
    let voter = account(&pool).await;

    let first = service
        .vote_hazard(hazard.id, &voter, Vote::Up, ip("192.168.2.10"))
        .await
        .unwrap();
    let again = service
        .vote_hazard(hazard.id, &voter, Vote::Up, ip("192.168.2.10"))
        .await
        .unwrap();

    assert_eq!(first.upvotes, 2);
    assert_eq!((again.upvotes, again.downvotes), (2, 0));
    let (ballots,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM hazard_votes WHERE hazard_id = $1")
            .bind(hazard.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(ballots, 2);
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_vote_not_found(pool: PgPool) {
    let service = service(&pool, 3);
    let voter = account(&pool).await;

    let err = service
        .vote_hazard(Uuid::new_v4(), &voter, Vote::Up, ip("192.168.1.10"))
        .await
        .unwrap_err();

    assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn voting_on_an_expired_report_is_not_found(pool: PgPool) {
    let service = service(&pool, 3);
    let creator = account(&pool).await;
    let expired = insert_hazard(
        &pool,
        creator.account_id,
        "road_closed",
        "unconfirmed",
        (-70.65, -33.45),
        -1,
    )
    .await;

    let err = service
        .vote_hazard(expired, &account(&pool).await, Vote::Up, ip("192.168.1.10"))
        .await
        .unwrap_err();

    assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_community_downvotes_retire_a_hazard(pool: PgPool) {
    let service = service(&pool, 3);
    let hazard = service
        .create_hazard(&account(&pool).await, &blocking_report(), ip("10.1.0.1"))
        .await
        .unwrap();

    // The creator's own vote is 1 up. Three independent downvotes leave net -2: still listed.
    for subnet in 2..=4 {
        let updated = service
            .vote_hazard(
                hazard.id,
                &account(&pool).await,
                Vote::Down,
                ip(&format!("10.{subnet}.0.1")),
            )
            .await
            .unwrap();
        assert_ne!(updated.status, HazardStatus::Resolved, "subnet {subnet}");
    }
    assert_eq!(
        service
            .list_active_hazards(&city_bbox())
            .await
            .unwrap()
            .len(),
        1
    );

    // The fourth makes net -3: retired, and gone from the listings.
    let retired = service
        .vote_hazard(hazard.id, &account(&pool).await, Vote::Down, ip("10.5.0.1"))
        .await
        .unwrap();
    assert_eq!(retired.status, HazardStatus::Resolved);
    assert!(
        service
            .list_active_hazards(&city_bbox())
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_resolved_hazard_no_longer_accepts_votes(pool: PgPool) {
    let service = service(&pool, 3);
    let hazard = service
        .create_hazard(&account(&pool).await, &blocking_report(), ip("10.1.0.1"))
        .await
        .unwrap();
    for subnet in 2..=5 {
        service
            .vote_hazard(
                hazard.id,
                &account(&pool).await,
                Vote::Down,
                ip(&format!("10.{subnet}.0.1")),
            )
            .await
            .unwrap();
    }

    let err = service
        .vote_hazard(hazard.id, &account(&pool).await, Vote::Up, ip("10.9.0.1"))
        .await
        .unwrap_err();

    assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_a_warning_can_be_retired_too(pool: PgPool) {
    let service = service(&pool, 2);
    let hazard = service
        .create_hazard(
            &account(&pool).await,
            &report(HazardCategory::Glass),
            ip("10.1.0.1"),
        )
        .await
        .unwrap();
    assert_eq!(hazard.status, HazardStatus::Confirmed);

    // 1 up, then 3 down: net -2 >= threshold 2.
    let mut last = hazard;
    for subnet in 2..=4 {
        last = service
            .vote_hazard(
                last.id,
                &account(&pool).await,
                Vote::Down,
                ip(&format!("10.{subnet}.0.1")),
            )
            .await
            .unwrap();
    }
    assert_eq!(last.status, HazardStatus::Resolved);
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_description_is_stored_trimmed(pool: PgPool) {
    let service = service(&pool, 3);
    let creator = account(&pool).await;
    let req = CreateHazardRequest {
        description: Some("  broken glass  ".into()),
        ..report(HazardCategory::Glass)
    };
    let hazard = service
        .create_hazard(&creator, &req, ip("10.1.0.1"))
        .await
        .unwrap();
    assert_eq!(hazard.description.as_deref(), Some("broken glass"));

    let blank = CreateHazardRequest {
        description: Some("   ".into()),
        ..req
    };
    let hazard = service
        .create_hazard(&creator, &blank, ip("10.1.0.1"))
        .await
        .unwrap();
    assert_eq!(hazard.description, None);
}

#[sqlx::test(migrations = "../../migrations")]
async fn ballots_keep_a_keyed_tag_of_the_network_and_never_the_address(pool: PgPool) {
    let service = service(&pool, 3);
    service
        .create_hazard(
            &account(&pool).await,
            &blocking_report(),
            ip("192.168.1.10"),
        )
        .await
        .unwrap();

    let tag: Vec<u8> = sqlx::query_scalar("SELECT voter_net FROM hazard_votes")
        .fetch_one(&pool)
        .await
        .unwrap();

    let expected = VoterNetKey::new(SECRET).tag(VoterNetwork::from_ip(ip("192.168.1.77")));
    assert_eq!(tag, expected, "same /24, same tag");
    assert!(!tag.windows(3).any(|w| w == [192, 168, 1]));
    // A table of only opaque bytes: no column could hold an address.
    let address_columns: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_name IN ('hazards', 'hazard_votes', 'accounts') \
           AND (data_type IN ('inet', 'cidr') OR column_name ~ '(^|_)(ip|addr|address)(_|$)')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(address_columns, 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn creating_and_voting_broadcast_the_stored_state(pool: PgPool) {
    let recorder = Arc::new(Recorder::default());
    let service = HazardService::new(pool.clone(), 3, 24, SECRET, recorder.clone());
    let hazard = service
        .create_hazard(&account(&pool).await, &blocking_report(), ip("10.1.0.1"))
        .await
        .unwrap();

    // A rejected vote is not announced.
    let _ = service
        .vote_hazard(
            Uuid::new_v4(),
            &account(&pool).await,
            Vote::Up,
            ip("10.2.0.1"),
        )
        .await
        .unwrap_err();
    service
        .vote_hazard(hazard.id, &account(&pool).await, Vote::Up, ip("10.2.0.1"))
        .await
        .unwrap();

    let seen = recorder.0.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].upvotes, 1);
    assert_eq!(seen[1].upvotes, 2);
    assert_eq!(seen[1].id, hazard.id);
}

// ------------------------------------------------------------------ concurrency

#[sqlx::test(migrations = "../../migrations")]
async fn twenty_parallel_voters_from_different_networks_all_count(pool: PgPool) {
    let service = service(&pool, 50);
    let hazard = service
        .create_hazard(
            &account(&pool).await,
            &blocking_report(),
            ip("192.168.1.10"),
        )
        .await
        .unwrap();

    let mut tasks = tokio::task::JoinSet::new();
    for i in 1..=20 {
        let service = service.clone();
        let voter = account(&pool).await;
        tasks.spawn(async move {
            service
                .vote_hazard(hazard.id, &voter, Vote::Up, ip(&format!("10.{i}.0.1")))
                .await
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap().unwrap();
    }

    let stored = assert_consistent(&pool, hazard.id).await;
    assert_eq!(stored.upvotes, 21);
}

#[sqlx::test(migrations = "../../migrations")]
async fn twenty_parallel_voters_from_one_network_count_once(pool: PgPool) {
    let service = service(&pool, 50);
    let hazard = service
        .create_hazard(
            &account(&pool).await,
            &blocking_report(),
            ip("192.168.1.10"),
        )
        .await
        .unwrap();

    let mut tasks = tokio::task::JoinSet::new();
    for host in 1..=20 {
        let service = service.clone();
        let voter = account(&pool).await;
        tasks.spawn(async move {
            service
                .vote_hazard(hazard.id, &voter, Vote::Up, ip(&format!("10.9.0.{host}")))
                .await
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap().unwrap();
    }

    // The creator's vote, plus exactly one of the twenty.
    let stored = assert_consistent(&pool, hazard.id).await;
    assert_eq!(stored.upvotes, 2);
    let (ballots,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM hazard_votes WHERE hazard_id = $1")
            .bind(hazard.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(ballots, 21, "every account still has its ballot");
}

#[sqlx::test(migrations = "../../migrations")]
async fn one_account_flipping_its_vote_in_parallel_keeps_one_consistent_ballot(pool: PgPool) {
    let service = service(&pool, 50);
    let hazard = service
        .create_hazard(
            &account(&pool).await,
            &blocking_report(),
            ip("192.168.1.10"),
        )
        .await
        .unwrap();
    let voter = account(&pool).await;

    let mut tasks = tokio::task::JoinSet::new();
    for i in 0..20 {
        let service = service.clone();
        let vote = if i % 2 == 0 { Vote::Up } else { Vote::Down };
        tasks.spawn(async move {
            service
                .vote_hazard(hazard.id, &voter, vote, ip("10.3.0.1"))
                .await
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap().unwrap();
    }

    assert_consistent(&pool, hazard.id).await;
    let (ballots,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM hazard_votes WHERE hazard_id = $1 AND account_id = $2",
    )
    .bind(hazard.id)
    .bind(voter.account_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ballots, 1);
}

// ------------------------------------------------------------------ listing

#[sqlx::test(migrations = "../../migrations")]
async fn listing_is_capped_and_newest_first(pool: PgPool) {
    let service = service(&pool, 3);
    let creator = account(&pool).await;
    sqlx::query(
        "INSERT INTO hazards (id, creator_account_id, category, hazard_type, status, upvotes, \
                              downvotes, geom, created_at, expires_at) \
         SELECT gen_random_uuid(), $1, 'pothole', 'warning', 'confirmed', 1, 0, \
                ST_SetSRID(ST_MakePoint(-70.65, -33.45), 4326), \
                now() - make_interval(secs => g::float8), now() + interval '1 day' \
         FROM generate_series(1, $2::int) g",
    )
    .bind(creator.account_id)
    .bind((MAX_LIST_RESULTS + 100) as i32)
    .execute(&pool)
    .await
    .unwrap();

    let listed = service.list_active_hazards(&city_bbox()).await.unwrap();

    assert_eq!(listed.len(), MAX_LIST_RESULTS);
    assert!(
        listed
            .windows(2)
            .all(|w| w[0].created_at >= w[1].created_at)
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn listing_returns_only_active_reports_inside_the_box(pool: PgPool) {
    let service = service(&pool, 3);
    let creator = account(&pool).await.account_id;
    let inside = insert_hazard(&pool, creator, "glass", "confirmed", (-70.65, -33.45), 5).await;
    insert_hazard(&pool, creator, "glass", "confirmed", (-70.65, -33.45), -1).await; // expired
    insert_hazard(&pool, creator, "flood", "resolved", (-70.65, -33.45), 5).await; // retired
    insert_hazard(&pool, creator, "glass", "confirmed", (-71.50, -33.45), 5).await; // elsewhere

    let listed = service.list_active_hazards(&city_bbox()).await.unwrap();

    assert_eq!(
        listed.iter().map(|h| h.id).collect::<Vec<_>>(),
        vec![inside]
    );
}

// ------------------------------------------------------------------ corridor

fn corridor() -> GeoJsonLineString {
    // About 1.9 km along the parallel at -33.45.
    GeoJsonLineString {
        geom_type: "LineString".into(),
        coordinates: vec![[-70.66, -33.45], [-70.64, -33.45]],
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn only_confirmed_live_blocking_reports_near_the_route_close_it(pool: PgPool) {
    let service = service(&pool, 3);
    let creator = account(&pool).await.account_id;
    let near = (-70.65, -33.4501); // ~11 m from the line
    insert_hazard(&pool, creator, "road_closed", "confirmed", near, 5).await;
    insert_hazard(&pool, creator, "road_closed", "unconfirmed", near, 5).await;
    insert_hazard(&pool, creator, "road_closed", "resolved", near, 5).await;
    insert_hazard(&pool, creator, "road_closed", "confirmed", near, -1).await; // expired
    insert_hazard(
        &pool,
        creator,
        "road_closed",
        "confirmed",
        (-70.65, -33.46),
        5,
    )
    .await; // ~1.1 km away
    insert_hazard(&pool, creator, "glass", "confirmed", near, 5).await; // a warning never blocks

    let polygons = service
        .find_blocking_polygons_along_corridor(&corridor(), 50.0)
        .await
        .unwrap();

    assert_eq!(polygons.len(), 1);
    let ring = &polygons[0].coordinates[0];
    assert_eq!(polygons[0].geom_type, "Polygon");
    assert_eq!(ring.first(), ring.last(), "the ring is closed");
    let (min_lon, max_lon) = ring.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
        (lo.min(p[0]), hi.max(p[0]))
    });
    let (min_lat, max_lat) = ring.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
        (lo.min(p[1]), hi.max(p[1]))
    });
    // It encloses the report and stays within a few hundred metres of it.
    assert!(min_lon < near.0 && near.0 < max_lon);
    assert!(min_lat < near.1 && near.1 < max_lat);
    assert!(max_lon - min_lon < 0.002 && max_lat - min_lat < 0.002);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_confirmed_closure_reaches_routing_once_the_votes_get_it_there(pool: PgPool) {
    let service = service(&pool, 3);
    let hazard = service
        .create_hazard(
            &account(&pool).await,
            &CreateHazardRequest {
                location: GeoJsonPoint::new(-70.65, -33.4501),
                ..blocking_report()
            },
            ip("10.1.0.1"),
        )
        .await
        .unwrap();
    let blocked = || async {
        service
            .find_blocking_polygons_along_corridor(&corridor(), 50.0)
            .await
            .unwrap()
            .len()
    };
    assert_eq!(blocked().await, 0, "an unconfirmed report closes nothing");

    for subnet in 2..=3 {
        service
            .vote_hazard(
                hazard.id,
                &account(&pool).await,
                Vote::Up,
                ip(&format!("10.{subnet}.0.1")),
            )
            .await
            .unwrap();
    }

    assert_eq!(blocked().await, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_corridor_listing_has_every_active_kind_near_the_route(pool: PgPool) {
    let service = service(&pool, 3);
    let creator = account(&pool).await.account_id;
    let near = (-70.65, -33.4501);
    let confirmed = insert_hazard(&pool, creator, "road_closed", "confirmed", near, 5).await;
    let unconfirmed = insert_hazard(&pool, creator, "flood", "unconfirmed", near, 5).await;
    let warning = insert_hazard(&pool, creator, "glass", "confirmed", near, 5).await;
    insert_hazard(&pool, creator, "glass", "resolved", near, 5).await;
    insert_hazard(&pool, creator, "glass", "confirmed", near, -1).await;
    insert_hazard(&pool, creator, "glass", "confirmed", (-70.65, -33.46), 5).await;

    let listed = service
        .list_hazards_near_corridor(&corridor(), 50.0)
        .await
        .unwrap();

    let ids: HashSet<Uuid> = listed.iter().map(|h| h.id).collect();
    assert_eq!(ids, HashSet::from([confirmed, unconfirmed, warning]));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_corridor_with_too_many_closures_is_an_error_not_a_silent_truncation(pool: PgPool) {
    let service = service(&pool, 3);
    let creator = account(&pool).await;
    sqlx::query(
        "INSERT INTO hazards (id, creator_account_id, category, hazard_type, status, upvotes, \
                              downvotes, geom, created_at, expires_at) \
         SELECT gen_random_uuid(), $1, 'road_closed', 'blocking', 'confirmed', 3, 0, \
                ST_SetSRID(ST_MakePoint(-70.65, -33.4501), 4326), \
                now() - interval '1 hour', now() + interval '1 day' \
         FROM generate_series(1, $2::int)",
    )
    .bind(creator.account_id)
    .bind((MAX_LIST_RESULTS + 1) as i32)
    .execute(&pool)
    .await
    .unwrap();

    let err = service
        .find_blocking_polygons_along_corridor(&corridor(), 50.0)
        .await
        .unwrap_err();

    assert!(matches!(err, AppError::Unavailable(_)), "{err:?}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_bad_corridor_is_a_validation_error_and_never_reaches_sql(pool: PgPool) {
    let service = service(&pool, 3);
    let bad = GeoJsonLineString {
        geom_type: "LineString".into(),
        coordinates: vec![[-70.65, -33.45]],
    };

    let err = service
        .find_blocking_polygons_along_corridor(&bad, 50.0)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)), "{err:?}");
    let err = service
        .list_hazards_near_corridor(&corridor(), f64::NAN)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)), "{err:?}");
}

// ------------------------------------------------------------------ purge and persistence

#[sqlx::test(migrations = "../../migrations")]
async fn test_purge_expired_hazards(pool: PgPool) {
    let service = service(&pool, 3);
    let creator = account(&pool).await;
    let live = service
        .create_hazard(&creator, &report(HazardCategory::Glass), ip("10.1.0.1"))
        .await
        .unwrap();
    // More than one purge batch of expired reports, one of them with a ballot.
    sqlx::query(
        "INSERT INTO hazards (id, creator_account_id, category, hazard_type, status, upvotes, \
                              downvotes, geom, created_at, expires_at) \
         SELECT gen_random_uuid(), $1, 'glass', 'warning', 'confirmed', 1, 0, \
                ST_SetSRID(ST_MakePoint(-70.65, -33.45), 4326), \
                now() - interval '2 days', now() - interval '1 day' \
         FROM generate_series(1, $2::int)",
    )
    .bind(creator.account_id)
    .bind((PURGE_BATCH * 2 + 500) as i32)
    .execute(&pool)
    .await
    .unwrap();
    let expired_with_ballot = insert_hazard(
        &pool,
        creator.account_id,
        "glass",
        "confirmed",
        (-70.65, -33.45),
        -1,
    )
    .await;
    sqlx::query(
        "INSERT INTO hazard_votes (hazard_id, account_id, vote_type, counts, voter_net) \
         VALUES ($1, $2, 1, TRUE, $3)",
    )
    .bind(expired_with_ballot)
    .bind(creator.account_id)
    .bind(&[9u8; 16][..])
    .execute(&pool)
    .await
    .unwrap();

    let purged = service.purge_expired().await.unwrap();

    assert_eq!(purged, (PURGE_BATCH * 2 + 500) as u64 + 1);
    let (left,): (i64,) = sqlx::query_as("SELECT count(*) FROM hazards")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 1);
    let (orphans,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM hazard_votes WHERE hazard_id = $1")
            .bind(expired_with_ballot)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(orphans, 0, "ballots go with their report");
    let listed = service.list_active_hazards(&city_bbox()).await.unwrap();
    assert_eq!(
        listed.iter().map(|h| h.id).collect::<Vec<_>>(),
        vec![live.id]
    );
    assert_eq!(service.purge_expired().await.unwrap(), 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn reports_and_votes_survive_a_restart(pool: PgPool) {
    let hazard = {
        let before = service(&pool, 3);
        let hazard = before
            .create_hazard(&account(&pool).await, &blocking_report(), ip("10.1.0.1"))
            .await
            .unwrap();
        before
            .vote_hazard(hazard.id, &account(&pool).await, Vote::Up, ip("10.2.0.1"))
            .await
            .unwrap();
        hazard
    };

    // A new process: a new pool and a new service over the same database.
    let reconnected = PgPoolOptions::new()
        .max_connections(2)
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap();
    let after = service(&reconnected, 3);
    let listed = after.list_active_hazards(&city_bbox()).await.unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, hazard.id);
    assert_eq!(listed[0].upvotes, 2);
    // And the one-vote-per-network memory survived too: 10.2.0.x already voted.
    let again = after
        .vote_hazard(hazard.id, &account(&pool).await, Vote::Up, ip("10.2.0.99"))
        .await
        .unwrap();
    assert_eq!(again.upvotes, 2);
}

// ------------------------------------------------------------------ query plans

/// The statements the service runs must use the spatial indexes, or a busy map becomes a table
/// scan. The planner is told not to scan sequentially, over a table big and spread out enough
/// for the statistics to matter, so a plan without the index means it cannot be used at all.
#[sqlx::test(migrations = "../../migrations")]
async fn spatial_queries_use_the_gist_indexes(pool: PgPool) {
    let creator = account(&pool).await;
    sqlx::query(
        "INSERT INTO hazards (id, creator_account_id, category, hazard_type, status, upvotes, \
                              downvotes, geom, created_at, expires_at) \
         SELECT gen_random_uuid(), $1, 'glass', 'warning', 'confirmed', 1, 0, \
                ST_SetSRID(ST_MakePoint(-70.9 + random() * 0.5, -33.7 + random() * 0.5), 4326), \
                now() - interval '1 hour', now() + interval '1 day' \
         FROM generate_series(1, 5000)",
    )
    .bind(creator.account_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("ANALYZE hazards").execute(&pool).await.unwrap();

    let mut conn = pool.acquire().await.unwrap();
    sqlx::query("SET enable_seqscan = off")
        .execute(&mut *conn)
        .await
        .unwrap();

    let list_plan: Vec<String> = sqlx::query_scalar(&format!("EXPLAIN {LIST_SQL}"))
        .bind(-70.7f64)
        .bind(-33.5f64)
        .bind(-70.6f64)
        .bind(-33.4f64)
        .bind(MAX_LIST_RESULTS as i64)
        .fetch_all(&mut *conn)
        .await
        .unwrap();
    assert!(
        list_plan.join("\n").contains("idx_hazards_geom"),
        "{}",
        list_plan.join("\n")
    );

    let line = serde_json::to_string(&corridor()).unwrap();
    let corridor_plan: Vec<String> = sqlx::query_scalar(&format!("EXPLAIN {CORRIDOR_LIST_SQL}"))
        .bind(&line)
        .bind(50.0f64)
        .bind(MAX_LIST_RESULTS as i64)
        .fetch_all(&mut *conn)
        .await
        .unwrap();
    assert!(
        corridor_plan.join("\n").contains("idx_hazards_geog"),
        "{}",
        corridor_plan.join("\n")
    );

    let blocking_plan: Vec<String> =
        sqlx::query_scalar(&format!("EXPLAIN {CORRIDOR_BLOCKING_SQL}"))
            .bind(&line)
            .bind(50.0f64)
            .bind(BLOCKING_EXCLUSION_RADIUS_METERS)
            .bind(MAX_LIST_RESULTS as i64 + 1)
            .fetch_all(&mut *conn)
            .await
            .unwrap();
    let blocking_plan = blocking_plan.join("\n");
    assert!(
        blocking_plan.contains("idx_hazards_geog")
            || blocking_plan.contains("idx_hazards_type_status"),
        "{blocking_plan}"
    );
}
