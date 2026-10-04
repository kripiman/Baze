// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Decoder for the encoded polylines Valhalla uses for route shapes (the Google algorithm with six
//! decimal digits). The engine writes latitude first; GeoJSON, and so the rest of Baze, wants
//! `[longitude, latitude]`.

const PRECISION: f64 = 1e6;

/// An encoded shape that cannot be a route: truncated, outside the alphabet or off the planet.
#[derive(Debug, PartialEq, Eq)]
pub struct PolylineError(&'static str);

impl std::fmt::Display for PolylineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid encoded polyline: {}", self.0)
    }
}

impl std::error::Error for PolylineError {}

/// Reads one zig-zag encoded signed integer starting at `*cursor`.
fn next_value(bytes: &[u8], cursor: &mut usize) -> Result<i64, PolylineError> {
    let mut result: i64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = *bytes
            .get(*cursor)
            .ok_or(PolylineError("ends in the middle of a value"))?;
        *cursor += 1;
        if !(63..=126).contains(&byte) {
            return Err(PolylineError("character outside the encoding alphabet"));
        }
        let chunk = i64::from(byte - 63);
        // Six digits of a whole planet need 32 bits; more than 35 means garbage, not a coordinate.
        if shift > 30 {
            return Err(PolylineError("value too large"));
        }
        result |= (chunk & 0x1f) << shift;
        shift += 5;
        if chunk & 0x20 == 0 {
            break;
        }
    }
    Ok(if result & 1 == 1 {
        !(result >> 1)
    } else {
        result >> 1
    })
}

/// Decodes `encoded` into `[lon, lat]` pairs. An empty string is an empty shape.
pub fn decode_polyline6(encoded: &str) -> Result<Vec<[f64; 2]>, PolylineError> {
    let bytes = encoded.as_bytes();
    let mut cursor = 0;
    let (mut lat, mut lon) = (0i64, 0i64);
    let mut points = Vec::with_capacity(bytes.len() / 4);
    while cursor < bytes.len() {
        lat += next_value(bytes, &mut cursor)?;
        lon += next_value(bytes, &mut cursor)?;
        let point = [lon as f64 / PRECISION, lat as f64 / PRECISION];
        if point[0].abs() > 180.0 || point[1].abs() > 90.0 {
            return Err(PolylineError("coordinate outside the planet"));
        }
        points.push(point);
    }
    Ok(points)
}

/// Inverse of [`decode_polyline6`], for building the engine's answers in tests.
#[cfg(test)]
pub(crate) fn encode_polyline6(points: &[[f64; 2]]) -> String {
    fn push(value: i64, out: &mut String) {
        let mut v = if value < 0 { !(value << 1) } else { value << 1 };
        while v >= 0x20 {
            out.push(char::from(((0x20 | (v & 0x1f)) + 63) as u8));
            v >>= 5;
        }
        out.push(char::from((v + 63) as u8));
    }
    let (mut previous_lat, mut previous_lon) = (0i64, 0i64);
    let mut out = String::new();
    for point in points {
        let lat = (point[1] * PRECISION).round() as i64;
        let lon = (point[0] * PRECISION).round() as i64;
        push(lat - previous_lat, &mut out);
        push(lon - previous_lon, &mut out);
        previous_lat = lat;
        previous_lon = lon;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_reference_example_of_the_algorithm() {
        // The worked example of the format's specification uses five digits; scaling it to six shows
        // the same bytes are read with the factor this decoder uses.
        let points = decode_polyline6("_p~iF~ps|U_ulLnnqC_mqNvxq`@").unwrap();
        let expected = [[-120.2, 38.5], [-120.95, 40.7], [-126.453, 43.252]];
        assert_eq!(points.len(), 3);
        for (decoded, want) in points.iter().zip(expected) {
            // Five-digit data read with a six-digit factor comes out ten times smaller.
            assert!((decoded[0] * 10.0 - want[0]).abs() < 1e-9, "{decoded:?}");
            assert!((decoded[1] * 10.0 - want[1]).abs() < 1e-9, "{decoded:?}");
        }
    }

    #[test]
    fn round_trips_a_route_with_six_digits_of_precision() {
        let route = vec![
            [-70.650123, -33.450456],
            [-70.649001, -33.451999],
            [-70.640000, -33.460000],
            [-70.640001, -33.459999],
            [-71.55, -33.02],
        ];
        let decoded = decode_polyline6(&encode_polyline6(&route)).unwrap();
        assert_eq!(decoded.len(), route.len());
        for (got, want) in decoded.iter().zip(&route) {
            assert!((got[0] - want[0]).abs() < 1e-9, "{got:?} {want:?}");
            assert!((got[1] - want[1]).abs() < 1e-9, "{got:?} {want:?}");
        }
    }

    #[test]
    fn handles_both_hemispheres_and_negative_steps() {
        let route = vec![
            [10.5, 51.25],
            [10.4, 51.2],
            [-0.000001, -0.000001],
            [0.0, 0.0],
        ];
        let decoded = decode_polyline6(&encode_polyline6(&route)).unwrap();
        for (got, want) in decoded.iter().zip(&route) {
            assert!((got[0] - want[0]).abs() < 1e-9 && (got[1] - want[1]).abs() < 1e-9);
        }
    }

    #[test]
    fn an_empty_string_is_an_empty_shape() {
        assert_eq!(decode_polyline6("").unwrap(), Vec::<[f64; 2]>::new());
    }

    #[test]
    fn rejects_truncated_and_foreign_input() {
        let full = encode_polyline6(&[[-70.65, -33.45], [-70.64, -33.44]]);
        // Cut in the middle of the second point's longitude.
        assert!(decode_polyline6(&full[..full.len() - 1]).is_err());
        // A latitude without its longitude.
        assert!(decode_polyline6("_p~iF").is_err());
        for bad in ["hello world", "abc\u{7f}", "ñ", "\n", "!!"] {
            assert!(decode_polyline6(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn rejects_values_that_cannot_be_coordinates() {
        // A long run of continuation characters is not a number.
        assert!(decode_polyline6(&"~".repeat(40)).is_err());
        // Latitude 100 degrees does not exist.
        assert!(decode_polyline6(&encode_polyline6(&[[0.0, 100.0]])).is_err());
        assert!(decode_polyline6(&encode_polyline6(&[[200.0, 0.0]])).is_err());
    }
}
