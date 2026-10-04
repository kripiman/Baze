// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Client for Valhalla's `/route` action.
//!
//! Nothing in here may put a coordinate, or text taken from the engine's answer, into an error: those
//! messages end up in the server log (ADR-0005: Baze never keeps where people go).

use crate::exclusions::exclusion_rings;
use crate::polyline::decode_polyline6;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use shared::{
    AppError, GeoJsonLineString, GeoJsonPoint, GeoJsonPolygon, RouteManeuver, clean_display_text,
};
use std::time::Duration;

/// The longest answer read from the engine. A 150 km route with turn-by-turn text is a few MB at most.
pub(crate) const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
/// Spacing of the elevation samples; 30 m is the resolution of the default elevation data.
const ELEVATION_INTERVAL_METERS: u32 = 30;
const MAX_INSTRUCTION_CHARS: usize = 300;
/// Valhalla marks a missing sample with -32768. Nothing on land is below the Dead Sea shore (-430 m).
const MIN_PLAUSIBLE_ELEVATION_METERS: f64 = -500.0;

/// Valhalla's own codes for "there is no route between these places".
const NO_ROUTE_CODES: [i64; 5] = [170, 171, 440, 441, 442];
/// "Path distance exceeds the max distance limit".
const TOO_LONG_CODE: i64 = 154;

/// What Valhalla answered, reduced to what Baze serves.
#[derive(Debug)]
pub(crate) struct EngineRoute {
    pub geometry: GeoJsonLineString,
    pub distance_meters: f64,
    pub duration_seconds: f64,
    pub ascent_meters: f64,
    pub descent_meters: f64,
    pub maneuvers: Vec<RouteManeuver>,
}

/// Why the engine did not produce a route.
#[derive(Debug)]
pub(crate) enum EngineError {
    /// The places are not connected by any road a bicycle can use (or one of them is off the map).
    NoRoute,
    /// The engine refused the request itself (other than for lack of a route); carries its error code.
    Rejected(i64),
    /// The engine could not be reached, answered an error, or answered nonsense.
    Failed(AppError),
}

impl From<AppError> for EngineError {
    fn from(error: AppError) -> Self {
        Self::Failed(error)
    }
}

/// Client for services on the private network: bounded time, no redirects.
pub(crate) fn internal_http_client() -> Result<Client, AppError> {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Internal(format!("Failed to build the Valhalla HTTP client: {e}")))
}

pub(crate) struct ValhallaClient {
    http: Client,
    route_url: String,
    max_response_bytes: usize,
}

impl ValhallaClient {
    pub(crate) fn new(base_url: &str) -> Result<Self, AppError> {
        Ok(Self::with_client(internal_http_client()?, base_url))
    }

    pub(crate) fn with_client(http: Client, base_url: &str) -> Self {
        Self {
            http,
            route_url: format!("{}/route", base_url.trim_end_matches('/')),
            max_response_bytes: MAX_RESPONSE_BYTES,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_max_response_bytes(mut self, limit: usize) -> Self {
        self.max_response_bytes = limit;
        self
    }

    /// Asks for the best bicycle route that does not touch `exclude` (closed areas).
    pub(crate) async fn route(
        &self,
        origin: &GeoJsonPoint,
        destination: &GeoJsonPoint,
        exclude: &[GeoJsonPolygon],
    ) -> Result<EngineRoute, EngineError> {
        let body = route_request_body(origin, destination, exclude)?;
        let response = self
            .http
            .post(&self.route_url)
            .json(&body)
            .send()
            .await
            .map_err(|error| EngineError::Failed(transport_error(error)))?;

        let status = response.status();
        if status.is_success() {
            let bytes = read_limited(response, self.max_response_bytes).await?;
            return parse_route(&bytes);
        }
        if status.is_client_error() {
            let bytes = read_limited(response, MAX_ERROR_BODY_BYTES).await?;
            return Err(classify_rejection(&bytes));
        }
        Err(EngineError::Failed(AppError::Upstream(format!(
            "Valhalla answered HTTP {status}"
        ))))
    }
}

/// A failure to talk to the engine, without the request URL (logs must not carry what was asked).
fn transport_error(error: reqwest::Error) -> AppError {
    let error = error.without_url();
    if error.is_timeout() {
        AppError::Upstream("Valhalla did not answer in time".into())
    } else if error.is_connect() {
        AppError::Upstream(format!("Valhalla is unreachable: {error}"))
    } else {
        AppError::Upstream(format!("Valhalla request failed: {error}"))
    }
}

/// Reads a body up to `limit` bytes; anything longer is refused rather than buffered.
async fn read_limited(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, AppError> {
    let too_large = || AppError::Upstream(format!("Valhalla answered more than {limit} bytes"));
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if body.len() + chunk.len() > limit {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// The JSON sent to `/route`. `exclude_polygons` is only present when there is something to avoid.
pub(crate) fn route_request_body(
    origin: &GeoJsonPoint,
    destination: &GeoJsonPoint,
    exclude: &[GeoJsonPolygon],
) -> Result<Value, AppError> {
    let mut body = json!({
        "locations": [
            {"lat": origin.lat(), "lon": origin.lon(), "type": "break"},
            {"lat": destination.lat(), "lon": destination.lon(), "type": "break"},
        ],
        "costing": "bicycle",
        "costing_options": {"bicycle": {"bicycle_type": "Hybrid", "use_roads": 0.2}},
        "units": "kilometers",
        "language": "es-ES",
        "elevation_interval": ELEVATION_INTERVAL_METERS,
    });
    if !exclude.is_empty() {
        body["exclude_polygons"] = json!(exclusion_rings(exclude)?);
    }
    Ok(body)
}

fn classify_rejection(body: &[u8]) -> EngineError {
    #[derive(Deserialize)]
    struct Rejection {
        error_code: Option<i64>,
        error: Option<String>,
    }
    let Ok(rejection) = serde_json::from_slice::<Rejection>(body) else {
        return EngineError::Failed(AppError::Upstream(
            "Valhalla rejected the request without an error code".into(),
        ));
    };
    let code = rejection.error_code.unwrap_or(-1);
    let off_the_map = rejection
        .error
        .as_deref()
        .is_some_and(|message| message.starts_with("No data found for location"));
    if NO_ROUTE_CODES.contains(&code) || off_the_map {
        EngineError::NoRoute
    } else if code == TOO_LONG_CODE {
        EngineError::Failed(AppError::Validation(
            "The route is too long for this server".into(),
        ))
    } else {
        EngineError::Rejected(code)
    }
}

#[derive(Deserialize)]
struct RouteEnvelope {
    trip: Trip,
}

#[derive(Deserialize)]
struct Trip {
    summary: Summary,
    legs: Vec<Leg>,
}

#[derive(Deserialize)]
struct Summary {
    /// Kilometers (the request asks for them).
    length: f64,
    /// Seconds.
    time: f64,
}

#[derive(Deserialize)]
struct Leg {
    /// Encoded polyline with six digits of precision.
    shape: String,
    #[serde(default)]
    maneuvers: Vec<EngineManeuver>,
    /// Heights in meters every `elevation_interval` meters; absent when the engine has no elevation data.
    #[serde(default)]
    elevation: Vec<Option<f64>>,
}

#[derive(Deserialize)]
struct EngineManeuver {
    #[serde(default)]
    instruction: String,
    /// Seconds.
    time: f64,
    /// Kilometers.
    length: f64,
    begin_shape_index: usize,
}

fn malformed(what: &str) -> EngineError {
    EngineError::Failed(AppError::Upstream(format!(
        "Valhalla answered something that is not a usable route: {what}"
    )))
}

fn parse_route(bytes: &[u8]) -> Result<EngineRoute, EngineError> {
    // Only the kind of failure is reported: serde's own message can quote a value from the answer.
    let envelope: RouteEnvelope = serde_json::from_slice(bytes)
        .map_err(|error| malformed(&format!("JSON of the wrong shape ({:?})", error.classify())))?;
    let trip = envelope.trip;
    let [leg] = <[Leg; 1]>::try_from(trip.legs).map_err(|_| malformed("not exactly one leg"))?;

    let shape = decode_polyline6(&leg.shape).map_err(|_| malformed("undecodable shape"))?;
    if shape.len() < 2 {
        return Err(malformed("a shape of fewer than two points"));
    }
    let non_negative = |value: f64| value.is_finite() && value >= 0.0;
    if !non_negative(trip.summary.length) || !non_negative(trip.summary.time) {
        return Err(malformed("a summary that is not a distance and a time"));
    }

    let maneuvers = leg
        .maneuvers
        .iter()
        .map(|maneuver| {
            let location = shape
                .get(maneuver.begin_shape_index)
                .ok_or_else(|| malformed("a maneuver outside the shape"))?;
            if !non_negative(maneuver.length) || !non_negative(maneuver.time) {
                return Err(malformed("a maneuver without a distance and a time"));
            }
            Ok(RouteManeuver {
                instruction: clean_display_text(&maneuver.instruction, MAX_INSTRUCTION_CHARS),
                distance_meters: maneuver.length * 1000.0,
                time_seconds: maneuver.time,
                location: GeoJsonPoint::new(location[0], location[1]),
            })
        })
        .collect::<Result<Vec<_>, EngineError>>()?;

    let (ascent_meters, descent_meters) = climb(&leg.elevation);
    if leg.elevation.is_empty() {
        // A graph built without elevation answers no heights. The route is still right; the climb is not
        // known, and the operator must hear about it (the data pipeline is meant to make this impossible).
        tracing::warn!("Valhalla returned no elevation: ascent and descent are reported as 0");
    }

    Ok(EngineRoute {
        geometry: GeoJsonLineString {
            geom_type: "LineString".into(),
            coordinates: shape,
        },
        distance_meters: trip.summary.length * 1000.0,
        duration_seconds: trip.summary.time,
        ascent_meters,
        descent_meters,
        maneuvers,
    })
}

/// Total climb and descent from consecutive height samples. A missing sample breaks the chain: the
/// difference across a gap is unknown, not zero.
pub(crate) fn climb(samples: &[Option<f64>]) -> (f64, f64) {
    let (mut ascent, mut descent) = (0.0, 0.0);
    let mut previous: Option<f64> = None;
    for sample in samples {
        let height = sample.filter(|h| h.is_finite() && *h > MIN_PLAUSIBLE_ELEVATION_METERS);
        if let (Some(before), Some(now)) = (previous, height) {
            if now > before {
                ascent += now - before;
            } else {
                descent += before - now;
            }
        }
        previous = height;
    }
    (ascent, descent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn climb_adds_up_the_ups_and_the_downs_separately() {
        let samples = [
            Some(100.0),
            Some(130.0),
            Some(120.0),
            Some(150.0),
            Some(150.0),
        ];
        assert_eq!(climb(&samples), (60.0, 10.0));
    }

    #[test]
    fn climb_of_nothing_or_of_one_sample_is_zero() {
        assert_eq!(climb(&[]), (0.0, 0.0));
        assert_eq!(climb(&[Some(500.0)]), (0.0, 0.0));
    }

    #[test]
    fn a_missing_sample_breaks_the_chain_instead_of_counting_as_the_sea() {
        // -32768 is Valhalla's "no data"; treating it as a height would add a 33 km cliff.
        let samples = [
            Some(100.0),
            Some(-32768.0),
            Some(110.0),
            None,
            Some(120.0),
            Some(125.0),
        ];
        assert_eq!(climb(&samples), (5.0, 0.0));
        assert_eq!(
            climb(&[Some(f64::NAN), Some(10.0), Some(20.0)]),
            (10.0, 0.0)
        );
    }

    #[test]
    fn rejections_are_told_apart_by_their_error_code() {
        let reject = |code: i64, message: &str| {
            classify_rejection(
                json!({"error_code": code, "error": message, "status_code": 400})
                    .to_string()
                    .as_bytes(),
            )
        };
        for code in NO_ROUTE_CODES {
            assert!(matches!(reject(code, "x"), EngineError::NoRoute), "{code}");
        }
        assert!(matches!(
            reject(171, "No suitable edges near location"),
            EngineError::NoRoute
        ));
        assert!(matches!(
            reject(999, "No data found for location 1"),
            EngineError::NoRoute
        ));
        assert!(matches!(
            reject(
                TOO_LONG_CODE,
                "Path distance exceeds the max distance limit"
            ),
            EngineError::Failed(AppError::Validation(_))
        ));
        assert!(matches!(
            reject(157, "Exceeded max avoid locations"),
            EngineError::Rejected(157)
        ));
        assert!(matches!(
            classify_rejection(b"<html>proxy error</html>"),
            EngineError::Failed(AppError::Upstream(_))
        ));
    }

    #[test]
    fn the_request_asks_for_a_bicycle_route_with_elevation_and_no_exclusions_by_default() {
        let body = route_request_body(
            &GeoJsonPoint::new(-70.65, -33.45),
            &GeoJsonPoint::new(-70.6, -33.4),
            &[],
        )
        .unwrap();
        assert_eq!(body["costing"], "bicycle");
        assert_eq!(body["units"], "kilometers");
        assert_eq!(body["elevation_interval"], 30);
        assert_eq!(body["locations"][0]["lat"], -33.45);
        assert_eq!(body["locations"][0]["lon"], -70.65);
        assert_eq!(body["locations"][0]["type"], "break");
        assert_eq!(body["locations"][1]["lat"], -33.4);
        assert!(body.get("exclude_polygons").is_none());
    }
}
