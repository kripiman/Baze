// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What stands in for Valhalla and Photon when `ENGINES_ENABLED` is off.
//!
//! A server without engines (for instance one that only keeps the hazard reports) answers routing and
//! search with 501 and says why. Inventing a route or an empty result would be worse: a cyclist would
//! follow the first, and the second hides that the feature is not there.

use async_trait::async_trait;
use shared::{
    AppError, GeocodingItem, GeocodingProvider, RouteRequest, RouteResponse, RoutingProvider,
};

pub struct EnginesDisabled;

#[async_trait]
impl RoutingProvider for EnginesDisabled {
    async fn route_bicycle(&self, _request: &RouteRequest) -> Result<RouteResponse, AppError> {
        Err(AppError::NotImplemented(
            "Bicycle routing is not enabled on this server".into(),
        ))
    }
}

#[async_trait]
impl GeocodingProvider for EnginesDisabled {
    async fn search_address(
        &self,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<GeocodingItem>, AppError> {
        Err(AppError::NotImplemented(
            "Address search is not enabled on this server".into(),
        ))
    }
}
