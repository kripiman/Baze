// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use reqwest::Client;
use shared::{
    AppError, GeoJsonLineString, GeoJsonPolygon, HazardBlockingReader, HazardReader, RouteManeuver,
    RouteRequest, RouteResponse, RoutingProvider,
};
use std::sync::Arc;

#[allow(dead_code)]
pub struct ValhallaRoutingService {
    http_client: Client,
    valhalla_url: String,
    blocking_reader: Arc<dyn HazardBlockingReader>,
    hazard_reader: Arc<dyn HazardReader>,
}

impl ValhallaRoutingService {
    pub fn new(
        valhalla_url: String,
        blocking_reader: Arc<dyn HazardBlockingReader>,
        hazard_reader: Arc<dyn HazardReader>,
    ) -> Self {
        Self {
            http_client: Client::new(),
            valhalla_url,
            blocking_reader,
            hazard_reader,
        }
    }

    async fn request_valhalla_route(
        &self,
        _req: &RouteRequest,
        _exclude_polygons: &[GeoJsonPolygon],
    ) -> Result<(GeoJsonLineString, f64, f64, f64, f64, Vec<RouteManeuver>), AppError> {
        // TODO(verify): Implementar llamada HTTP POST a {valhalla_url}/route con payload:
        // {
        //   "locations": [{"lat": req.origin.lat(), "lon": req.origin.lon()}, ...],
        //   "costing": "bicycle",
        //   "costing_options": { "bicycle": { "bicycle_type": "Road", "use_roads": 0.2 } },
        //   "exclude_polygons": [...] (si hay polígonos)
        // }
        let placeholder_line = GeoJsonLineString {
            geom_type: "LineString".to_string(),
            coordinates: vec![[0.0, 0.0], [0.001, 0.001]],
        };
        Ok((placeholder_line, 1500.0, 300.0, 15.0, 10.0, Vec::new()))
    }
}

#[async_trait]
impl RoutingProvider for ValhallaRoutingService {
    async fn route_bicycle(&self, req: &RouteRequest) -> Result<RouteResponse, AppError> {
        tracing::debug!(
            origin = ?req.origin.coordinates,
            dest = ?req.destination.coordinates,
            "Calculating bicycle route"
        );

        // 1. Solicitar ruta inicial a Valhalla sin exclusiones
        let (initial_geometry, distance, duration, ascent, descent, maneuvers) =
            self.request_valhalla_route(req, &[]).await?;

        // 2. Cruce espacial en PostGIS: buscar bloqueos confirmados a lo largo del corredor de la ruta (15 metros)
        let blocking_polygons = self
            .blocking_reader
            .find_blocking_polygons_along_corridor(&initial_geometry, 15.0)
            .await?;

        // 3. Si hay bloqueos confirmados en la trayectoria, re-calcular con exclude_polygons
        let (final_geometry, final_distance, final_duration, final_ascent, final_descent, final_maneuvers) =
            if !blocking_polygons.is_empty() {
                tracing::info!(
                    count = blocking_polygons.len(),
                    "Confirmed blocking hazards intersected: recalculating with exclude_polygons"
                );
                self.request_valhalla_route(req, &blocking_polygons).await?
            } else {
                (initial_geometry, distance, duration, ascent, descent, maneuvers)
            };

        // 4. Obtener peligros (warning y blocking) cercanos al corredor para alertas en el dispositivo (50 metros)
        let nearby_hazards = self
            .hazard_reader
            .list_hazards_near_corridor(&final_geometry, 50.0)
            .await?;

        Ok(RouteResponse {
            distance_meters: final_distance,
            duration_seconds: final_duration,
            ascent_meters: final_ascent,
            descent_meters: final_descent,
            geometry: final_geometry,
            maneuvers: final_maneuvers,
            nearby_hazards,
        })
    }
}
