// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The client and the closure-avoidance loop against an in-process Valhalla and a store that knows
//! where the closures are.

use crate::test_support::{
    Closure, FakeCorridor, FakeValhalla, detour, engine_error, excluded_count, ok_route,
    pothole_at, route_json, straight,
};
use crate::valhalla::ValhallaClient;
use crate::{MAX_EXCLUDE_POLYGONS, ValhallaRoutingService};
use axum::http::StatusCode;
use serde_json::{Value, json};
use shared::{AppError, GeoJsonPoint, RouteRequest, RoutingProvider};
use std::sync::Arc;
use std::time::Duration;

const ORIGIN: [f64; 2] = [-70.70, -33.45];
const DESTINATION: [f64; 2] = [-70.60, -33.45];
/// Where the base route passes: halfway along the straight run.
const MIDPOINT: [f64; 2] = [-70.65, -33.45];

fn request() -> RouteRequest {
    RouteRequest {
        origin: GeoJsonPoint::new(ORIGIN[0], ORIGIN[1]),
        destination: GeoJsonPoint::new(DESTINATION[0], DESTINATION[1]),
    }
}

fn service(fake: &FakeValhalla, corridor: &Arc<FakeCorridor>) -> ValhallaRoutingService {
    ValhallaRoutingService::with_engine(ValhallaClient::new(&fake.url).unwrap(), corridor.clone())
}

fn direct() -> Vec<[f64; 2]> {
    straight(ORIGIN, DESTINATION, 200)
}

/// An error message that ends up in the server log must not say where anybody was going.
fn assert_says_no_coordinates(error: &AppError) {
    let message = error.to_string();
    for needle in ["-70.", "-33.", "70.6", "33.4"] {
        assert!(!message.contains(needle), "{needle} leaked into: {message}");
    }
}

fn assert_close(actual: &[[f64; 2]], expected: &[[f64; 2]]) {
    assert_eq!(actual.len(), expected.len());
    for (a, e) in actual.iter().zip(expected) {
        assert!(
            (a[0] - e[0]).abs() < 1e-6 && (a[1] - e[1]).abs() < 1e-6,
            "{a:?} {e:?}"
        );
    }
}

#[tokio::test]
async fn a_route_without_closures_costs_one_engine_call() {
    let route = direct();
    let expected = route.clone();
    let fake = FakeValhalla::start(move |_| ok_route(&route)).await;
    let corridor = Arc::new(FakeCorridor::default());

    let answer = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap();

    let requests = fake.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].get("exclude_polygons").is_none());
    assert_eq!(requests[0]["costing"], "bicycle");
    assert_eq!(requests[0]["locations"][0]["lon"], ORIGIN[0]);
    assert_eq!(requests[0]["locations"][1]["lat"], DESTINATION[1]);
    assert_close(&answer.geometry.coordinates, &expected);
    assert_eq!(answer.geometry.geom_type, "LineString");
    assert_eq!(corridor.blocking_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_engines_answer_is_mapped_to_the_api_units() {
    let points = direct();
    let mut answer = route_json(&points);
    answer["trip"]["legs"][0]["elevation"] = json!([100.0, 130.0, 120.0, 150.0]);
    answer["trip"]["legs"][0]["maneuvers"][0]["instruction"] =
        json!("Gire a la\n derecha\u{202e}  en Avenida\u{0}Providencia");
    let fake = FakeValhalla::start(move |_| (StatusCode::OK, answer.to_string())).await;
    let corridor = Arc::new(FakeCorridor::default());

    let route = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap();

    assert_eq!(route.distance_meters, 9500.0);
    assert_eq!(route.duration_seconds, 1900.0);
    assert_eq!(route.ascent_meters, 60.0);
    assert_eq!(route.descent_meters, 10.0);
    assert_eq!(route.maneuvers.len(), 2);
    assert_eq!(route.maneuvers[0].distance_meters, 9400.0);
    assert_eq!(route.maneuvers[0].time_seconds, 1800.0);
    assert_eq!(
        route.maneuvers[0].instruction,
        "Gire a la derecha en Avenida Providencia"
    );
    // A maneuver is placed where its shape index says.
    assert!((route.maneuvers[0].location.lon() - ORIGIN[0]).abs() < 1e-6);
    assert!((route.maneuvers[1].location.lon() - DESTINATION[0]).abs() < 1e-6);
}

#[tokio::test]
async fn a_route_without_elevation_reports_no_climb() {
    let points = direct();
    let fake = FakeValhalla::start(move |_| ok_route(&points)).await;
    let corridor = Arc::new(FakeCorridor::default());

    let route = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap();

    assert_eq!((route.ascent_meters, route.descent_meters), (0.0, 0.0));
}

#[tokio::test]
async fn a_confirmed_closure_on_the_route_makes_the_engine_avoid_it() {
    let (base, bypass) = (direct(), detour(ORIGIN, DESTINATION, 0.01));
    let expected = bypass.clone();
    let fake = FakeValhalla::start(move |body| {
        ok_route(if excluded_count(body) == 0 {
            &base
        } else {
            &bypass
        })
    })
    .await;
    let closure = Closure::at(MIDPOINT[0], MIDPOINT[1]);
    let ring = json!(closure.polygon.coordinates[0]);
    let corridor = Arc::new(FakeCorridor::with_closures(vec![closure]));

    let route = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap();

    let requests = fake.requests();
    assert_eq!(
        requests.len(),
        2,
        "one call for the base route and one with exclusions"
    );
    assert!(requests[0].get("exclude_polygons").is_none());
    assert_eq!(requests[1]["exclude_polygons"], json!([ring]));
    assert_close(&route.geometry.coordinates, &expected);

    // The alerts are those of the route that is served, not of the one that was discarded.
    let alerts = corridor.alert_calls.lock().unwrap();
    assert_eq!(alerts.len(), 1);
    let served_apex = alerts[0].1.iter().map(|p| p[1]).fold(f64::MIN, f64::max);
    assert!(served_apex > ORIGIN[1] + 0.009, "{served_apex}");
}

#[tokio::test]
async fn a_closure_off_the_route_is_never_sent_to_the_engine() {
    let route = direct();
    let fake = FakeValhalla::start(move |_| ok_route(&route)).await;
    let corridor = Arc::new(FakeCorridor::with_closures(vec![Closure::at(
        -70.65, -33.40,
    )]));

    service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap();

    assert_eq!(fake.requests().len(), 1);
}

#[tokio::test]
async fn the_detour_is_checked_too_and_avoids_what_it_would_cross() {
    // Each answer is one step further from the first: closing the first road makes the engine take a
    // second that is also closed, and so on.
    let routes = [
        direct(),
        detour(ORIGIN, DESTINATION, 0.01),
        detour(ORIGIN, DESTINATION, 0.02),
    ];
    let expected = routes[2].clone();
    let fake =
        FakeValhalla::start(move |body| ok_route(&routes[excluded_count(body).min(2)])).await;
    let corridor = Arc::new(FakeCorridor::with_closures(vec![
        Closure::at(MIDPOINT[0], MIDPOINT[1]),
        Closure::at(MIDPOINT[0], MIDPOINT[1] + 0.01),
    ]));

    let route = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap();

    let requests = fake.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(excluded_count(&requests[1]), 1);
    assert_eq!(
        excluded_count(&requests[2]),
        2,
        "the first closure stays excluded"
    );
    assert_close(&route.geometry.coordinates, &expected);
}

#[tokio::test]
async fn it_gives_up_after_three_reroutes_instead_of_serving_a_route_that_crosses_a_closure() {
    let routes: Vec<_> = (0..4)
        .map(|i| detour(ORIGIN, DESTINATION, 0.01 * f64::from(i)))
        .collect();
    let closures = (0..4)
        .map(|i| Closure::at(MIDPOINT[0], MIDPOINT[1] + 0.01 * f64::from(i)))
        .collect();
    let fake =
        FakeValhalla::start(move |body| ok_route(&routes[excluded_count(body).min(3)])).await;
    let corridor = Arc::new(FakeCorridor::with_closures(closures));

    let error = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap_err();

    assert!(matches!(error, AppError::Unavailable(_)), "{error}");
    assert_eq!(
        fake.requests().len(),
        4,
        "the base route plus three reroutes, no more"
    );
    assert_says_no_coordinates(&error);
}

#[tokio::test]
async fn more_closures_than_the_engine_can_exclude_is_refused_before_asking() {
    let route = direct();
    let fake = FakeValhalla::start(move |_| ok_route(&route)).await;
    // 51 closures along the base route, 150 m apart.
    let closures = (0..=MAX_EXCLUDE_POLYGONS)
        .map(|i| Closure::at(ORIGIN[0] + 0.0015 * (i as f64 + 1.0), ORIGIN[1]))
        .collect();
    let corridor = Arc::new(FakeCorridor::with_closures(closures));

    let error = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap_err();

    assert!(
        matches!(&error, AppError::Unavailable(m) if m.contains("Too many")),
        "{error}"
    );
    assert_eq!(
        fake.requests().len(),
        1,
        "an oversized exclusion is never sent"
    );
}

#[tokio::test]
async fn when_every_alternative_is_closed_the_answer_is_no_route_not_the_closed_one() {
    let route = direct();
    let fake = FakeValhalla::start(move |body| {
        if excluded_count(body) == 0 {
            ok_route(&route)
        } else {
            engine_error(442, "No path could be found for input")
        }
    })
    .await;
    let corridor = Arc::new(FakeCorridor::with_closures(vec![Closure::at(
        MIDPOINT[0],
        MIDPOINT[1],
    )]));

    let error = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap_err();

    assert!(
        matches!(&error, AppError::NotFound(m) if m.contains("avoids")),
        "{error}"
    );
    assert_says_no_coordinates(&error);
}

#[tokio::test]
async fn an_engine_that_ignores_the_exclusions_is_not_believed() {
    // The engine is told to avoid the closure and answers the very same route: that route must not be
    // served just because it was the engine's answer to the second question.
    let route = direct();
    let fake = FakeValhalla::start(move |_| ok_route(&route)).await;
    let corridor = Arc::new(FakeCorridor::with_closures(vec![Closure::at(
        MIDPOINT[0],
        MIDPOINT[1],
    )]));

    let error = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap_err();

    assert!(
        matches!(&error, AppError::NotFound(m) if m.contains("avoids")),
        "{error}"
    );
    assert_eq!(fake.requests().len(), 2);
}

#[tokio::test]
async fn an_unreachable_destination_is_not_found() {
    let fake = FakeValhalla::start(|_| engine_error(442, "No path could be found for input")).await;
    let corridor = Arc::new(FakeCorridor::default());

    let error = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap_err();

    assert!(
        matches!(&error, AppError::NotFound(m) if m.contains("between these points")),
        "{error}"
    );
    assert!(corridor.blocking_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_point_off_the_map_is_not_found() {
    let fake = FakeValhalla::start(|_| engine_error(171, "No suitable edges near location")).await;
    let corridor = Arc::new(FakeCorridor::default());

    let error = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap_err();

    assert!(matches!(error, AppError::NotFound(_)), "{error}");
}

#[tokio::test]
async fn a_route_the_engine_considers_too_long_is_a_validation_error() {
    let fake =
        FakeValhalla::start(|_| engine_error(154, "Path distance exceeds the max distance limit"))
            .await;
    let corridor = Arc::new(FakeCorridor::default());

    let error = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap_err();

    assert!(matches!(error, AppError::Validation(_)), "{error}");
}

#[tokio::test]
async fn a_refusal_of_the_exclusions_themselves_is_unavailable_not_a_gateway_error() {
    let route = direct();
    let fake = FakeValhalla::start(move |body| {
        if excluded_count(body) == 0 {
            ok_route(&route)
        } else {
            engine_error(167, "Exceeded max exclude polygons length")
        }
    })
    .await;
    let corridor = Arc::new(FakeCorridor::with_closures(vec![Closure::at(
        MIDPOINT[0],
        MIDPOINT[1],
    )]));

    let error = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap_err();

    assert!(matches!(error, AppError::Unavailable(_)), "{error}");
}

#[tokio::test]
async fn an_unexpected_refusal_without_exclusions_is_an_upstream_error() {
    let fake = FakeValhalla::start(|_| engine_error(125, "No costing method found")).await;
    let corridor = Arc::new(FakeCorridor::default());

    let error = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap_err();

    assert!(
        matches!(&error, AppError::Upstream(m) if m.contains("125")),
        "{error}"
    );
}

#[tokio::test]
async fn engine_failures_are_upstream_errors_that_say_nothing_about_the_trip() {
    let valid = route_json(&direct());
    let two_legs = {
        let mut answer = valid.clone();
        let leg = answer["trip"]["legs"][0].clone();
        answer["trip"]["legs"] = json!([leg.clone(), leg]);
        answer
    };
    let bad_shape = {
        let mut answer = valid.clone();
        answer["trip"]["legs"][0]["shape"] = json!("this is not a polyline!");
        answer
    };
    let stray_maneuver = {
        let mut answer = valid.clone();
        answer["trip"]["legs"][0]["maneuvers"][0]["begin_shape_index"] = json!(100_000);
        answer
    };
    let negative_length = {
        let mut answer = valid.clone();
        answer["trip"]["summary"]["length"] = json!(-1.0);
        answer
    };
    let wrong_type = json!({"trip": {"summary": {"length": "-33.45", "time": 1.0}, "legs": []}});
    let cases: Vec<(StatusCode, String)> = vec![
        (StatusCode::INTERNAL_SERVER_ERROR, "boom".into()),
        (StatusCode::BAD_GATEWAY, String::new()),
        (StatusCode::OK, "this is not json".into()),
        (StatusCode::OK, "{}".into()),
        (StatusCode::OK, wrong_type.to_string()),
        (StatusCode::OK, two_legs.to_string()),
        (StatusCode::OK, bad_shape.to_string()),
        (StatusCode::OK, stray_maneuver.to_string()),
        (StatusCode::OK, negative_length.to_string()),
        (StatusCode::BAD_REQUEST, "<html>proxy</html>".into()),
    ];
    for (status, body) in cases {
        let label = format!("{status} {body:.40}");
        let fake = FakeValhalla::start(move |_| (status, body.clone())).await;
        let corridor = Arc::new(FakeCorridor::default());

        let error = service(&fake, &corridor)
            .route_bicycle(&request())
            .await
            .unwrap_err();

        assert!(matches!(error, AppError::Upstream(_)), "{label}: {error}");
        assert_says_no_coordinates(&error);
        assert!(
            corridor.blocking_calls.lock().unwrap().is_empty(),
            "{label}: nothing is checked for a route that does not exist"
        );
    }
}

#[tokio::test]
async fn an_engine_that_is_down_is_an_upstream_error() {
    let url = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    let corridor = Arc::new(FakeCorridor::default());
    let service = ValhallaRoutingService::with_engine(ValhallaClient::new(&url).unwrap(), corridor);

    let error = service.route_bicycle(&request()).await.unwrap_err();

    assert!(
        matches!(&error, AppError::Upstream(m) if m.contains("unreachable")),
        "{error}"
    );
    assert_says_no_coordinates(&error);
}

#[tokio::test]
async fn an_engine_that_does_not_answer_in_time_is_an_upstream_error() {
    let route = direct();
    let fake =
        FakeValhalla::start_with_delay(move |_| ok_route(&route), Some(Duration::from_secs(2)))
            .await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let corridor = Arc::new(FakeCorridor::default());
    let service = ValhallaRoutingService::with_engine(
        ValhallaClient::with_client(client, &fake.url),
        corridor,
    );

    let error = service.route_bicycle(&request()).await.unwrap_err();

    assert!(
        matches!(&error, AppError::Upstream(m) if m.contains("in time")),
        "{error}"
    );
    assert_says_no_coordinates(&error);
}

#[tokio::test]
async fn an_oversized_answer_is_refused_not_buffered() {
    let route = direct();
    let fake = FakeValhalla::start(move |_| ok_route(&route)).await;
    let corridor = Arc::new(FakeCorridor::default());
    let service = ValhallaRoutingService::with_engine(
        ValhallaClient::new(&fake.url)
            .unwrap()
            .with_max_response_bytes(200),
        corridor,
    );

    let error = service.route_bicycle(&request()).await.unwrap_err();

    assert!(
        matches!(&error, AppError::Upstream(m) if m.contains("more than")),
        "{error}"
    );
}

#[tokio::test]
async fn a_failing_hazard_store_fails_the_route_instead_of_skipping_the_check() {
    let route = direct();
    let fake = FakeValhalla::start(move |_| ok_route(&route)).await;
    let corridor = Arc::new(FakeCorridor::failing(|| {
        AppError::Unavailable("Too many blocking reports along this route to avoid them all".into())
    }));

    let error = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap_err();

    assert!(matches!(error, AppError::Unavailable(_)), "{error}");
    assert_eq!(fake.requests().len(), 1);
}

#[tokio::test]
async fn nearby_reports_travel_with_the_route() {
    let route = direct();
    let fake = FakeValhalla::start(move |_| ok_route(&route)).await;
    let alert = pothole_at(-70.66, -33.4501);
    let corridor = Arc::new(FakeCorridor::default().with_alerts(vec![alert.clone()]));

    let answer = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap();

    assert_eq!(answer.nearby_hazards.len(), 1);
    assert_eq!(answer.nearby_hazards[0].id, alert.id);
    let alerts = corridor.alert_calls.lock().unwrap();
    assert!(
        (alerts[0].0.buffer_meters - 52.0).abs() < 1e-9,
        "{:?}",
        alerts[0].0
    );
}

#[tokio::test]
async fn a_long_route_is_simplified_before_it_is_checked_but_served_whole() {
    // 20 000 points: twice what the store accepts. The closure sits in the middle of a stretch that the
    // simplification reduces to a couple of points, and must still be found.
    let long: Vec<[f64; 2]> = (0..20_000)
        .map(|i| {
            let t = f64::from(i) / 19_999.0;
            [
                ORIGIN[0] + (DESTINATION[0] - ORIGIN[0]) * t,
                ORIGIN[1] + (t * 40.0).sin() * 0.00005,
            ]
        })
        .collect();
    let (served, bypass) = (long.clone(), detour(ORIGIN, DESTINATION, 0.02));
    let fake = FakeValhalla::start(move |body| {
        ok_route(if excluded_count(body) == 0 {
            &served
        } else {
            &bypass
        })
    })
    .await;
    let corridor = Arc::new(FakeCorridor::with_closures(vec![Closure::at(
        MIDPOINT[0],
        MIDPOINT[1],
    )]));

    let route = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap();

    assert_eq!(
        fake.requests().len(),
        2,
        "the closure was found on the simplified line"
    );
    let first_call = corridor.blocking_calls.lock().unwrap()[0];
    assert!(first_call.points < 10_000, "{first_call:?}");
    assert!(
        first_call.buffer_meters > 15.0,
        "the search is widened by the tolerance: {first_call:?}"
    );
    // The route that is served keeps its full resolution.
    assert_eq!(route.geometry.coordinates.len(), 201);

    // And a long route with nothing on it is served with all its points.
    let all = long.clone();
    let quiet = FakeValhalla::start(move |_| ok_route(&all)).await;
    let empty = Arc::new(FakeCorridor::default());
    let whole = service(&quiet, &empty)
        .route_bicycle(&request())
        .await
        .unwrap();
    assert_eq!(whole.geometry.coordinates.len(), 20_000);
    assert!(empty.blocking_calls.lock().unwrap()[0].points < 10_000);
}

#[tokio::test]
async fn a_route_too_intricate_to_check_is_refused_rather_than_unchecked() {
    // Alternating sides 110 m apart: no tolerance up to 16 m can remove a single point.
    let zigzag: Vec<[f64; 2]> = (0..10_100)
        .map(|i| {
            [
                ORIGIN[0] + f64::from(i) * 0.00001,
                ORIGIN[1] + if i % 2 == 0 { 0.0 } else { 0.001 },
            ]
        })
        .collect();
    let fake = FakeValhalla::start(move |_| ok_route(&zigzag)).await;
    let corridor = Arc::new(FakeCorridor::default());

    let error = service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap_err();

    assert!(
        matches!(&error, AppError::Unavailable(m) if m.contains("intricate")),
        "{error}"
    );
    assert!(corridor.blocking_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_request_body_carries_no_exclusions_unless_there_is_something_to_avoid() {
    let route = direct();
    let fake = FakeValhalla::start(move |_| ok_route(&route)).await;
    let corridor = Arc::new(FakeCorridor::default());

    service(&fake, &corridor)
        .route_bicycle(&request())
        .await
        .unwrap();

    let body: &Value = &fake.requests()[0];
    assert!(body.get("exclude_polygons").is_none());
    assert_eq!(body["elevation_interval"], 30);
}
