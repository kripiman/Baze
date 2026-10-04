// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Test doubles: a Valhalla that answers what a test scripts, and a hazard store that knows where its
//! closures are. Both are real enough for the client and the avoidance loop to run unmodified.

use crate::polyline::encode_polyline6;
use async_trait::async_trait;
use axum::{Router, body::Bytes, extract::State, http::StatusCode, routing::post};
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::{Value, json};
use shared::{
    AppError, CorridorHazards, GeoJsonLineString, GeoJsonPoint, GeoJsonPolygon, Hazard,
    HazardCategory, HazardStatus, MAX_CORRIDOR_POINTS, haversine_meters,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

pub(crate) type Responder = Box<dyn Fn(&Value) -> (StatusCode, String) + Send + Sync>;

struct ServerState {
    requests: Arc<Mutex<Vec<Value>>>,
    responder: Responder,
    delay: Option<Duration>,
}

/// An HTTP server on a loopback port that plays the part of Valhalla's `/route`.
pub(crate) struct FakeValhalla {
    pub url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl FakeValhalla {
    pub(crate) async fn start(
        responder: impl Fn(&Value) -> (StatusCode, String) + Send + Sync + 'static,
    ) -> Self {
        Self::start_with_delay(responder, None).await
    }

    pub(crate) async fn start_with_delay(
        responder: impl Fn(&Value) -> (StatusCode, String) + Send + Sync + 'static,
        delay: Option<Duration>,
    ) -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(ServerState {
            requests: requests.clone(),
            responder: Box::new(responder),
            delay,
        });
        let app = Router::new()
            .route("/route", post(handle))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            url,
            requests,
            task,
        }
    }

    /// The JSON bodies received so far, oldest first.
    pub(crate) fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for FakeValhalla {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle(
    State(state): State<Arc<ServerState>>,
    body: Bytes,
) -> (StatusCode, [(&'static str, &'static str); 1], String) {
    let request = serde_json::from_slice(&body).unwrap_or(Value::Null);
    state.requests.lock().unwrap().push(request.clone());
    if let Some(delay) = state.delay {
        tokio::time::sleep(delay).await;
    }
    let (status, answer) = (state.responder)(&request);
    (status, [("content-type", "application/json")], answer)
}

/// The number of polygons a request asks the engine to avoid.
pub(crate) fn excluded_count(request: &Value) -> usize {
    request["exclude_polygons"].as_array().map_or(0, Vec::len)
}

/// A straight run from `from` to `to` with `steps` segments (so `steps + 1` points).
pub(crate) fn straight(from: [f64; 2], to: [f64; 2], steps: usize) -> Vec<[f64; 2]> {
    (0..=steps)
        .map(|i| {
            let t = i as f64 / steps as f64;
            [
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
            ]
        })
        .collect()
}

/// A route that bows `bulge_degrees` of latitude away from the straight line between the ends.
pub(crate) fn detour(from: [f64; 2], to: [f64; 2], bulge_degrees: f64) -> Vec<[f64; 2]> {
    let apex = [
        (from[0] + to[0]) / 2.0,
        (from[1] + to[1]) / 2.0 + bulge_degrees,
    ];
    let mut points = straight(from, apex, 100);
    points.extend(straight(apex, to, 100).into_iter().skip(1));
    points
}

/// Valhalla's answer for `points`: one maneuver at the start and one at the end, no elevation.
pub(crate) fn route_json(points: &[[f64; 2]]) -> Value {
    let last = points.len() - 1;
    json!({"trip": {
        "status": 0,
        "status_message": "Found route between points",
        "units": "kilometers",
        "summary": {"length": 9.5, "time": 1900.0},
        "legs": [{
            "shape": encode_polyline6(points),
            "summary": {"length": 9.5, "time": 1900.0},
            "maneuvers": [
                {"instruction": "Salga hacia el este.", "time": 1800.0, "length": 9.4, "begin_shape_index": 0, "end_shape_index": last},
                {"instruction": "Ha llegado a su destino.", "time": 0.0, "length": 0.0, "begin_shape_index": last, "end_shape_index": last},
            ],
        }],
    }})
}

pub(crate) fn ok_route(points: &[[f64; 2]]) -> (StatusCode, String) {
    (StatusCode::OK, route_json(points).to_string())
}

/// Valhalla's body for a request it refuses.
pub(crate) fn engine_error(code: i64, message: &str) -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        json!({"error_code": code, "error": message, "status_code": 400, "status": "Bad Request"})
            .to_string(),
    )
}

pub(crate) fn square(lon: f64, lat: f64, half: f64) -> GeoJsonPolygon {
    GeoJsonPolygon {
        geom_type: "Polygon".into(),
        coordinates: vec![vec![
            [lon - half, lat - half],
            [lon + half, lat - half],
            [lon + half, lat + half],
            [lon - half, lat + half],
            [lon - half, lat - half],
        ]],
    }
}

pub(crate) struct Closure {
    pub center: [f64; 2],
    pub polygon: GeoJsonPolygon,
}

impl Closure {
    /// A closure of about 30 m of radius around `center`.
    pub(crate) fn at(lon: f64, lat: f64) -> Self {
        Self {
            center: [lon, lat],
            polygon: square(lon, lat, 0.0003),
        }
    }
}

/// A call received by the fake store: how many points the corridor had and how wide it was.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CorridorCall {
    pub points: usize,
    pub buffer_meters: f64,
}

/// A hazard store that holds closures at known places. Like the real one it refuses corridors with too
/// many points, so a route that was not simplified fails loudly instead of being skipped.
#[derive(Default)]
pub(crate) struct FakeCorridor {
    closures: Vec<Closure>,
    alerts: Vec<Hazard>,
    fail_blocking: Option<fn() -> AppError>,
    pub blocking_calls: Mutex<Vec<CorridorCall>>,
    pub alert_calls: Mutex<Vec<(CorridorCall, Vec<[f64; 2]>)>>,
}

impl FakeCorridor {
    pub(crate) fn with_closures(closures: Vec<Closure>) -> Self {
        Self {
            closures,
            ..Self::default()
        }
    }

    pub(crate) fn with_alerts(mut self, alerts: Vec<Hazard>) -> Self {
        self.alerts = alerts;
        self
    }

    pub(crate) fn failing(error: fn() -> AppError) -> Self {
        Self {
            fail_blocking: Some(error),
            ..Self::default()
        }
    }
}

fn check_like_the_store(corridor: &GeoJsonLineString) -> Result<(), AppError> {
    if corridor.coordinates.len() < 2 || corridor.coordinates.len() > MAX_CORRIDOR_POINTS {
        return Err(AppError::Validation(
            "Corridor must have between 2 and 10000 points".into(),
        ));
    }
    Ok(())
}

/// Whether `center` is within `meters` of the polyline, sampling every few meters along it.
fn near(corridor: &GeoJsonLineString, center: [f64; 2], meters: f64) -> bool {
    let target = GeoJsonPoint::new(center[0], center[1]);
    corridor.coordinates.windows(2).any(|pair| {
        let (a, b) = (
            GeoJsonPoint::new(pair[0][0], pair[0][1]),
            GeoJsonPoint::new(pair[1][0], pair[1][1]),
        );
        let samples = (haversine_meters(&a, &b) / 5.0).ceil().max(1.0) as usize;
        (0..=samples).any(|i| {
            let t = i as f64 / samples as f64;
            let point = GeoJsonPoint::new(
                pair[0][0] + (pair[1][0] - pair[0][0]) * t,
                pair[0][1] + (pair[1][1] - pair[0][1]) * t,
            );
            haversine_meters(&point, &target) <= meters
        })
    })
}

#[async_trait]
impl CorridorHazards for FakeCorridor {
    async fn find_blocking_polygons_along_corridor(
        &self,
        corridor: &GeoJsonLineString,
        buffer_meters: f64,
    ) -> Result<Vec<GeoJsonPolygon>, AppError> {
        self.blocking_calls.lock().unwrap().push(CorridorCall {
            points: corridor.coordinates.len(),
            buffer_meters,
        });
        if let Some(error) = self.fail_blocking {
            return Err(error());
        }
        check_like_the_store(corridor)?;
        Ok(self
            .closures
            .iter()
            .filter(|closure| near(corridor, closure.center, buffer_meters))
            .map(|closure| closure.polygon.clone())
            .collect())
    }

    async fn list_hazards_near_corridor(
        &self,
        corridor: &GeoJsonLineString,
        buffer_meters: f64,
    ) -> Result<Vec<Hazard>, AppError> {
        self.alert_calls.lock().unwrap().push((
            CorridorCall {
                points: corridor.coordinates.len(),
                buffer_meters,
            },
            corridor.coordinates.clone(),
        ));
        check_like_the_store(corridor)?;
        Ok(self.alerts.clone())
    }
}

pub(crate) fn pothole_at(lon: f64, lat: f64) -> Hazard {
    let now = Utc::now();
    Hazard {
        id: Uuid::new_v4(),
        category: HazardCategory::Pothole,
        hazard_type: HazardCategory::Pothole.hazard_type(),
        status: HazardStatus::Confirmed,
        description: None,
        upvotes: 1,
        downvotes: 0,
        location: GeoJsonPoint::new(lon, lat),
        created_at: now,
        expires_at: now + ChronoDuration::hours(24),
    }
}
