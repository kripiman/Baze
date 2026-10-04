// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use chrono::{DateTime, Utc};
pub use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::net::IpAddr;
use thiserror::Error;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

mod geometry;
pub use geometry::{haversine_meters, simplify_line};

/// Widest bounding box (degrees per side) a client may request when listing hazards (~55 km).
pub const MAX_LIST_BBOX_SPAN_DEGREES: f64 = 0.5;
/// Widest bounding box (degrees per side) for the realtime stream: it follows a whole route (~220 km).
pub const MAX_STREAM_BBOX_SPAN_DEGREES: f64 = 2.0;
const _: () = assert!(MAX_STREAM_BBOX_SPAN_DEGREES > MAX_LIST_BBOX_SPAN_DEGREES);

/// Farthest apart (straight line, in meters) the origin and destination of a route may be. A longer
/// trip is not a commute and would make the routing engine work for minutes.
pub const MAX_ROUTE_DISTANCE_METERS: f64 = 150_000.0;

/// Most points of a route corridor the hazard store accepts in one query. Routes are simplified to fit
/// (see [`simplify_line`]); longer lines are refused rather than sent to the database.
pub const MAX_CORRIDOR_POINTS: usize = 10_000;

pub const CLIENT_IPV4_PREFIX: u8 = 32;
pub const CLIENT_IPV6_PREFIX: u8 = 64;

/// Trunca una dirección IP a la subred de la longitud de prefijo solicitada (v4 para IPv4, v6 para IPv6).
pub fn network_of(ip: IpAddr, v4: u8, v6: u8) -> IpNet {
    match ip {
        IpAddr::V4(a) => Ipv4Net::new(a, v4)
            .expect("IPv4 prefix must be <= 32")
            .trunc()
            .into(),
        IpAddr::V6(a) => Ipv6Net::new(a, v6)
            .expect("IPv6 prefix must be <= 128")
            .trunc()
            .into(),
    }
}

/// Obtiene la red canónica para políticas de cliente (rate limits y SSE): IPv4 /32 e IPv6 /64.
pub fn client_network(ip: IpAddr) -> IpNet {
    network_of(ip, CLIENT_IPV4_PREFIX, CLIENT_IPV6_PREFIX)
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Rate limit exceeded: {0}")]
    RateLimited(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error("Service temporarily unavailable: {0}")]
    Unavailable(String),

    #[error("Upstream service error: {0}")]
    Upstream(String),

    #[error("Internal database or execution error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GeoJsonPoint {
    #[serde(rename = "type")]
    pub point_type: String, // "Point"
    /// [longitude, latitude]
    pub coordinates: [f64; 2],
}

impl GeoJsonPoint {
    pub fn new(lon: f64, lat: f64) -> Self {
        Self {
            point_type: "Point".to_string(),
            coordinates: [lon, lat],
        }
    }

    pub fn lon(&self) -> f64 {
        self.coordinates[0]
    }

    pub fn lat(&self) -> f64 {
        self.coordinates[1]
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.point_type != "Point" {
            return Err(AppError::Validation("GeoJSON type must be Point".into()));
        }
        let lon = self.lon();
        let lat = self.lat();
        if !lon.is_finite() || !lat.is_finite() {
            return Err(AppError::Validation(
                "Coordinates must be finite numbers".into(),
            ));
        }
        if !(-180.0..=180.0).contains(&lon) {
            return Err(AppError::Validation(format!(
                "Longitude {} out of range [-180, 180]",
                lon
            )));
        }
        if !(-90.0..=90.0).contains(&lat) {
            return Err(AppError::Validation(format!(
                "Latitude {} out of range [-90, 90]",
                lat
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GeoJsonLineString {
    #[serde(rename = "type")]
    pub geom_type: String, // "LineString"
    /// Array of [longitude, latitude]
    pub coordinates: Vec<[f64; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GeoJsonPolygon {
    #[serde(rename = "type")]
    pub geom_type: String, // "Polygon"
    /// Array of linear rings of [longitude, latitude]
    pub coordinates: Vec<Vec<[f64; 2]>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, IntoParams)]
pub struct BoundingBox {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

impl BoundingBox {
    pub fn validate(&self) -> Result<(), AppError> {
        if !self.min_lon.is_finite()
            || !self.min_lat.is_finite()
            || !self.max_lon.is_finite()
            || !self.max_lat.is_finite()
        {
            return Err(AppError::Validation(
                "Bbox bounds must be finite numbers".into(),
            ));
        }
        if !(-180.0..=180.0).contains(&self.min_lon) || !(-180.0..=180.0).contains(&self.max_lon) {
            return Err(AppError::Validation(
                "Bbox longitude out of range [-180, 180]".into(),
            ));
        }
        if !(-90.0..=90.0).contains(&self.min_lat) || !(-90.0..=90.0).contains(&self.max_lat) {
            return Err(AppError::Validation(
                "Bbox latitude out of range [-90, 90]".into(),
            ));
        }
        if self.min_lon >= self.max_lon {
            return Err(AppError::Validation(
                "min_lon must be strictly less than max_lon".into(),
            ));
        }
        if self.min_lat >= self.max_lat {
            return Err(AppError::Validation(
                "min_lat must be strictly less than max_lat".into(),
            ));
        }
        Ok(())
    }

    /// Rejects boxes wider than `max_degrees` on either side, so one request cannot ask for the whole world.
    pub fn validate_max_span(&self, max_degrees: f64) -> Result<(), AppError> {
        if self.max_lon - self.min_lon > max_degrees || self.max_lat - self.min_lat > max_degrees {
            return Err(AppError::Validation(format!(
                "Bounding box too large: at most {max_degrees} degrees per side"
            )));
        }
        Ok(())
    }

    pub fn contains_point(&self, point: &GeoJsonPoint) -> bool {
        let lon = point.lon();
        let lat = point.lat();
        lon >= self.min_lon && lon <= self.max_lon && lat >= self.min_lat && lat <= self.max_lat
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HazardType {
    Warning,
    Blocking,
}

impl HazardType {
    /// The value stored in the database and used in the JSON contract.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Blocking => "blocking",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "warning" => Some(Self::Warning),
            "blocking" => Some(Self::Blocking),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HazardCategory {
    Glass,
    Pothole,
    Debris,
    RoadClosed,
    Construction,
    Flood,
}

impl HazardCategory {
    /// The value stored in the database and used in the JSON contract.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Glass => "glass",
            Self::Pothole => "pothole",
            Self::Debris => "debris",
            Self::RoadClosed => "road_closed",
            Self::Construction => "construction",
            Self::Flood => "flood",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "glass" => Some(Self::Glass),
            "pothole" => Some(Self::Pothole),
            "debris" => Some(Self::Debris),
            "road_closed" => Some(Self::RoadClosed),
            "construction" => Some(Self::Construction),
            "flood" => Some(Self::Flood),
            _ => None,
        }
    }

    pub const fn hazard_type(self) -> HazardType {
        match self {
            Self::Glass | Self::Pothole | Self::Debris => HazardType::Warning,
            Self::RoadClosed | Self::Construction | Self::Flood => HazardType::Blocking,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HazardStatus {
    Unconfirmed,
    Confirmed,
    Resolved,
}

impl HazardStatus {
    /// The value stored in the database and used in the JSON contract.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unconfirmed => "unconfirmed",
            Self::Confirmed => "confirmed",
            Self::Resolved => "resolved",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "unconfirmed" => Some(Self::Unconfirmed),
            "confirmed" => Some(Self::Confirmed),
            "resolved" => Some(Self::Resolved),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize_repr, Deserialize_repr, ToSchema)]
#[repr(i16)]
pub enum Vote {
    Up = 1,
    Down = -1,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Hazard {
    pub id: Uuid,
    pub category: HazardCategory,
    pub hazard_type: HazardType,
    pub status: HazardStatus,
    pub description: Option<String>,
    pub upvotes: i32,
    pub downvotes: i32,
    pub location: GeoJsonPoint,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateHazardRequest {
    pub category: HazardCategory,
    pub description: Option<String>,
    pub location: GeoJsonPoint,
}

/// Longest description, counted in characters (not bytes: accented text is multi-byte).
pub const MAX_DESCRIPTION_CHARS: usize = 500;

/// Characters that reorder surrounding text (Trojan Source style spoofing).
fn is_bidi_override(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Text that comes from third-party data (OpenStreetMap names and maneuver instructions) as it is shown to
/// people: control and bidirectional override characters never reach a screen, runs of whitespace become one
/// space, and the result is at most `max_chars` characters (not bytes) long.
pub fn clean_display_text(text: &str, max_chars: usize) -> String {
    let mut cleaned = String::with_capacity(text.len().min(max_chars * 4));
    let mut pending_space = false;
    let mut chars = 0;
    for c in text.chars().filter(|c| !is_bidi_override(*c)) {
        if c.is_control() || c.is_whitespace() {
            pending_space = chars > 0;
            continue;
        }
        if chars >= max_chars {
            break;
        }
        if pending_space {
            if chars + 1 >= max_chars {
                break;
            }
            cleaned.push(' ');
            chars += 1;
            pending_space = false;
        }
        cleaned.push(c);
        chars += 1;
    }
    cleaned
}

impl CreateHazardRequest {
    pub fn validate(&self) -> Result<(), AppError> {
        self.location.validate()?;
        if let Some(desc) = &self.description {
            if desc.chars().count() > MAX_DESCRIPTION_CHARS {
                return Err(AppError::Validation(format!(
                    "Description must not exceed {MAX_DESCRIPTION_CHARS} characters"
                )));
            }
            // NUL breaks PostgreSQL text columns; other control characters have no place in a short note.
            // A line break is the only one worth keeping.
            if desc.chars().any(|c| c.is_control() && c != '\n') {
                return Err(AppError::Validation(
                    "Description must not contain control characters".into(),
                ));
            }
            if desc.chars().any(is_bidi_override) {
                return Err(AppError::Validation(
                    "Description must not contain bidirectional text overrides".into(),
                ));
            }
        }
        Ok(())
    }

    /// The description as it is stored: surrounding whitespace removed, and `None` when nothing is left.
    pub fn sanitized_description(&self) -> Option<String> {
        self.description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_owned)
    }
}

/// Response to creating an anonymous account.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuthResponse {
    pub account_id: Uuid,
    /// Bearer token: `baze_v2.<payload>.<mac>`. Send it as `Authorization: Bearer <token>`.
    pub token: String,
    pub created_at: DateTime<Utc>,
    /// The token stops working at this instant; create a new account afterwards.
    pub expires_at: DateTime<Utc>,
}

/// An authenticated, active account. Authentication reads the account row anyway, so the
/// facts policies need (such as how old the account is) travel with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountContext {
    pub account_id: Uuid,
    pub created_at: DateTime<Utc>,
}

/// Anonymous accounts and the bearer tokens that identify them.
#[async_trait]
pub trait AccountService: Send + Sync {
    async fn create_anonymous_account(&self) -> Result<AuthResponse, AppError>;

    /// Resolves a bearer token to an active account, or fails with `Unauthorized`.
    async fn authenticate(&self, token: &str) -> Result<AccountContext, AppError>;
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HazardVoteRequest {
    /// 1 for upvote, -1 for downvote
    pub vote: Vote,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RouteRequest {
    pub origin: GeoJsonPoint,
    pub destination: GeoJsonPoint,
}

impl RouteRequest {
    pub fn validate(&self) -> Result<(), AppError> {
        self.origin.validate()?;
        self.destination.validate()?;
        if (self.origin.lon() - self.destination.lon()).abs() < f64::EPSILON
            && (self.origin.lat() - self.destination.lat()).abs() < f64::EPSILON
        {
            return Err(AppError::Validation(
                "Origin and destination cannot be identical".into(),
            ));
        }
        if haversine_meters(&self.origin, &self.destination) > MAX_ROUTE_DISTANCE_METERS {
            return Err(AppError::Validation(format!(
                "Origin and destination must be at most {} km apart",
                MAX_ROUTE_DISTANCE_METERS / 1000.0
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RouteManeuver {
    pub instruction: String,
    pub distance_meters: f64,
    pub time_seconds: f64,
    pub location: GeoJsonPoint,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RouteResponse {
    pub distance_meters: f64,
    pub duration_seconds: f64,
    pub ascent_meters: f64,
    pub descent_meters: f64,
    pub geometry: GeoJsonLineString,
    pub maneuvers: Vec<RouteManeuver>,
    pub nearby_hazards: Vec<Hazard>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, IntoParams)]
pub struct GeocodingQuery {
    pub q: String,
    #[schema(minimum = 1, maximum = 100)]
    #[param(minimum = 1, maximum = 100)]
    pub limit: Option<usize>,
}

impl GeocodingQuery {
    pub fn validate(&self) -> Result<(), AppError> {
        let trimmed = self.q.trim();
        if trimmed.is_empty() {
            return Err(AppError::Validation(
                "Query parameter 'q' must not be empty".into(),
            ));
        }
        if trimmed.len() > 200 {
            return Err(AppError::Validation(
                "Query parameter 'q' must not exceed 200 characters".into(),
            ));
        }
        if let Some(limit) = self.limit
            && !(1..=100).contains(&limit)
        {
            return Err(AppError::Validation(
                "Limit parameter must be between 1 and 100".into(),
            ));
        }
        Ok(())
    }

    pub fn sanitized_query(&self) -> &str {
        self.q.trim()
    }

    pub fn effective_limit(&self) -> usize {
        self.limit.unwrap_or(10)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GeocodingItem {
    pub name: String,
    pub street: Option<String>,
    pub city: Option<String>,
    pub country: Option<String>,
    pub location: GeoJsonPoint,
}

/// Community hazard reports: what the HTTP layer needs from the store behind them.
#[async_trait]
pub trait HazardStore: Send + Sync {
    /// Stores a new report. The reporter's own vote counts as the first one.
    async fn create_hazard(
        &self,
        account: &AccountContext,
        request: &CreateHazardRequest,
        client_ip: IpAddr,
    ) -> Result<Hazard, AppError>;

    /// Records or changes `account`'s vote. Unknown, expired and retired reports are `NotFound`.
    async fn vote_hazard(
        &self,
        hazard_id: Uuid,
        account: &AccountContext,
        vote: Vote,
        client_ip: IpAddr,
    ) -> Result<Hazard, AppError>;

    /// Active (not expired, not retired) reports inside `bbox`, newest first and capped.
    async fn list_active_hazards(&self, bbox: &BoundingBox) -> Result<Vec<Hazard>, AppError>;
}

#[async_trait]
pub trait CorridorHazards: Send + Sync {
    /// Finds confirmed blocking hazard polygons along the route corridor buffer.
    async fn find_blocking_polygons_along_corridor(
        &self,
        corridor: &GeoJsonLineString,
        buffer_meters: f64,
    ) -> Result<Vec<GeoJsonPolygon>, AppError>;

    /// Lists hazards (warning and blocking) along the route corridor buffer.
    async fn list_hazards_near_corridor(
        &self,
        corridor: &GeoJsonLineString,
        buffer_meters: f64,
    ) -> Result<Vec<Hazard>, AppError>;
}

pub trait HazardNotifier: Send + Sync {
    fn broadcast_hazard(&self, hazard: &Hazard);
}

/// Bicycle routing as the HTTP layer sees it.
#[async_trait]
pub trait RoutingProvider: Send + Sync {
    /// The best route from origin to destination that does not cross a confirmed closure. When no such
    /// route can be computed the answer is an error, never a route that crosses one.
    async fn route_bicycle(&self, request: &RouteRequest) -> Result<RouteResponse, AppError>;
}

#[async_trait]
pub trait GeocodingProvider: Send + Sync {
    async fn search_address(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<GeocodingItem>, AppError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_strings_match_the_json_contract_and_round_trip() {
        for category in [
            HazardCategory::Glass,
            HazardCategory::Pothole,
            HazardCategory::Debris,
            HazardCategory::RoadClosed,
            HazardCategory::Construction,
            HazardCategory::Flood,
        ] {
            let json = serde_json::to_value(category).unwrap();
            assert_eq!(json, category.as_str());
            assert_eq!(HazardCategory::from_db(category.as_str()), Some(category));
            assert_eq!(
                serde_json::to_value(category.hazard_type()).unwrap(),
                category.hazard_type().as_str()
            );
            assert_eq!(
                HazardType::from_db(category.hazard_type().as_str()),
                Some(category.hazard_type())
            );
        }
        for status in [
            HazardStatus::Unconfirmed,
            HazardStatus::Confirmed,
            HazardStatus::Resolved,
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), status.as_str());
            assert_eq!(HazardStatus::from_db(status.as_str()), Some(status));
        }
        assert_eq!(HazardCategory::from_db("Glass"), None);
        assert_eq!(HazardType::from_db(""), None);
        assert_eq!(HazardStatus::from_db("deleted"), None);
    }

    #[test]
    fn test_network_of_and_client_network() {
        let v4: IpAddr = "192.168.1.100".parse().unwrap();
        assert_eq!(network_of(v4, 32, 64).to_string(), "192.168.1.100/32");
        assert_eq!(network_of(v4, 24, 64).to_string(), "192.168.1.0/24");
        assert_eq!(client_network(v4).to_string(), "192.168.1.100/32");

        let v6: IpAddr = "2001:db8:85a3:0:1234:8a2e:370:7334".parse().unwrap();
        assert_eq!(network_of(v6, 32, 64).to_string(), "2001:db8:85a3::/64");
        assert_eq!(network_of(v6, 32, 48).to_string(), "2001:db8:85a3::/48");
        assert_eq!(client_network(v6).to_string(), "2001:db8:85a3::/64");
    }

    fn bbox(min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> BoundingBox {
        BoundingBox {
            min_lon,
            min_lat,
            max_lon,
            max_lat,
        }
    }

    #[test]
    fn bbox_span_within_limit_is_accepted() {
        let city = bbox(-70.7, -33.5, -70.5, -33.3);
        assert!(city.validate_max_span(MAX_LIST_BBOX_SPAN_DEGREES).is_ok());
        let edge = bbox(0.0, 0.0, 0.5, 0.5);
        assert!(edge.validate_max_span(0.5).is_ok());
    }

    #[test]
    fn bbox_span_over_limit_is_rejected_on_either_axis() {
        let world = bbox(-180.0, -90.0, 180.0, 90.0);
        assert!(
            world
                .validate_max_span(MAX_STREAM_BBOX_SPAN_DEGREES)
                .is_err()
        );
        let wide = bbox(0.0, 0.0, 0.6, 0.1);
        assert!(wide.validate_max_span(0.5).is_err());
        let tall = bbox(0.0, 0.0, 0.1, 0.6);
        assert!(tall.validate_max_span(0.5).is_err());
    }

    fn report(description: Option<&str>) -> CreateHazardRequest {
        CreateHazardRequest {
            category: HazardCategory::Glass,
            description: description.map(str::to_owned),
            location: GeoJsonPoint::new(-70.65, -33.45),
        }
    }

    #[test]
    fn description_limit_counts_characters_not_bytes() {
        // 500 two-byte characters are 1000 bytes and must be accepted; 501 characters must not.
        assert!(report(Some(&"ñ".repeat(500))).validate().is_ok());
        assert!(report(Some(&"ñ".repeat(501))).validate().is_err());
        assert!(report(Some(&"a".repeat(500))).validate().is_ok());
        assert!(report(Some(&"a".repeat(501))).validate().is_err());
        // Emoji are one character but four bytes.
        assert!(report(Some(&"🚧".repeat(500))).validate().is_ok());
    }

    #[test]
    fn description_rejects_control_characters_but_keeps_line_breaks() {
        for bad in [
            "\u{0}x",
            "a\u{1b}[31mb",
            "tab\there",
            "carriage\rreturn",
            "\u{7f}",
            "\u{85}",
        ] {
            assert!(report(Some(bad)).validate().is_err(), "{bad:?}");
        }
        assert!(report(Some("first line\nsecond line")).validate().is_ok());
    }

    #[test]
    fn description_rejects_bidirectional_overrides() {
        for bad in ["a\u{202e}b", "a\u{202a}b", "a\u{2066}b", "a\u{2069}b"] {
            assert!(report(Some(bad)).validate().is_err(), "{bad:?}");
        }
        // Plain right-to-left text is fine: only the invisible controls are refused.
        assert!(report(Some("שלום עולם")).validate().is_ok());
    }

    #[test]
    fn description_is_trimmed_and_empty_becomes_none() {
        assert_eq!(
            report(Some("  broken glass  "))
                .sanitized_description()
                .as_deref(),
            Some("broken glass")
        );
        assert_eq!(report(Some("   \n ")).sanitized_description(), None);
        assert_eq!(report(Some("")).sanitized_description(), None);
        assert_eq!(report(None).sanitized_description(), None);
    }

    #[test]
    fn absent_description_is_valid() {
        assert!(report(None).validate().is_ok());
    }

    fn route(origin: [f64; 2], destination: [f64; 2]) -> RouteRequest {
        RouteRequest {
            origin: GeoJsonPoint::new(origin[0], origin[1]),
            destination: GeoJsonPoint::new(destination[0], destination[1]),
        }
    }

    #[test]
    fn a_commute_is_a_valid_route_request() {
        assert!(route([-70.65, -33.45], [-70.55, -33.40]).validate().is_ok());
    }

    #[test]
    fn a_route_across_the_country_is_rejected() {
        // Santiago to Puerto Montt is ~850 km in a straight line.
        let error = route([-70.65, -33.45], [-72.94, -41.47])
            .validate()
            .unwrap_err();
        assert!(
            matches!(&error, AppError::Validation(m) if m.contains("150 km")),
            "{error}"
        );
    }

    #[test]
    fn the_distance_limit_is_measured_on_the_sphere() {
        // 1.3 degrees of latitude is ~144 km (accepted), 1.4 is ~156 km (refused): the limit applies
        // to the true distance, not to degrees.
        assert!(route([-70.0, -33.0], [-70.0, -34.3]).validate().is_ok());
        assert!(route([-70.0, -33.0], [-70.0, -34.4]).validate().is_err());
    }

    #[test]
    fn display_text_loses_control_and_bidi_characters_and_collapses_whitespace() {
        assert_eq!(
            clean_display_text("  Avenida \u{202e}Providencia\n\t1234  ", 100),
            "Avenida Providencia 1234"
        );
        assert_eq!(clean_display_text("a\u{0}b\u{1b}[31mc", 100), "a b [31mc");
        assert_eq!(clean_display_text("\n\n", 100), "");
        assert_eq!(clean_display_text("", 100), "");
    }

    #[test]
    fn display_text_is_cut_by_characters_not_bytes() {
        assert_eq!(
            clean_display_text(&"ñ".repeat(300), 200).chars().count(),
            200
        );
        assert_eq!(clean_display_text("abc def", 5), "abc d");
        // A cut never leaves a trailing space.
        assert_eq!(clean_display_text("abc def", 4), "abc");
        assert_eq!(clean_display_text("abcdef", 0), "");
    }
}
