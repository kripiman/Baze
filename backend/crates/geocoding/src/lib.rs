// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Address search through Photon.
//!
//! What people type into the search box is personal data (ADR-0005): it is passed to Photon as a
//! properly encoded query parameter, never interpolated into the URL, and it appears in no log line
//! and in no error message.

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Map, Value};
use shared::{AppError, GeoJsonPoint, GeocodingItem, GeocodingProvider, clean_display_text};
use std::time::Duration;

/// Longest answer read from Photon: fifty small features are a few tens of kilobytes.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// What Photon is asked for at most, whatever the client asked: its own ceiling is not documented.
const MAX_RESULTS: usize = 50;
const MAX_FIELD_CHARS: usize = 200;

pub struct PhotonGeocodingService {
    http_client: Client,
    search_url: String,
}

impl PhotonGeocodingService {
    pub fn new(photon_url: String) -> Result<Self, AppError> {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(2))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| {
                AppError::Internal(format!("Failed to build the Photon HTTP client: {e}"))
            })?;
        Ok(Self::with_client(http_client, &photon_url))
    }

    fn with_client(http_client: Client, photon_url: &str) -> Self {
        Self {
            http_client,
            search_url: format!("{}/api", photon_url.trim_end_matches('/')),
        }
    }
}

/// A failure to talk to Photon. The URL is removed from the error: it carries the search text.
fn transport_error(error: reqwest::Error) -> AppError {
    let error = error.without_url();
    if error.is_timeout() {
        AppError::Upstream("Photon did not answer in time".into())
    } else if error.is_connect() {
        AppError::Upstream(format!("Photon is unreachable: {error}"))
    } else {
        AppError::Upstream(format!("Photon request failed: {error}"))
    }
}

async fn read_limited(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, AppError> {
    let too_large = || AppError::Upstream(format!("Photon answered more than {limit} bytes"));
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

fn text(properties: &Map<String, Value>, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .map(|value| clean_display_text(value, MAX_FIELD_CHARS))
        .filter(|value| !value.is_empty())
}

/// One Photon feature as a search result, or `None` when it cannot be shown: no point to go to, or
/// nothing to call it by.
fn item_from(feature: &Value) -> Option<GeocodingItem> {
    let geometry = feature.get("geometry")?;
    if geometry.get("type")?.as_str()? != "Point" {
        return None;
    }
    let coordinates = geometry.get("coordinates")?.as_array()?;
    let location = GeoJsonPoint::new(
        coordinates.first()?.as_f64()?,
        coordinates.get(1)?.as_f64()?,
    );
    location.validate().ok()?;

    let properties = feature.get("properties")?.as_object()?;
    let street = match (text(properties, "street"), text(properties, "housenumber")) {
        (Some(street), Some(number)) => Some(format!("{street} {number}")),
        (street, _) => street,
    };
    let city = text(properties, "city")
        .or_else(|| text(properties, "locality"))
        .or_else(|| text(properties, "district"))
        .or_else(|| text(properties, "county"));
    let name = text(properties, "name")
        .or_else(|| street.clone())
        .or_else(|| city.clone())?;
    Some(GeocodingItem {
        name,
        street,
        city,
        country: text(properties, "country"),
        location,
    })
}

#[async_trait]
impl GeocodingProvider for PhotonGeocodingService {
    async fn search_address(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<GeocodingItem>, AppError> {
        let wanted = limit.clamp(1, MAX_RESULTS);
        let response = self
            .http_client
            .get(&self.search_url)
            .query(&[("q", query), ("limit", &wanted.to_string())])
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(AppError::Upstream(format!("Photon answered HTTP {status}")));
        }
        let bytes = read_limited(response, MAX_RESPONSE_BYTES).await?;

        let answer: Value = serde_json::from_slice(&bytes).map_err(|error| {
            // Only the kind of failure: serde's message can quote part of the answer.
            AppError::Upstream(format!(
                "Photon answered invalid JSON ({:?})",
                error.classify()
            ))
        })?;
        let features = answer
            .get("features")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                AppError::Upstream(
                    "Photon answered something that is not a feature collection".into(),
                )
            })?;
        Ok(features.iter().filter_map(item_from).take(wanted).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        extract::{Query, State},
        http::StatusCode,
        routing::get,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    type Queries = Arc<Mutex<Vec<HashMap<String, String>>>>;

    struct State_ {
        queries: Queries,
        status: StatusCode,
        body: String,
        delay: Option<Duration>,
    }

    struct FakePhoton {
        url: String,
        queries: Queries,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for FakePhoton {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn start(status: StatusCode, body: impl ToString, delay: Option<Duration>) -> FakePhoton {
        let queries: Queries = Arc::default();
        let state = Arc::new(State_ {
            queries: queries.clone(),
            status,
            body: body.to_string(),
            delay,
        });
        async fn handle(
            State(state): State<Arc<State_>>,
            Query(query): Query<HashMap<String, String>>,
        ) -> (StatusCode, [(&'static str, &'static str); 1], String) {
            state.queries.lock().unwrap().push(query);
            if let Some(delay) = state.delay {
                tokio::time::sleep(delay).await;
            }
            (
                state.status,
                [("content-type", "application/json")],
                state.body.clone(),
            )
        }
        let app = Router::new().route("/api", get(handle)).with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        FakePhoton { url, queries, task }
    }

    fn service(fake: &FakePhoton) -> PhotonGeocodingService {
        PhotonGeocodingService::new(fake.url.clone()).unwrap()
    }

    fn feature(lon: f64, lat: f64, properties: Value) -> Value {
        json!({
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [lon, lat]},
            "properties": properties,
        })
    }

    fn collection(features: Vec<Value>) -> Value {
        json!({"type": "FeatureCollection", "features": features})
    }

    #[tokio::test]
    async fn features_become_search_results() {
        let answer = collection(vec![
            feature(
                -70.6112,
                -33.4263,
                json!({
                    "name": "Plaza Italia",
                    "street": "Avenida Providencia",
                    "housenumber": "1234",
                    "city": "Santiago",
                    "country": "Chile",
                    "osm_id": 4242,
                    "postcode": "7500000"
                }),
            ),
            // No name: the address is the name. No city: a district stands in.
            feature(
                -70.58,
                -33.45,
                json!({"street": "Calle Larga", "district": "Ñuñoa", "country": "Chile"}),
            ),
            // Only a place: its own name and nothing else.
            feature(
                -71.6,
                -33.04,
                json!({"name": "Valparaíso", "county": "Valparaíso"}),
            ),
        ]);
        let fake = start(StatusCode::OK, answer, None).await;

        let items = service(&fake).search_address("plaza", 10).await.unwrap();

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].name, "Plaza Italia");
        assert_eq!(items[0].street.as_deref(), Some("Avenida Providencia 1234"));
        assert_eq!(items[0].city.as_deref(), Some("Santiago"));
        assert_eq!(items[0].country.as_deref(), Some("Chile"));
        assert_eq!(items[0].location.coordinates, [-70.6112, -33.4263]);
        assert_eq!(items[1].name, "Calle Larga");
        assert_eq!(items[1].street.as_deref(), Some("Calle Larga"));
        assert_eq!(items[1].city.as_deref(), Some("Ñuñoa"));
        assert_eq!(items[2].name, "Valparaíso");
        assert_eq!(items[2].street, None);
        assert_eq!(items[2].city.as_deref(), Some("Valparaíso"));
    }

    #[tokio::test]
    async fn the_search_text_travels_as_one_encoded_parameter() {
        let fake = start(StatusCode::OK, collection(vec![]), None).await;
        let hostile = "Ñuñoa & limit=1000&lang=xx#frag?x=y%00";

        service(&fake).search_address(hostile, 5).await.unwrap();

        let queries = fake.queries.lock().unwrap();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].get("q").map(String::as_str), Some(hostile));
        assert_eq!(queries[0].get("limit").map(String::as_str), Some("5"));
        // Nothing the text said became a parameter of its own.
        assert_eq!(queries[0].len(), 2, "{:?}", queries[0]);
    }

    #[tokio::test]
    async fn the_number_of_results_asked_of_photon_is_capped() {
        let fake = start(StatusCode::OK, collection(vec![]), None).await;

        service(&fake).search_address("calle", 100).await.unwrap();
        service(&fake).search_address("calle", 0).await.unwrap();

        let queries = fake.queries.lock().unwrap();
        assert_eq!(queries[0]["limit"], "50");
        assert_eq!(queries[1]["limit"], "1");
    }

    #[tokio::test]
    async fn features_that_cannot_be_shown_are_dropped_and_the_rest_kept() {
        let good = feature(-70.65, -33.45, json!({"name": "Bueno"}));
        let answer = collection(vec![
            json!({"type": "Feature", "geometry": null, "properties": {"name": "sin punto"}}),
            json!({"type": "Feature", "properties": {"name": "sin geometría"}}),
            json!({"type": "Feature", "geometry": {"type": "Polygon", "coordinates": [[[0, 0], [1, 1], [0, 1], [0, 0]]]}, "properties": {"name": "polígono"}}),
            feature(-70.65, 95.0, json!({"name": "fuera del planeta"})),
            feature(500.0, -33.0, json!({"name": "fuera del planeta"})),
            json!({"type": "Feature", "geometry": {"type": "Point", "coordinates": [-70.65]}, "properties": {"name": "incompleto"}}),
            json!({"type": "Feature", "geometry": {"type": "Point", "coordinates": ["a", "b"]}, "properties": {"name": "texto"}}),
            json!({"type": "Feature", "geometry": {"type": "Point", "coordinates": [-70.65, -33.45]}}),
            feature(-70.65, -33.45, json!({})),
            feature(-70.65, -33.45, json!({"name": "   ", "osm_id": 7})),
            good,
            json!("not even a feature"),
        ]);
        let fake = start(StatusCode::OK, answer, None).await;

        let items = service(&fake).search_address("x", 10).await.unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Bueno");
    }

    #[tokio::test]
    async fn never_more_results_than_asked_for() {
        let features = (0..20)
            .map(|i| feature(-70.65, -33.45, json!({"name": format!("Lugar {i}")})))
            .collect();
        let fake = start(StatusCode::OK, collection(features), None).await;

        let items = service(&fake).search_address("lugar", 3).await.unwrap();

        assert_eq!(items.len(), 3);
        assert_eq!(items[2].name, "Lugar 2");
    }

    #[tokio::test]
    async fn text_from_the_map_is_made_safe_to_display() {
        let answer = collection(vec![feature(
            -70.65,
            -33.45,
            json!({
                "name": "Café \u{202e}Central\n\t(cerrado)\u{0}",
                "street": "x".repeat(500),
            }),
        )]);
        let fake = start(StatusCode::OK, answer, None).await;

        let items = service(&fake).search_address("cafe", 10).await.unwrap();

        assert_eq!(items[0].name, "Café Central (cerrado)");
        assert_eq!(items[0].street.as_ref().unwrap().chars().count(), 200);
    }

    #[tokio::test]
    async fn no_matches_is_an_empty_list_not_an_error() {
        let fake = start(StatusCode::OK, collection(vec![]), None).await;
        assert!(
            service(&fake)
                .search_address("zzzz", 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    fn assert_upstream_without(error: &AppError, secret: &str) {
        assert!(matches!(error, AppError::Upstream(_)), "{error}");
        assert!(
            !error.to_string().contains(secret),
            "the search text leaked: {error}"
        );
    }

    #[tokio::test]
    async fn photon_failures_are_upstream_errors_that_do_not_repeat_the_search() {
        let secret = "mi casa en calle secreta 42";
        let cases = [
            (StatusCode::INTERNAL_SERVER_ERROR, "boom".to_string()),
            (
                StatusCode::BAD_REQUEST,
                "{\"message\":\"nope\"}".to_string(),
            ),
            (StatusCode::OK, "this is not json".to_string()),
            (StatusCode::OK, "{\"hits\":[]}".to_string()),
            (StatusCode::OK, "[]".to_string()),
            (
                StatusCode::OK,
                json!({"features": "calle secreta"}).to_string(),
            ),
        ];
        for (status, body) in cases {
            let fake = start(status, body.clone(), None).await;
            let error = service(&fake).search_address(secret, 10).await.unwrap_err();
            assert_upstream_without(&error, secret);
            assert_upstream_without(&error, "calle secreta");
        }
    }

    #[tokio::test]
    async fn a_photon_that_is_down_does_not_leak_the_search_through_the_error() {
        let secret = "direccion-confidencial";
        let url = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}", listener.local_addr().unwrap())
        };
        let service = PhotonGeocodingService::new(url).unwrap();

        let error = service.search_address(secret, 10).await.unwrap_err();

        assert!(
            matches!(&error, AppError::Upstream(m) if m.contains("unreachable")),
            "{error}"
        );
        assert_upstream_without(&error, secret);
    }

    #[tokio::test]
    async fn a_photon_that_does_not_answer_in_time_is_an_upstream_error() {
        let fake = start(
            StatusCode::OK,
            collection(vec![]),
            Some(Duration::from_secs(2)),
        )
        .await;
        let client = Client::builder()
            .timeout(Duration::from_millis(200))
            .build()
            .unwrap();
        let service = PhotonGeocodingService::with_client(client, &fake.url);

        let error = service
            .search_address("lento-secreto", 10)
            .await
            .unwrap_err();

        assert!(
            matches!(&error, AppError::Upstream(m) if m.contains("in time")),
            "{error}"
        );
        assert_upstream_without(&error, "lento-secreto");
    }

    #[tokio::test]
    async fn an_oversized_answer_is_refused_not_buffered() {
        let filler = "x".repeat(MAX_RESPONSE_BYTES + 1);
        let fake = start(
            StatusCode::OK,
            collection(vec![feature(-70.0, -33.0, json!({"name": filler}))]),
            None,
        )
        .await;

        let error = service(&fake).search_address("x", 10).await.unwrap_err();

        assert!(
            matches!(&error, AppError::Upstream(m) if m.contains("more than")),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_trailing_slash_in_the_configured_url_is_harmless() {
        let fake = start(StatusCode::OK, collection(vec![]), None).await;
        let service = PhotonGeocodingService::new(format!("{}/", fake.url)).unwrap();
        service.search_address("x", 1).await.unwrap();
        assert_eq!(fake.queries.lock().unwrap().len(), 1);
    }
}
