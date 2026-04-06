//! Haversine distance computation and coordinate validation.
//!
//! Provides a pure, allocation-free great-circle distance function and
//! coordinate range validation for GeoPoint columns.

use super::types::ColumnStoreError;

/// Earth radius in meters (WGS-84 mean radius).
const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// Computes the great-circle distance between two points using the Haversine formula.
///
/// Accepts coordinates in degrees. Returns distance in meters.
/// Pure function, no allocations, no side effects.
#[must_use]
pub(crate) fn haversine_distance(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    let (lat1, lng1) = (lat1.to_radians(), lng1.to_radians());
    let (lat2, lng2) = (lat2.to_radians(), lng2.to_radians());
    let dlat = lat2 - lat1;
    let dlng = lng2 - lng1;
    let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlng / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    EARTH_RADIUS_M * c
}

/// Validates that latitude is in [-90, +90] and longitude is in [-180, +180].
///
/// # Errors
///
/// Returns `ColumnStoreError::TypeMismatch` with a descriptive message
/// when either coordinate is out of range.
pub(crate) fn validate_coordinates(lat: f64, lng: f64) -> Result<(), ColumnStoreError> {
    if !(-90.0..=90.0).contains(&lat) {
        return Err(ColumnStoreError::TypeMismatch {
            expected: "latitude in [-90, 90]".to_string(),
            actual: format!("{lat}"),
        });
    }
    if !(-180.0..=180.0).contains(&lng) {
        return Err(ColumnStoreError::TypeMismatch {
            expected: "longitude in [-180, 180]".to_string(),
            actual: format!("{lng}"),
        });
    }
    Ok(())
}
