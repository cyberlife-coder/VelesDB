//! Unit tests for `GeoPoint` coordinate validation.

use super::coordinates::validate_coordinates;

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
