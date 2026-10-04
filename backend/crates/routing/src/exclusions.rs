// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The closures a route must avoid, in the form the engine takes them.

use shared::{AppError, GeoJsonPolygon};

/// Most polygons one request may carry. Valhalla bounds the total perimeter of its exclusions
/// (`service_limits.max_exclude_polygons_length`); a closure is a ~30 m radius circle of ~190 m of
/// perimeter, so the default limit holds a little over fifty of them. Asking for more would make the
/// engine refuse the whole request, so beyond this the route is refused here with a clear reason.
pub const MAX_EXCLUDE_POLYGONS: usize = 50;

/// The exterior rings of `polygons`, which is all `exclude_polygons` takes: nested `[lon, lat]` arrays.
/// Holes are ignored (a closure is a solid shape) and rings the engine could not use are refused.
pub fn exclusion_rings(polygons: &[GeoJsonPolygon]) -> Result<Vec<&[[f64; 2]]>, AppError> {
    polygons
        .iter()
        .map(|polygon| {
            let ring = polygon
                .coordinates
                .first()
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let valid = ring.len() >= 4
                && ring.iter().all(|[lon, lat]| {
                    lon.is_finite() && lat.is_finite() && lon.abs() <= 180.0 && lat.abs() <= 90.0
                });
            if valid {
                Ok(ring)
            } else {
                Err(AppError::Internal(
                    "A blocking polygon from the store is not a usable ring".into(),
                ))
            }
        })
        .collect()
}

/// Whether `polygon` is already one of `list` (same exterior ring, vertex by vertex).
pub fn contains_polygon(list: &[GeoJsonPolygon], polygon: &GeoJsonPolygon) -> bool {
    list.iter()
        .any(|other| other.coordinates == polygon.coordinates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::square;

    #[test]
    fn takes_the_exterior_ring_of_each_polygon() {
        let mut with_hole = square(-70.65, -33.45, 0.001);
        with_hole.coordinates.push(vec![[0.0, 0.0]; 4]);
        let polygons = [with_hole, square(-70.6, -33.4, 0.001)];

        let rings = exclusion_rings(&polygons).unwrap();

        assert_eq!(rings.len(), 2);
        assert_eq!(rings[0], polygons[0].coordinates[0].as_slice());
        assert_eq!(rings[1], polygons[1].coordinates[0].as_slice());
    }

    #[test]
    fn no_polygons_means_no_rings() {
        assert!(exclusion_rings(&[]).unwrap().is_empty());
    }

    #[test]
    fn refuses_rings_the_engine_could_not_use() {
        let empty = GeoJsonPolygon {
            geom_type: "Polygon".into(),
            coordinates: vec![],
        };
        let too_short = GeoJsonPolygon {
            geom_type: "Polygon".into(),
            coordinates: vec![vec![[0.0, 0.0], [1.0, 1.0], [0.0, 0.0]]],
        };
        let mut off_planet = square(-70.65, -33.45, 0.001);
        off_planet.coordinates[0][1][1] = 95.0;
        let mut not_a_number = square(-70.65, -33.45, 0.001);
        not_a_number.coordinates[0][2][0] = f64::NAN;
        for polygon in [empty, too_short, off_planet, not_a_number] {
            assert!(exclusion_rings(&[polygon]).is_err());
        }
    }

    #[test]
    fn recognises_a_polygon_it_already_has() {
        let a = square(-70.65, -33.45, 0.001);
        let list = vec![a.clone(), square(-70.6, -33.4, 0.001)];
        assert!(contains_polygon(&list, &a));
        assert!(!contains_polygon(&list, &square(-70.5, -33.3, 0.001)));
        assert!(!contains_polygon(&[], &a));
    }
}
