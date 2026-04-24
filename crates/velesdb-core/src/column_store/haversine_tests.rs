//! Unit tests for Haversine distance computation and coordinate validation.

use super::haversine::{haversine_distance, validate_coordinates};

// =========================================================================
// Haversine distance tests
// =========================================================================

#[test]
fn test_haversine_identical_points_returns_zero() {
    let dist = haversine_distance(48.8566, 2.3522, 48.8566, 2.3522);
    assert!(
        dist.abs() < 1e-10,
        "Identical points should have distance 0, got {dist}"
    );
}

#[test]
fn test_haversine_paris_to_london() {
    // Paris (48.8566, 2.3522) to London (51.5074, -0.1278) ≈ 343 km
    let dist = haversine_distance(48.8566, 2.3522, 51.5074, -0.1278);
    let expected_km = 343.0;
    let dist_km = dist / 1000.0;
    assert!(
        (dist_km - expected_km).abs() < 10.0,
        "Paris-London should be ~343 km, got {dist_km:.1} km"
    );
}

#[test]
fn test_haversine_nyc_to_la() {
    // NYC (40.7128, -74.0060) to LA (34.0522, -118.2437) ≈ 3944 km
    let dist = haversine_distance(40.7128, -74.0060, 34.0522, -118.2437);
    let dist_km = dist / 1000.0;
    assert!(
        (dist_km - 3944.0).abs() < 50.0,
        "NYC-LA should be ~3944 km, got {dist_km:.1} km"
    );
}

#[test]
fn test_haversine_antipodal_points() {
    // North pole to south pole ≈ π × R ≈ 20015 km
    let dist = haversine_distance(90.0, 0.0, -90.0, 0.0);
    let expected = std::f64::consts::PI * 6_371_000.0;
    let tolerance = expected * 0.01; // 1% tolerance
    assert!(
        (dist - expected).abs() < tolerance,
        "Antipodal distance should be ~{:.0} m, got {dist:.0} m",
        expected
    );
}

#[test]
fn test_haversine_date_line_crossing() {
    // Points on either side of the date line
    let dist = haversine_distance(0.0, 179.0, 0.0, -179.0);
    // Should be ~222 km (2 degrees at equator)
    let dist_km = dist / 1000.0;
    assert!(
        (dist_km - 222.0).abs() < 10.0,
        "Date line crossing should be ~222 km, got {dist_km:.1} km"
    );
}

#[test]
fn test_haversine_symmetry() {
    let d1 = haversine_distance(48.8566, 2.3522, 51.5074, -0.1278);
    let d2 = haversine_distance(51.5074, -0.1278, 48.8566, 2.3522);
    assert!(
        (d1 - d2).abs() < f64::EPSILON,
        "Haversine should be symmetric: {d1} vs {d2}"
    );
}

// =========================================================================
// Coordinate validation tests
// =========================================================================

#[test]
fn test_validate_coordinates_valid_boundary() {
    assert!(validate_coordinates(-90.0, -180.0).is_ok());
    assert!(validate_coordinates(90.0, 180.0).is_ok());
    assert!(validate_coordinates(0.0, 0.0).is_ok());
}

#[test]
fn test_validate_coordinates_lat_out_of_range() {
    assert!(validate_coordinates(91.0, 0.0).is_err());
    assert!(validate_coordinates(-91.0, 0.0).is_err());
}

#[test]
fn test_validate_coordinates_lng_out_of_range() {
    assert!(validate_coordinates(0.0, 181.0).is_err());
    assert!(validate_coordinates(0.0, -181.0).is_err());
}
