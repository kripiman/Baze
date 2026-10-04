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

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Rate limit exceeded: {0}")]
    RateLimited(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),

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

impl CreateHazardRequest {
    pub fn validate(&self) -> Result<(), AppError> {
        self.location.validate()?;
        if let Some(desc) = &self.description
            && desc.len() > 500
        {
            return Err(AppError::Validation(
                "Description must not exceed 500 characters".into(),
            ));
        }
        Ok(())
    }
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
}
