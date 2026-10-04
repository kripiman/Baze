// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Closure avoidance with the real hazard store: the routing client, an in-process Valhalla and
//! PostGIS, through the HTTP layer. Enabled with `--features db-tests` like the other database tests.
#![cfg(feature = "db-tests")]

mod common;

use axum::Router;
use axum::http::StatusCode;
use baze_app::disabled::EnginesDisabled;
use chrono::Duration;
use common::valhalla::{FakeValhalla, detour, excluded_count, route_json, straight};
use common::{json_request, send, test_accounts, test_app_with_services, test_config};
use hazards::{HazardPolicy, HazardService};
use realtime::RealtimeService;
use routing::ValhallaRoutingService;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: [f64; 2] = [-70.70, -33.45];
const DESTINATION: [f64; 2] = [-70.60, -33.45];
const MIDPOINT: [f64; 2] = [-70.65, -33.45];
const SECRET: &[u8] = b"routing-db-test-secret-0123456789abcdef";

fn policy() -> HazardPolicy {
    HazardPolicy {
        confirmation_threshold: 3,
        default_ttl_hours: 24,
        min_account_age: Duration::zero(),
        reports_per_day: 1000,
        votes_per_day: 5000,
    }
}

fn app(pool: &PgPool, valhalla: &FakeValhalla) -> Router {
    let realtime = RealtimeService::new(16);
    let hazards = Arc::new(HazardService::new(
        pool.clone(),
        policy(),
        SECRET,
        Arc::new(realtime.clone()),
    ));
    let routing =
        Arc::new(ValhallaRoutingService::new(valhalla.url.clone(), hazards.clone()).unwrap());
    let config = test_config();
    test_app_with_services(
        config.clone(),
        test_accounts(&config),
        hazards,
        routing,
        Arc::new(EnginesDisabled),
        realtime,
    )
}

fn request_body() -> Value {
    json!({
        "origin": {"type": "Point", "coordinates": ORIGIN},
        "destination": {"type": "Point", "coordinates": DESTINATION},
    })
}

async fn route(app: &Router) -> (StatusCode, Value) {
    send(
        app,
        json_request("POST", "/api/v1/routing/route", None, &request_body()),
    )
    .await
}

async fn creator(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("INSERT INTO accounts DEFAULT VALUES RETURNING id")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A stored report. `category` decides the type; `status` is set as the votes would have.
async fn hazard(pool: &PgPool, category: &str, status: &str, lon: f64, lat: f64) -> Uuid {
    let hazard_type = if matches!(category, "road_closed" | "construction" | "flood") {
        "blocking"
    } else {
        "warning"
    };
    sqlx::query_scalar(
        "INSERT INTO hazards (creator_account_id, category, hazard_type, status, geom, expires_at) \
         VALUES ($1, $2, $3, $4, ST_SetSRID(ST_MakePoint($5, $6), 4326), now() + interval '24 hours') \
         RETURNING id",
    )
    .bind(creator(pool).await)
    .bind(category)
    .bind(hazard_type)
    .bind(status)
    .bind(lon)
    .bind(lat)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Valhalla that takes the direct road unless it is asked to avoid something, then bows north.
async fn valhalla_with_a_detour() -> FakeValhalla {
    let (direct, bypass) = (
        straight(ORIGIN, DESTINATION, 200),
        detour(ORIGIN, DESTINATION, 0.01),
    );
    FakeValhalla::start(move |request| {
        route_json(if excluded_count(request) == 0 {
            &direct
        } else {
            &bypass
        })
    })
    .await
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn a_confirmed_closure_is_avoided_and_an_unconfirmed_one_is_not(pool: PgPool) {
    hazard(&pool, "road_closed", "confirmed", MIDPOINT[0], MIDPOINT[1]).await;
    // Reported but not confirmed by the community yet: it must not move anybody's route.
    hazard(&pool, "road_closed", "unconfirmed", -70.68, -33.45).await;
    let valhalla = valhalla_with_a_detour().await;

    let (status, body) = route(&app(&pool, &valhalla)).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let requests = valhalla.requests();
    assert_eq!(requests.len(), 2, "the base route and one with exclusions");
    assert_eq!(
        excluded_count(&requests[1]),
        1,
        "only the confirmed closure is excluded"
    );
    // The excluded area is the closure's neighbourhood: a closed ring of ~30 m around the report.
    let ring = requests[1]["exclude_polygons"][0].as_array().unwrap();
    assert_eq!(ring.first(), ring.last());
    assert!(ring.len() >= 8, "{ring:?}");
    for vertex in ring {
        let (lon, lat) = (vertex[0].as_f64().unwrap(), vertex[1].as_f64().unwrap());
        assert!(
            (lon - MIDPOINT[0]).abs() < 0.0006 && (lat - MIDPOINT[1]).abs() < 0.0006,
            "{vertex}"
        );
    }
    // The route served is the detour, not the base route.
    let served_apex = body["geometry"]["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p[1].as_f64().unwrap())
        .fold(f64::MIN, f64::max);
    assert!(served_apex > ORIGIN[1] + 0.009, "{served_apex}");
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn a_route_with_no_closures_on_it_is_not_asked_twice(pool: PgPool) {
    hazard(&pool, "road_closed", "confirmed", -70.65, -33.40).await; // 5 km north of the road
    hazard(&pool, "flood", "resolved", MIDPOINT[0], MIDPOINT[1]).await; // retired by the community
    let valhalla = valhalla_with_a_detour().await;

    let (status, body) = route(&app(&pool, &valhalla)).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(valhalla.requests().len(), 1);
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn an_expired_closure_is_ignored(pool: PgPool) {
    let id = hazard(&pool, "road_closed", "confirmed", MIDPOINT[0], MIDPOINT[1]).await;
    sqlx::query(
        "UPDATE hazards SET created_at = now() - interval '2 hours', expires_at = now() - interval '1 hour' WHERE id = $1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    let valhalla = valhalla_with_a_detour().await;

    let (status, _) = route(&app(&pool, &valhalla)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(valhalla.requests().len(), 1);
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn alerts_within_fifty_meters_of_the_served_route_travel_with_it(pool: PgPool) {
    hazard(&pool, "road_closed", "confirmed", MIDPOINT[0], MIDPOINT[1]).await;
    // Near the detour's far point: it is on the route that is served.
    let near_the_detour = hazard(
        &pool,
        "pothole",
        "confirmed",
        MIDPOINT[0],
        MIDPOINT[1] + 0.01,
    )
    .await;
    // Near the road the closure made the route leave: not on the route that is served.
    hazard(&pool, "glass", "confirmed", -70.62, -33.4501).await;
    // Beside the detour, but 300 m away.
    hazard(
        &pool,
        "debris",
        "confirmed",
        MIDPOINT[0],
        MIDPOINT[1] + 0.0127,
    )
    .await;
    let valhalla = valhalla_with_a_detour().await;

    let (status, body) = route(&app(&pool, &valhalla)).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let alerts: Vec<&str> = body["nearby_hazards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].as_str().unwrap())
        .collect();
    assert_eq!(alerts, vec![near_the_detour.to_string()], "{body}");
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn a_closure_at_the_destination_means_no_route_never_a_route_through_it(pool: PgPool) {
    hazard(
        &pool,
        "road_closed",
        "confirmed",
        DESTINATION[0],
        DESTINATION[1],
    )
    .await;
    // An engine that cannot avoid it (the destination is inside the closed area) answers the same road.
    let direct = straight(ORIGIN, DESTINATION, 200);
    let valhalla = FakeValhalla::start(move |_| route_json(&direct)).await;

    let (status, body) = route(&app(&pool, &valhalla)).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(body.get("geometry").is_none(), "{body}");
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn more_closures_than_the_store_will_list_make_the_route_unavailable(pool: PgPool) {
    let account = creator(&pool).await;
    // 501 confirmed closures, 9 m apart, all on the road: one more than the store returns at most.
    sqlx::query(
        "INSERT INTO hazards (creator_account_id, category, hazard_type, status, geom, expires_at) \
         SELECT $1, 'road_closed', 'blocking', 'confirmed', \
                ST_SetSRID(ST_MakePoint(-70.699 + i * 0.0001, -33.45), 4326), now() + interval '24 hours' \
         FROM generate_series(0, 500) AS i",
    )
    .bind(account)
    .execute(&pool)
    .await
    .unwrap();
    let valhalla = valhalla_with_a_detour().await;

    let response = app(&pool, &valhalla)
        .oneshot(json_request(
            "POST",
            "/api/v1/routing/route",
            None,
            &request_body(),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        valhalla.requests().len(),
        1,
        "no exclusion is attempted with that many closures"
    );
}

#[sqlx::test(migrator = "baze_app::db::MIGRATOR")]
async fn a_route_of_twenty_thousand_points_is_checked_against_the_real_store(pool: PgPool) {
    hazard(&pool, "road_closed", "confirmed", MIDPOINT[0], MIDPOINT[1]).await;
    // Twice the points the store accepts in a corridor: the router must simplify before asking.
    let long: Vec<[f64; 2]> = (0..20_000)
        .map(|i| {
            let t = f64::from(i) / 19_999.0;
            [
                ORIGIN[0] + (DESTINATION[0] - ORIGIN[0]) * t,
                ORIGIN[1] + (t * 40.0).sin() * 0.00005,
            ]
        })
        .collect();
    let bypass = detour(ORIGIN, DESTINATION, 0.01);
    let valhalla = FakeValhalla::start(move |request| {
        route_json(if excluded_count(request) == 0 {
            &long
        } else {
            &bypass
        })
    })
    .await;

    let (status, body) = route(&app(&pool, &valhalla)).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        valhalla.requests().len(),
        2,
        "the closure was found on the long route"
    );
}
