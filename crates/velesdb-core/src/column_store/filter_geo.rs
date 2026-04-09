//! Geo-spatial filter operations for `ColumnStore`.
//!
//! Provides `GEO_DISTANCE` and `GEO_BBOX` filters with both
//! `Vec<usize>` and `RoaringBitmap` return variants.

use roaring::RoaringBitmap;

use super::haversine::haversine_distance;
use super::types::TypedColumn;
use super::ColumnStore;

/// Comparison operator for geo-distance filters.
///
/// Mirrors `velesql::CompareOp` but lives in the column-store layer
/// to avoid a dependency on the parser module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompareOp {
    /// Equal (=)
    Eq,
    /// Not equal (!=)
    NotEq,
    /// Greater than (>)
    Gt,
    /// Greater than or equal (>=)
    Gte,
    /// Less than (<)
    Lt,
    /// Less than or equal (<=)
    Lte,
}

/// Parameters for `GEO_DISTANCE` filter (>3 params → use struct per project rules).
#[derive(Debug)]
pub struct GeoDistanceParams<'a> {
    /// Column name containing `GeoPoint` data.
    pub column: &'a str,
    /// Reference latitude in degrees.
    pub lat: f64,
    /// Reference longitude in degrees.
    pub lng: f64,
    /// Comparison operator to apply against the computed distance.
    pub operator: CompareOp,
    /// Distance threshold in meters.
    pub threshold: f64,
}

/// Parameters for `GEO_BBOX` filter (>3 params → use struct per project rules).
#[derive(Debug)]
pub struct GeoBboxParams<'a> {
    /// Column name containing `GeoPoint` data.
    pub column: &'a str,
    /// Minimum latitude of the bounding box.
    pub lat_min: f64,
    /// Minimum longitude of the bounding box.
    pub lng_min: f64,
    /// Maximum latitude of the bounding box.
    pub lat_max: f64,
    /// Maximum longitude of the bounding box.
    pub lng_max: f64,
}

/// Applies a comparison operator to two `f64` values.
fn compare_f64(a: f64, b: f64, op: CompareOp) -> bool {
    match op {
        CompareOp::Eq => (a - b).abs() < f64::EPSILON,
        CompareOp::NotEq => (a - b).abs() >= f64::EPSILON,
        CompareOp::Gt => a > b,
        CompareOp::Gte => a >= b,
        CompareOp::Lt => a < b,
        CompareOp::Lte => a <= b,
    }
}

impl ColumnStore {
    /// Returns row indices where the Haversine distance satisfies the comparison.
    ///
    /// Returns empty results for non-existent or non-GeoPoint columns.
    /// Excludes deleted rows and null values.
    #[must_use]
    pub fn filter_geo_distance(&self, params: &GeoDistanceParams<'_>) -> Vec<usize> {
        let Some(TypedColumn::GeoPoint(col)) = self.columns.get(params.column) else {
            return Vec::new();
        };
        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| {
                let (lat, lng) = (*v).as_ref()?;
                let dist = haversine_distance(*lat, *lng, params.lat, params.lng);
                if compare_f64(dist, params.threshold, params.operator)
                    && !self.deleted_rows.contains(&idx)
                {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Bitmap variant of `filter_geo_distance`.
    ///
    /// Safely skips indices exceeding `u32::MAX`.
    #[must_use]
    pub fn filter_geo_distance_bitmap(&self, params: &GeoDistanceParams<'_>) -> RoaringBitmap {
        let Some(TypedColumn::GeoPoint(col)) = self.columns.get(params.column) else {
            return RoaringBitmap::new();
        };
        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| {
                let (lat, lng) = (*v).as_ref()?;
                let dist = haversine_distance(*lat, *lng, params.lat, params.lng);
                if compare_f64(dist, params.threshold, params.operator)
                    && !self.deleted_rows.contains(&idx)
                {
                    u32::try_from(idx).ok()
                } else {
                    None
                }
            })
            .collect()
    }

    /// Returns row indices where the GeoPoint falls within the bounding box (inclusive).
    ///
    /// Returns empty results for non-existent or non-GeoPoint columns,
    /// or when `lat_min > lat_max` or `lng_min > lng_max`.
    #[must_use]
    pub fn filter_geo_bbox(&self, params: &GeoBboxParams<'_>) -> Vec<usize> {
        if params.lat_min > params.lat_max || params.lng_min > params.lng_max {
            return Vec::new();
        }
        let Some(TypedColumn::GeoPoint(col)) = self.columns.get(params.column) else {
            return Vec::new();
        };
        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| {
                let (lat, lng) = (*v).as_ref()?;
                if *lat >= params.lat_min
                    && *lat <= params.lat_max
                    && *lng >= params.lng_min
                    && *lng <= params.lng_max
                    && !self.deleted_rows.contains(&idx)
                {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Bitmap variant of `filter_geo_bbox`.
    ///
    /// Safely skips indices exceeding `u32::MAX`.
    #[must_use]
    pub fn filter_geo_bbox_bitmap(&self, params: &GeoBboxParams<'_>) -> RoaringBitmap {
        if params.lat_min > params.lat_max || params.lng_min > params.lng_max {
            return RoaringBitmap::new();
        }
        let Some(TypedColumn::GeoPoint(col)) = self.columns.get(params.column) else {
            return RoaringBitmap::new();
        };
        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| {
                let (lat, lng) = (*v).as_ref()?;
                if *lat >= params.lat_min
                    && *lat <= params.lat_max
                    && *lng >= params.lng_min
                    && *lng <= params.lng_max
                    && !self.deleted_rows.contains(&idx)
                {
                    u32::try_from(idx).ok()
                } else {
                    None
                }
            })
            .collect()
    }
}
