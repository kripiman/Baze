// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bicycle routing on Valhalla with avoidance of confirmed closures (ADR-0002).
//!
//! The rule is fail-closed: a route that crosses a confirmed closure is never returned as if it were
//! fine. The base route is checked against the closures stored in PostGIS; when it crosses any, the
//! route is asked again with those areas excluded, and checked again, a few times at most. When no
//! route clear of them can be established the answer is an error.

mod exclusions;
mod polyline;
mod valhalla;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

pub use exclusions::MAX_EXCLUDE_POLYGONS;

use async_trait::async_trait;
use exclusions::contains_polygon;
use shared::{
    AppError, CorridorHazards, GeoJsonLineString, GeoJsonPolygon, MAX_CORRIDOR_POINTS,
    RouteRequest, RouteResponse, RoutingProvider, simplify_line,
};
use std::sync::Arc;
use valhalla::{EngineError, EngineRoute, ValhallaClient};

/// How close (meters) to the route a confirmed closure must be to count as on it.
const CLOSURE_CORRIDOR_METERS: f64 = 15.0;
/// How close (meters) a report must be to the route to be sent along as an alert.
const ALERT_CORRIDOR_METERS: f64 = 50.0;
/// Times the engine is asked again after the first route. The first answer plus these is the most
/// engine calls one request can cost.
const MAX_REROUTES: usize = 3;
/// Tolerance (meters) of the first simplification of the route before it is sent to the database; it is
/// doubled while the line still has too many points.
const FIRST_SIMPLIFICATION_METERS: f64 = 2.0;
const LAST_SIMPLIFICATION_METERS: f64 = 16.0;

pub struct ValhallaRoutingService {
    engine: ValhallaClient,
    hazards: Arc<dyn CorridorHazards>,
}

impl ValhallaRoutingService {
    pub fn new(valhalla_url: String, hazards: Arc<dyn CorridorHazards>) -> Result<Self, AppError> {
        Ok(Self {
            engine: ValhallaClient::new(&valhalla_url)?,
            hazards,
        })
    }

    #[cfg(test)]
    fn with_engine(engine: ValhallaClient, hazards: Arc<dyn CorridorHazards>) -> Self {
        Self { engine, hazards }
    }

    /// One engine call, with the engine's failures turned into the answers the API gives.
    async fn engine_route(
        &self,
        request: &RouteRequest,
        exclude: &[GeoJsonPolygon],
    ) -> Result<EngineRoute, AppError> {
        match self
            .engine
            .route(&request.origin, &request.destination, exclude)
            .await
        {
            Ok(route) => Ok(route),
            Err(EngineError::NoRoute) if exclude.is_empty() => Err(AppError::NotFound(
                "No bicycle route was found between these points".into(),
            )),
            Err(EngineError::NoRoute) => Err(no_route_clear_of_closures()),
            // With exclusions in the request a refusal is most likely the engine's limit on how much
            // can be excluded; without them it is a bug or a misconfiguration.
            Err(EngineError::Rejected(_)) if !exclude.is_empty() => Err(AppError::Unavailable(
                "Routes that avoid the confirmed closures cannot be computed right now".into(),
            )),
            Err(EngineError::Rejected(code)) => Err(AppError::Upstream(format!(
                "Valhalla rejected the request (error {code})"
            ))),
            Err(EngineError::Failed(error)) => Err(error),
        }
    }
}

fn no_route_clear_of_closures() -> AppError {
    AppError::NotFound("No route avoids the confirmed closures between these points".into())
}

/// The route as it is sent to the hazard store, and how much wider than asked the search must be
/// because of the simplification (every dropped point is within that distance of the line sent).
struct Corridor {
    line: GeoJsonLineString,
    widening_meters: f64,
}

/// The store takes at most [`MAX_CORRIDOR_POINTS`] points and a long route has more. Simplify with a
/// small tolerance first and a larger one only if needed, and widen the search by the tolerance so no
/// closure within the corridor distance of the real route can be missed. A route that still does not
/// fit cannot be checked, so it is refused: the alternative would be to skip the check.
fn corridor_of(route: &GeoJsonLineString) -> Result<Corridor, AppError> {
    let mut tolerance = FIRST_SIMPLIFICATION_METERS;
    while tolerance <= LAST_SIMPLIFICATION_METERS {
        let points = simplify_line(&route.coordinates, tolerance);
        if points.len() <= MAX_CORRIDOR_POINTS {
            return Ok(Corridor {
                line: GeoJsonLineString {
                    geom_type: "LineString".into(),
                    coordinates: points,
                },
                widening_meters: tolerance,
            });
        }
        tolerance *= 2.0;
    }
    Err(AppError::Unavailable(
        "This route is too intricate to check against the confirmed closures".into(),
    ))
}

#[async_trait]
impl RoutingProvider for ValhallaRoutingService {
    async fn route_bicycle(&self, request: &RouteRequest) -> Result<RouteResponse, AppError> {
        let mut excluded: Vec<GeoJsonPolygon> = Vec::new();
        let mut route = self.engine_route(request, &excluded).await?;
        let mut reroutes = 0;

        let corridor = loop {
            let corridor = corridor_of(&route.geometry)?;
            let blocking = self
                .hazards
                .find_blocking_polygons_along_corridor(
                    &corridor.line,
                    CLOSURE_CORRIDOR_METERS + corridor.widening_meters,
                )
                .await?;
            if blocking.is_empty() {
                break corridor;
            }

            let fresh: Vec<GeoJsonPolygon> = blocking
                .into_iter()
                .filter(|polygon| !contains_polygon(&excluded, polygon))
                .collect();
            if fresh.is_empty() {
                // The engine was told to avoid these and its route still touches them (a closure at the
                // origin or the destination, for instance). That route must not be served.
                return Err(no_route_clear_of_closures());
            }
            if reroutes == MAX_REROUTES {
                return Err(AppError::Unavailable(
                    "Could not find a route clear of the confirmed closures".into(),
                ));
            }
            excluded.extend(fresh);
            if excluded.len() > MAX_EXCLUDE_POLYGONS {
                return Err(AppError::Unavailable(
                    "Too many confirmed closures along this route to avoid them all".into(),
                ));
            }

            reroutes += 1;
            tracing::info!(
                closures = excluded.len(),
                reroutes,
                "route recalculated to avoid confirmed closures"
            );
            route = self.engine_route(request, &excluded).await?;
        };

        let nearby_hazards = self
            .hazards
            .list_hazards_near_corridor(
                &corridor.line,
                ALERT_CORRIDOR_METERS + corridor.widening_meters,
            )
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
