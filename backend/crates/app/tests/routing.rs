// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Routing and address search through the HTTP layer: what each failure looks like to a client, and
//! what is guaranteed when the engines are off. A made-up route returned with HTTP 200 is dangerous,
//! because a cyclist would follow it.

mod common;

use async_trait::async_trait;
use axum::http::{StatusCode, header};
use common::valhalla::{FakeValhalla, route_json, straight};
use common::{
    FakeHazards, get, json_request, point, send, test_accounts, test_app, test_app_with_engines,
    test_app_with_services, test_config,
};
use realtime::RealtimeService;
use routing::ValhallaRoutingService;
use serde_json::{Value, json};
use shared::{
    AppError, GeoJsonLineString, GeoJsonPoint, GeocodingItem, GeocodingProvider, RouteRequest,
    RouteResponse, RoutingProvider,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

const ORIGIN: [f64; 2] = [-70.70, -33.45];
const DESTINATION: [f64; 2] = [-70.60, -33.45];

fn route_body() -> Value {
    json!({
        "origin": {"type": "Point", "coordinates": ORIGIN},
        "destination": {"type": "Point", "coordinates": DESTINATION},
    })
}

fn post_route(body: &Value) -> axum::http::Request<axum::body::Body> {
    json_request("POST", "/api/v1/routing/route", None, body)
}

/// A routing engine that answers whatever the test scripts and counts how often it was asked.
struct ScriptedRouting {
    calls: AtomicUsize,
    answer: Box<dyn Fn() -> Result<RouteResponse, AppError> + Send + Sync>,
}

impl ScriptedRouting {
    fn new(
        answer: impl Fn() -> Result<RouteResponse, AppError> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            answer: Box::new(answer),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl RoutingProvider for ScriptedRouting {
    async fn route_bicycle(&self, _request: &RouteRequest) -> Result<RouteResponse, AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        (self.answer)()
    }
}

struct ScriptedGeocoder(Box<dyn Fn() -> Result<Vec<GeocodingItem>, AppError> + Send + Sync>);

#[async_trait]
impl GeocodingProvider for ScriptedGeocoder {
    async fn search_address(
        &self,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<GeocodingItem>, AppError> {
        (self.0)()
    }
}

fn sample_route() -> RouteResponse {
    RouteResponse {
        distance_meters: 9500.0,
        duration_seconds: 1900.0,
        ascent_meters: 12.5,
        descent_meters: 3.0,
        geometry: GeoJsonLineString {
            geom_type: "LineString".into(),
            coordinates: vec![ORIGIN, DESTINATION],
        },
        maneuvers: vec![shared::RouteManeuver {
            instruction: "Salga hacia el este.".into(),
            distance_meters: 9400.0,
            time_seconds: 1800.0,
            location: GeoJsonPoint::new(ORIGIN[0], ORIGIN[1]),
        }],
        nearby_hazards: vec![],
    }
}

// ------------------------------------------------------------------ engines off

#[tokio::test]
async fn routing_answers_501_instead_of_a_fabricated_route_when_the_engines_are_off() {
    let (status, body) = send(&test_app(), post_route(&route_body())).await;

    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
    assert!(body["error"].as_str().unwrap().contains("not enabled"));
    assert!(
        body.get("geometry").is_none(),
        "no route data may be returned: {body}"
    );
}

#[tokio::test]
async fn geocoding_answers_501_instead_of_an_empty_result_when_the_engines_are_off() {
    let (status, body) = send(
        &test_app(),
        get("/api/v1/geocoding/search?q=Plaza%20de%20Armas"),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
    assert!(body["error"].as_str().unwrap().contains("not enabled"));
}

// ------------------------------------------------------------------ input checks

#[tokio::test]
async fn invalid_routing_input_is_rejected_before_it_reaches_the_engine() {
    let engine = ScriptedRouting::new(|| Ok(sample_route()));
    let app = test_app_with_engines(
        test_config(),
        engine.clone(),
        Arc::new(ScriptedGeocoder(Box::new(|| Ok(vec![])))),
    );
    let bad_bodies = [
        json!({"origin": point(), "destination": point()}),
        json!({"origin": point(), "destination": {"type": "Point", "coordinates": [-70.6, 95.0]}}),
        json!({"origin": {"type": "Polygon", "coordinates": [-70.6, -33.4]}, "destination": point()}),
        // Santiago to Puerto Montt: ~850 km, beyond the 150 km limit.
        json!({"origin": point(), "destination": {"type": "Point", "coordinates": [-72.94, -41.47]}}),
    ];

    for body in bad_bodies {
        let (status, response) = send(&app, post_route(&body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body} -> {response}");
    }

    assert_eq!(engine.calls(), 0, "nothing invalid may cost an engine call");
}

#[tokio::test]
async fn the_distance_limit_names_itself() {
    let engine = ScriptedRouting::new(|| Ok(sample_route()));
    let app = test_app_with_engines(
        test_config(),
        engine,
        Arc::new(ScriptedGeocoder(Box::new(|| Ok(vec![])))),
    );
    let far = json!({"origin": point(), "destination": {"type": "Point", "coordinates": [-72.94, -41.47]}});

    let (_, body) = send(&app, post_route(&far)).await;

    assert!(body["error"].as_str().unwrap().contains("150 km"), "{body}");
}

// ------------------------------------------------------------------ how failures look to a client

#[tokio::test]
async fn a_route_is_served_with_its_alerts_and_its_climb() {
    let app = test_app_with_engines(
        test_config(),
        ScriptedRouting::new(|| Ok(sample_route())),
        Arc::new(ScriptedGeocoder(Box::new(|| Ok(vec![])))),
    );

    let (status, body) = send(&app, post_route(&route_body())).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["distance_meters"], 9500.0);
    assert_eq!(body["ascent_meters"], 12.5);
    assert_eq!(body["geometry"]["type"], "LineString");
    assert_eq!(body["maneuvers"][0]["instruction"], "Salga hacia el este.");
    assert_eq!(body["nearby_hazards"], json!([]));
}

#[tokio::test]
async fn no_route_is_404_and_a_closure_that_cannot_be_avoided_is_503() {
    let missing = ScriptedRouting::new(|| {
        Err(AppError::NotFound(
            "No route avoids the confirmed closures between these points".into(),
        ))
    });
    let app = test_app_with_engines(
        test_config(),
        missing,
        Arc::new(ScriptedGeocoder(Box::new(|| Ok(vec![])))),
    );
    let (status, body) = send(&app, post_route(&route_body())).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(body["error"].as_str().unwrap().contains("avoids"));

    let busy = ScriptedRouting::new(|| {
        Err(AppError::Unavailable(
            "Too many confirmed closures along this route to avoid them all".into(),
        ))
    });
    let app = test_app_with_engines(
        test_config(),
        busy,
        Arc::new(ScriptedGeocoder(Box::new(|| Ok(vec![])))),
    );
    let response = app.oneshot(post_route(&route_body())).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key(header::RETRY_AFTER));
}

#[tokio::test]
async fn an_engine_failure_is_a_generic_502_that_does_not_echo_the_details() {
    let engine = ScriptedRouting::new(|| {
        Err(AppError::Upstream(
            "Valhalla is unreachable: connection refused at 10.0.0.7:8002".into(),
        ))
    });
    let app = test_app_with_engines(
        test_config(),
        engine,
        Arc::new(ScriptedGeocoder(Box::new(|| Ok(vec![])))),
    );

    let (status, body) = send(&app, post_route(&route_body())).await;

    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert!(body["error_id"].is_string(), "{body}");
    assert!(!body.to_string().contains("10.0.0.7"), "{body}");
    assert!(!body.to_string().contains("Valhalla"), "{body}");
}

#[tokio::test]
async fn address_search_is_served_and_its_failures_stay_generic() {
    let item = GeocodingItem {
        name: "Plaza de Armas".into(),
        street: None,
        city: Some("Santiago".into()),
        country: Some("Chile".into()),
        location: GeoJsonPoint::new(-70.6506, -33.4378),
    };
    let working = ScriptedGeocoder(Box::new(move || Ok(vec![item.clone()])));
    let app = test_app_with_engines(
        test_config(),
        ScriptedRouting::new(|| Ok(sample_route())),
        Arc::new(working),
    );
    let (status, body) = send(&app, get("/api/v1/geocoding/search?q=plaza")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body[0]["name"], "Plaza de Armas");
    assert_eq!(
        body[0]["location"]["coordinates"],
        json!([-70.6506, -33.4378])
    );

    let broken = ScriptedGeocoder(Box::new(|| {
        Err(AppError::Upstream(
            "Photon answered HTTP 500 Internal Server Error".into(),
        ))
    }));
    let app = test_app_with_engines(
        test_config(),
        ScriptedRouting::new(|| Ok(sample_route())),
        Arc::new(broken),
    );
    let (status, body) = send(&app, get("/api/v1/geocoding/search?q=calle-secreta")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert!(!body.to_string().contains("Photon"), "{body}");
    assert!(!body.to_string().contains("calle-secreta"), "{body}");
}

// ------------------------------------------------------------------ the real client end to end

#[tokio::test]
async fn the_real_routing_client_serves_a_route_through_the_http_layer() {
    let points = straight(ORIGIN, DESTINATION, 200);
    let answer = route_json(&points);
    let valhalla = FakeValhalla::start(move |_| answer.clone()).await;
    let hazards = Arc::new(FakeHazards::default());
    let routing =
        Arc::new(ValhallaRoutingService::new(valhalla.url.clone(), hazards.clone()).unwrap());
    let config = test_config();
    let app = test_app_with_services(
        config.clone(),
        test_accounts(&config),
        hazards,
        routing,
        Arc::new(baze_app::disabled::EnginesDisabled),
        RealtimeService::new(16),
    );

    let (status, body) = send(&app, post_route(&route_body())).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["distance_meters"], 9500.0);
    assert_eq!(body["ascent_meters"], 20.0, "{body}");
    assert_eq!(body["descent_meters"], 5.0, "{body}");
    assert_eq!(
        body["geometry"]["coordinates"].as_array().unwrap().len(),
        201
    );
    let request = &valhalla.requests()[0];
    assert_eq!(request["costing"], "bicycle");
    assert_eq!(request["locations"][0]["lat"], ORIGIN[1]);
    assert!(request.get("exclude_polygons").is_none());
}
