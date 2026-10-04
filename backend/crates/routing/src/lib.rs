// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use reqwest::Client;
use shared::{
    AppError, CorridorHazards, GeoJsonLineString, GeoJsonPolygon, RouteManeuver, RouteRequest,
    RouteResponse,
};
use std::sync::Arc;

struct ValhallaCalculatedRoute {
    geometry: GeoJsonLineString,
    distance_meters: f64,
    duration_seconds: f64,
    ascent_meters: f64,
    descent_meters: f64,
    maneuvers: Vec<RouteManeuver>,
}

pub struct ValhallaRoutingService {
    #[allow(dead_code)]
    http_client: Client,
    #[allow(dead_code)]
    valhalla_url: String,
    hazards: Arc<dyn CorridorHazards>,
}

impl ValhallaRoutingService {
    pub fn new(valhalla_url: String, hazards: Arc<dyn CorridorHazards>) -> Self {
        Self {
            http_client: Client::new(),
            valhalla_url,
            hazards,
        }
    }

    async fn request_valhalla_route(
        &self,
        _req: &RouteRequest,
        _exclude_polygons: &[GeoJsonPolygon],
    ) -> Result<ValhallaCalculatedRoute, AppError> {
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
        Ok(ValhallaCalculatedRoute {
            geometry: placeholder_line,
            distance_meters: 1500.0,
            duration_seconds: 300.0,
            ascent_meters: 15.0,
            descent_meters: 10.0,
            maneuvers: Vec::new(),
        })
    }

    pub async fn route_bicycle(&self, req: &RouteRequest) -> Result<RouteResponse, AppError> {
        tracing::debug!(
            origin = ?req.origin.coordinates,
            dest = ?req.destination.coordinates,
            "Calculating bicycle route"
        );

        // 1. Solicitar ruta inicial a Valhalla sin exclusiones
        let mut route = self.request_valhalla_route(req, &[]).await?;

        // 2. Cruce espacial en PostGIS: buscar bloqueos confirmados a lo largo del corredor de la ruta (15 metros)
        let blocking_polygons = self
            .hazards
            .find_blocking_polygons_along_corridor(&route.geometry, 15.0)
            .await?;

        // 3. Si hay bloqueos confirmados en la trayectoria, re-calcular con exclude_polygons
        if !blocking_polygons.is_empty() {
            tracing::info!(
                count = blocking_polygons.len(),
                "Confirmed blocking hazards intersected: recalculating with exclude_polygons"
            );
            route = self.request_valhalla_route(req, &blocking_polygons).await?;
        }

        // 4. Obtener peligros (warning y blocking) cercanos al corredor para alertas en el dispositivo (50 metros)
        let nearby_hazards = self
            .hazards
            .list_hazards_near_corridor(&route.geometry, 50.0)
            .await?;

        Ok(RouteResponse {
            distance_meters: route.distance_meters,
            duration_seconds: route.duration_seconds,
            ascent_meters: route.ascent_meters,
            descent_meters: route.descent_meters,
            geometry: route.geometry,
            maneuvers: route.maneuvers,
            nearby_hazards,
        })
    }
}
