// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pure geometry helpers shared by the domain crates: no I/O, nothing but arithmetic.

use crate::GeoJsonPoint;

const EARTH_RADIUS_METERS: f64 = 6_371_008.8;
const METERS_PER_DEGREE: f64 = EARTH_RADIUS_METERS * std::f64::consts::PI / 180.0;

/// Great-circle distance between two points, in meters.
pub fn haversine_meters(a: &GeoJsonPoint, b: &GeoJsonPoint) -> f64 {
    let (lat_a, lat_b) = (a.lat().to_radians(), b.lat().to_radians());
    let delta_lat = lat_b - lat_a;
    let delta_lon = (b.lon() - a.lon()).to_radians();
    let h = (delta_lat / 2.0).sin().powi(2)
        + lat_a.cos() * lat_b.cos() * (delta_lon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_METERS * h.clamp(0.0, 1.0).sqrt().asin()
}

/// Distance from `p` to the segment `a`-`b` in a flat plane.
fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length_sq = dx * dx + dy * dy;
    let t = if length_sq == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length_sq).clamp(0.0, 1.0)
    };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// Douglas-Peucker simplification of a `[lon, lat]` polyline.
///
/// The first and last points are always kept and every dropped point lies within `tolerance_meters` of
/// the result, so a query made with a buffer widened by the tolerance cannot miss anything the original
/// line would have found. Distances are measured on a local equirectangular projection, which is exact
/// enough for the metres-scale tolerances used here; the line must not cross the antimeridian.
///
/// Iterative on purpose: a route can have tens of thousands of points and recursion could overflow the
/// stack. Points that are not finite are dropped (they never count as far from anything).
pub fn simplify_line(points: &[[f64; 2]], tolerance_meters: f64) -> Vec<[f64; 2]> {
    let count = points.len();
    if count <= 2 || tolerance_meters.is_nan() || tolerance_meters <= 0.0 {
        return points.to_vec();
    }
    let mean_lat = points.iter().map(|p| p[1]).sum::<f64>() / count as f64;
    let x_scale = METERS_PER_DEGREE * mean_lat.to_radians().cos();
    let projected: Vec<(f64, f64)> = points
        .iter()
        .map(|p| (p[0] * x_scale, p[1] * METERS_PER_DEGREE))
        .collect();

    let mut keep = vec![false; count];
    keep[0] = true;
    keep[count - 1] = true;
    let mut pending = vec![(0usize, count - 1)];
    while let Some((first, last)) = pending.pop() {
        if last <= first + 1 {
            continue;
        }
        let (mut farthest, mut farthest_distance) = (first, 0.0_f64);
        for index in first + 1..last {
            let distance = segment_distance(projected[index], projected[first], projected[last]);
            if distance > farthest_distance {
                farthest = index;
                farthest_distance = distance;
            }
        }
        if farthest_distance > tolerance_meters {
            keep[farthest] = true;
            pending.push((first, farthest));
            pending.push((farthest, last));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(point, kept)| kept.then_some(*point))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(lon: f64, lat: f64) -> GeoJsonPoint {
        GeoJsonPoint::new(lon, lat)
    }

    #[test]
    fn a_degree_of_latitude_is_about_111_kilometers() {
        let meters = haversine_meters(&point(-70.0, -33.0), &point(-70.0, -32.0));
        assert!((meters - 111_195.0).abs() < 50.0, "{meters}");
    }

    #[test]
    fn distance_is_symmetric_and_zero_for_the_same_point() {
        let (santiago, valparaiso) = (point(-70.6693, -33.4489), point(-71.6127, -33.0472));
        let there = haversine_meters(&santiago, &valparaiso);
        assert_eq!(there, haversine_meters(&valparaiso, &santiago));
        assert!((95_000.0..105_000.0).contains(&there), "{there}");
        assert_eq!(haversine_meters(&santiago, &santiago), 0.0);
    }

    #[test]
    fn longitude_shrinks_with_latitude() {
        let at_equator = haversine_meters(&point(0.0, 0.0), &point(1.0, 0.0));
        let at_60 = haversine_meters(&point(0.0, 60.0), &point(1.0, 60.0));
        assert!(
            (at_60 / at_equator - 0.5).abs() < 0.01,
            "{at_60} {at_equator}"
        );
    }

    #[test]
    fn short_lines_and_a_useless_tolerance_are_returned_unchanged() {
        let one = [[-70.0, -33.0]];
        let two = [[-70.0, -33.0], [-70.1, -33.1]];
        assert_eq!(simplify_line(&one, 5.0), one);
        assert_eq!(simplify_line(&two, 5.0), two);
        let three = [[-70.0, -33.0], [-70.05, -33.02], [-70.1, -33.1]];
        assert_eq!(simplify_line(&three, 0.0), three);
        assert_eq!(simplify_line(&three, -1.0), three);
        assert_eq!(simplify_line(&three, f64::NAN), three);
    }

    #[test]
    fn collinear_points_collapse_to_the_endpoints() {
        let line: Vec<[f64; 2]> = (0..=100)
            .map(|i| [-70.0 + f64::from(i) * 0.001, -33.0])
            .collect();
        assert_eq!(simplify_line(&line, 1.0), vec![line[0], line[100]]);
    }

    #[test]
    fn a_corner_wider_than_the_tolerance_survives() {
        // An L: east for ~1 km, then north for ~1 km. The corner is ~700 m from the chord.
        let line = vec![[-70.0, -33.0], [-69.99, -33.0], [-69.99, -32.99]];
        assert_eq!(simplify_line(&line, 5.0), line);
    }

    #[test]
    fn a_wiggle_smaller_than_the_tolerance_is_dropped() {
        // A ~2 m bump on a 1 km line.
        let bump = 2.0 / METERS_PER_DEGREE;
        let line = vec![[-70.0, -33.0], [-69.995, -33.0 + bump], [-69.99, -33.0]];
        assert_eq!(simplify_line(&line, 5.0).len(), 2);
        assert_eq!(simplify_line(&line, 1.0).len(), 3);
    }

    #[test]
    fn a_very_long_route_is_simplified_within_the_tolerance() {
        // 20 000 points (one every ~5 m) on a gentle S-curve: more than the corridor query accepts.
        let line: Vec<[f64; 2]> = (0..20_000)
            .map(|i| {
                let t = f64::from(i);
                [-70.65 + t * 0.00005, -33.45 + (t * 0.0005).sin() * 0.002]
            })
            .collect();
        let tolerance = 2.0;

        let simplified = simplify_line(&line, tolerance);

        assert!(simplified.len() < 10_000, "{}", simplified.len());
        assert!(simplified.len() < line.len() / 4, "{}", simplified.len());
        assert_eq!(simplified.first(), line.first());
        assert_eq!(simplified.last(), line.last());

        // Every original point stays within the tolerance of the simplified line.
        let mean_lat = line.iter().map(|p| p[1]).sum::<f64>() / line.len() as f64;
        let x_scale = METERS_PER_DEGREE * mean_lat.to_radians().cos();
        let project = |p: &[f64; 2]| (p[0] * x_scale, p[1] * METERS_PER_DEGREE);
        let segments: Vec<_> = simplified
            .windows(2)
            .map(|w| (project(&w[0]), project(&w[1])))
            .collect();
        for original in line.iter().step_by(7) {
            let nearest = segments
                .iter()
                .map(|(a, b)| segment_distance(project(original), *a, *b))
                .fold(f64::INFINITY, f64::min);
            assert!(nearest <= tolerance * 1.001, "{nearest}");
        }
    }

    #[test]
    fn simplification_does_not_recurse_on_a_zigzag() {
        // Alternating points keep every one of them far from the chord: the worst case for recursion depth.
        let line: Vec<[f64; 2]> = (0..6_000)
            .map(|i| {
                let t = f64::from(i);
                [
                    -70.0 + t * 0.00001,
                    -33.0 + if i % 2 == 0 { 0.0 } else { 0.001 },
                ]
            })
            .collect();
        let simplified = simplify_line(&line, 1.0);
        assert_eq!(simplified.len(), line.len());
    }
}
