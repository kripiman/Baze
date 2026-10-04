// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use reqwest::Client;
use shared::{AppError, GeocodingItem, GeocodingProvider};
use std::time::Duration;

#[allow(dead_code)]
pub struct PhotonGeocodingService {
    http_client: Client,
    photon_url: String,
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
        Ok(Self {
            http_client,
            photon_url,
        })
    }
}

#[async_trait]
impl GeocodingProvider for PhotonGeocodingService {
    async fn search_address(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<GeocodingItem>, AppError> {
        // TODO(verify): Implementar llamada GET a {photon_url}/api con
        //   `.query(&[("q", query), ("limit", &limit.to_string())])` (nunca interpolar `query`
        //   en la URL: permitiría inyectar parámetros) y mapear la respuesta GeoJSON
        //   FeatureCollection a GeocodingItem. No registrar el texto de búsqueda: es un dato personal.
        let _ = (query, limit);
        Err(AppError::NotImplemented(
            "Address search is not available yet".into(),
        ))
    }
}
