// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

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
            return Err(AppError::Validation("Coordinates must be finite numbers".into()));
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GeoJsonBbox {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

impl GeoJsonBbox {
    pub fn validate(&self) -> Result<(), AppError> {
        if !self.min_lon.is_finite()
            || !self.min_lat.is_finite()
            || !self.max_lon.is_finite()
            || !self.max_lat.is_finite()
        {
            return Err(AppError::Validation("Bbox bounds must be finite numbers".into()));
        }
        if !(-180.0..=180.0).contains(&self.min_lon) || !(-180.0..=180.0).contains(&self.max_lon) {
            return Err(AppError::Validation("Bbox longitude out of range [-180, 180]".into()));
        }
        if !(-90.0..=90.0).contains(&self.min_lat) || !(-90.0..=90.0).contains(&self.max_lat) {
            return Err(AppError::Validation("Bbox latitude out of range [-90, 90]".into()));
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
pub enum HazardStatus {
    Unconfirmed,
    Confirmed,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Hazard {
    pub id: Uuid,
    pub creator_account_id: Uuid,
    pub category: String,
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
    pub category: String,
    pub hazard_type: HazardType,
    pub description: Option<String>,
    pub location: GeoJsonPoint,
}

pub const ALLOWED_HAZARD_CATEGORIES: &[&str] = &[
    "glass",
    "pothole",
    "debris",
    "road_closed",
    "construction",
    "flood",
];

impl CreateHazardRequest {
    pub fn validate(&self) -> Result<(), AppError> {
        self.location.validate()?;
        if !ALLOWED_HAZARD_CATEGORIES.contains(&self.category.as_str()) {
            return Err(AppError::Validation(format!(
                "Category '{}' is not supported. Allowed categories: {:?}",
                self.category, ALLOWED_HAZARD_CATEGORIES
            )));
        }
        if let Some(ref desc) = self.description {
            if desc.len() > 500 {
                return Err(AppError::Validation(
                    "Description must not exceed 500 characters".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HazardVoteRequest {
    /// 1 for upvote, -1 for downvote
    pub vote_type: i16,
}

impl HazardVoteRequest {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.vote_type != 1 && self.vote_type != -1 {
            return Err(AppError::Validation("Vote type must be 1 (upvote) or -1 (downvote)".into()));
        }
        Ok(())
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GeocodingItem {
    pub name: String,
    pub street: Option<String>,
    pub city: Option<String>,
    pub country: Option<String>,
    pub location: GeoJsonPoint,
}

#[async_trait]
pub trait HazardBlockingReader: Send + Sync {
    /// Finds confirmed blocking hazard polygons along the route corridor buffer.
    async fn find_blocking_polygons_along_corridor(
        &self,
        corridor: &GeoJsonLineString,
        buffer_meters: f64,
    ) -> Result<Vec<GeoJsonPolygon>, AppError>;
}

#[async_trait]
pub trait HazardReader: Send + Sync {
    async fn list_active_hazards(&self, bbox: &GeoJsonBbox) -> Result<Vec<Hazard>, AppError>;
    async fn list_hazards_near_corridor(
        &self,
        corridor: &GeoJsonLineString,
        buffer_meters: f64,
    ) -> Result<Vec<Hazard>, AppError>;
}

#[async_trait]
pub trait HazardWriter: Send + Sync {
    async fn create_hazard(
        &self,
        account_id: Uuid,
        req: &CreateHazardRequest,
    ) -> Result<Hazard, AppError>;
    async fn vote_hazard(
        &self,
        hazard_id: Uuid,
        account_id: Uuid,
        vote_type: i16,
    ) -> Result<Hazard, AppError>;
}

#[async_trait]
pub trait HazardNotifier: Send + Sync {
    async fn broadcast_hazard(&self, hazard: &Hazard) -> Result<(), AppError>;
}

#[async_trait]
pub trait GeocodingProvider: Send + Sync {
    async fn search_address(&self, query: &str, limit: usize) -> Result<Vec<GeocodingItem>, AppError>;
}

#[async_trait]
pub trait RoutingProvider: Send + Sync {
    async fn route_bicycle(&self, req: &RouteRequest) -> Result<RouteResponse, AppError>;
}
