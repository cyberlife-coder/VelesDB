//! Coordinate range validation for `GeoPoint` columns.
//!
//! The ranges themselves, and the great-circle distance, live in
//! [`crate::geo_distance`], shared with `filter::matching`.

use super::types::ColumnStoreError;
use crate::geo_distance::{LATITUDE_RANGE_DEG, LONGITUDE_RANGE_DEG};

/// Validates that latitude is in [-90, +90] and longitude is in [-180, +180].
///
/// # Errors
///
/// Returns `ColumnStoreError::TypeMismatch` with a descriptive message
/// when either coordinate is out of range.
pub(crate) fn validate_coordinates(lat: f64, lng: f64) -> Result<(), ColumnStoreError> {
    if !LATITUDE_RANGE_DEG.contains(&lat) {
        return Err(ColumnStoreError::TypeMismatch {
            expected: "latitude in [-90, 90]".to_string(),
            actual: format!("{lat}"),
        });
    }
    if !LONGITUDE_RANGE_DEG.contains(&lng) {
        return Err(ColumnStoreError::TypeMismatch {
            expected: "longitude in [-180, 180]".to_string(),
            actual: format!("{lng}"),
        });
    }
    Ok(())
}
