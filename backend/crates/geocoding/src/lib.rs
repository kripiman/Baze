// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use reqwest::Client;
use shared::{AppError, GeocodingItem, GeocodingProvider};

#[allow(dead_code)]
pub struct PhotonGeocodingService {
    http_client: Client,
    photon_url: String,
}

impl PhotonGeocodingService {
    pub fn new(photon_url: String) -> Self {
        Self {
            http_client: Client::new(),
            photon_url,
        }
    }
}

#[async_trait]
impl GeocodingProvider for PhotonGeocodingService {
    async fn search_address(&self, query: &str, limit: usize) -> Result<Vec<GeocodingItem>, AppError> {
        // TODO(verify): Implementar llamada GET a {photon_url}/api?q={query}&limit={limit}
        // y mapear la respuesta GeoJSON FeatureCollection a GeocodingItem
        tracing::debug!(query = %query, limit = limit, "Forwarding geocoding query to Photon");

        Ok(Vec::new())
    }
}
