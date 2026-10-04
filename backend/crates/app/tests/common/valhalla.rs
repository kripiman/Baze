// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A stand-in for Valhalla's `/route` on a loopback port, for the tests that run the real routing
//! client through the HTTP layer. The routing crate has its own, richer one for its unit tests.
#![allow(dead_code)]

use axum::{Router, body::Bytes, extract::State, routing::post};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

type Responder = Box<dyn Fn(&Value) -> Value + Send + Sync>;

struct ServerState {
    requests: Arc<Mutex<Vec<Value>>>,
    responder: Responder,
}

pub struct FakeValhalla {
    pub url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl FakeValhalla {
    /// `responder` gets the request body and returns the route JSON to send back.
    pub async fn start(responder: impl Fn(&Value) -> Value + Send + Sync + 'static) -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(ServerState {
            requests: requests.clone(),
            responder: Box::new(responder),
        });
        async fn handle(
            State(state): State<Arc<ServerState>>,
            body: Bytes,
        ) -> ([(&'static str, &'static str); 1], String) {
            let request = serde_json::from_slice(&body).unwrap_or(Value::Null);
            state.requests.lock().unwrap().push(request.clone());
            (
                [("content-type", "application/json")],
                (state.responder)(&request).to_string(),
            )
        }
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

    pub fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for FakeValhalla {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// How many polygons the request asks the engine to avoid.
pub fn excluded_count(request: &Value) -> usize {
    request["exclude_polygons"].as_array().map_or(0, Vec::len)
}

pub fn straight(from: [f64; 2], to: [f64; 2], steps: usize) -> Vec<[f64; 2]> {
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

/// A route that bows `bulge_degrees` of latitude away from the straight line between its ends.
pub fn detour(from: [f64; 2], to: [f64; 2], bulge_degrees: f64) -> Vec<[f64; 2]> {
    let apex = [
        (from[0] + to[0]) / 2.0,
        (from[1] + to[1]) / 2.0 + bulge_degrees,
    ];
    let mut points = straight(from, apex, 100);
    points.extend(straight(apex, to, 100).into_iter().skip(1));
    points
}

fn encode(points: &[[f64; 2]]) -> String {
    fn push(value: i64, out: &mut String) {
        let mut v = if value < 0 { !(value << 1) } else { value << 1 };
        while v >= 0x20 {
            out.push(char::from(((0x20 | (v & 0x1f)) + 63) as u8));
            v >>= 5;
        }
        out.push(char::from((v + 63) as u8));
    }
    let (mut previous_lat, mut previous_lon) = (0i64, 0i64);
    let mut out = String::new();
    for point in points {
        let lat = (point[1] * 1e6).round() as i64;
        let lon = (point[0] * 1e6).round() as i64;
        push(lat - previous_lat, &mut out);
        push(lon - previous_lon, &mut out);
        previous_lat = lat;
        previous_lon = lon;
    }
    out
}

/// Valhalla's answer for a route along `points`, with a climb of 20 m.
pub fn route_json(points: &[[f64; 2]]) -> Value {
    let last = points.len() - 1;
    json!({"trip": {
        "status": 0,
        "units": "kilometers",
        "summary": {"length": 9.5, "time": 1900.0},
        "legs": [{
            "shape": encode(points),
            "elevation": [100.0, 110.0, 105.0, 115.0],
            "maneuvers": [
                {"instruction": "Salga hacia el este.", "time": 1800.0, "length": 9.4, "begin_shape_index": 0},
                {"instruction": "Ha llegado a su destino.", "time": 0.0, "length": 0.0, "begin_shape_index": last},
            ],
        }],
    }})
}
