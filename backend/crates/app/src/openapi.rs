// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use auth::AuthResponse;
use serde::Serialize;
use shared::{
    BoundingBox, CreateHazardRequest, GeoJsonLineString, GeoJsonPoint, GeoJsonPolygon,
    GeocodingItem, GeocodingQuery, Hazard, HazardCategory, HazardStatus, HazardType,
    HazardVoteRequest, RouteManeuver, RouteRequest, RouteResponse, Vote,
};
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi, ToSchema};

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Serialize, ToSchema)]
pub struct SourceResponse {
    pub repository: String,
    pub commit: String,
    pub license: String,
}

pub struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("Token")
                        .description(Some(
                            "Anonymous bearer token in format: baze_anon_<uuid>.<sig>",
                        ))
                        .build(),
                ),
            );
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Baze API",
        version = "0.1.0",
        description = "Collaborative Bicycle Navigation REST & Realtime API",
        contact(name = "Gabriel Piñones"),
        license(
            name = "AGPL-3.0-or-later",
            url = "https://www.gnu.org/licenses/agpl-3.0.html"
        )
    ),
    paths(
        crate::router::health_handler,
        crate::router::source_handler,
        crate::router::anonymous_auth_handler,
        crate::router::list_hazards_handler,
        crate::router::create_hazard_handler,
        crate::router::vote_hazard_handler,
        crate::router::route_handler,
        crate::router::geocoding_handler,
        crate::router::realtime_sse_handler,
    ),
    components(
        schemas(
            HealthResponse,
            SourceResponse,
            AuthResponse,
            Hazard,
            HazardCategory,
            HazardType,
            HazardStatus,
            Vote,
            CreateHazardRequest,
            HazardVoteRequest,
            RouteRequest,
            RouteResponse,
            RouteManeuver,
            GeocodingItem,
            GeocodingQuery,
            GeoJsonPoint,
            GeoJsonLineString,
            GeoJsonPolygon,
            BoundingBox,
        )
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "system", description = "System health and AGPL source distribution"),
        (name = "auth", description = "Anonymous account provisioning"),
        (name = "hazards", description = "Hazard reporting and voting"),
        (name = "routing", description = "Bicycle routing with confirmed hazard avoidance"),
        (name = "geocoding", description = "Address search via Photon proxy"),
        (name = "realtime", description = "Realtime SSE stream of road hazards")
    )
)]
pub struct ApiDoc;
