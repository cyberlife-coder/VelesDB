//! Geo-spatial filter operations for `ColumnStore`.
//!
//! Provides `GEO_DISTANCE` and `GEO_BBOX` filters with both
//! `Vec<usize>` and `RoaringBitmap` return variants.

use roaring::RoaringBitmap;

use super::types::TypedColumn;
use super::ColumnStore;
use crate::geo_distance::{distance_satisfies, great_circle_distance_m};

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

impl ColumnStore {
    /// Returns row indices where the great-circle distance satisfies the comparison.
    ///
    /// The distance and the comparison rule (equality to the millimetre, no
    /// match for an out-of-range reference point or a NaN threshold) come
    /// from the crate's `geo_distance` module, shared with `VelesQL` payload
    /// filtering.
    ///
    /// Returns empty results for non-existent or non-GeoPoint columns.
    /// Excludes deleted rows and null values.
    #[must_use]
    pub fn filter_geo_distance(&self, params: &GeoDistanceParams<'_>) -> Vec<usize> {
        self.geo_distance_rows(params).collect()
    }

    /// Bitmap variant of `filter_geo_distance`.
    ///
    /// Safely skips indices exceeding `u32::MAX`.
    #[must_use]
    pub fn filter_geo_distance_bitmap(&self, params: &GeoDistanceParams<'_>) -> RoaringBitmap {
        self.geo_distance_rows(params)
            .filter_map(|idx| u32::try_from(idx).ok())
            .collect()
    }

    /// Rows whose point satisfies `GEO_DISTANCE` (see `filter_geo_distance`).
    fn geo_distance_rows<'a>(
        &'a self,
        params: &'a GeoDistanceParams<'_>,
    ) -> impl Iterator<Item = usize> + 'a {
        let operator = params.operator.into();
        self.geo_rows(params.column, move |lat, lng| {
            let dist = great_circle_distance_m(lat, lng, params.lat, params.lng);
            distance_satisfies(dist, operator, params.threshold)
        })
    }

    /// Rows whose point lies in the `GEO_BBOX` (see `filter_geo_bbox`).
    fn geo_bbox_rows<'a>(
        &'a self,
        params: &'a GeoBboxParams<'_>,
    ) -> impl Iterator<Item = usize> + 'a {
        let inverted = params.lat_min > params.lat_max || params.lng_min > params.lng_max;
        // `take(0)` yields nothing without reading a single row.
        let limit = if inverted { 0 } else { usize::MAX };
        self.geo_rows(params.column, move |lat, lng| {
            (params.lat_min..=params.lat_max).contains(&lat)
                && (params.lng_min..=params.lng_max).contains(&lng)
        })
        .take(limit)
    }

    /// Live, non-null rows of the `GeoPoint` column `column` whose point
    /// satisfies `keep`; no row when the column is missing or not a
    /// `GeoPoint` column.
    fn geo_rows<'a>(
        &'a self,
        column: &'a str,
        keep: impl Fn(f64, f64) -> bool + 'a,
    ) -> impl Iterator<Item = usize> + 'a {
        let points = match self.columns.get(column) {
            Some(TypedColumn::GeoPoint(col)) => col.as_slice(),
            _ => &[],
        };
        points.iter().enumerate().filter_map(move |(idx, point)| {
            let (lat, lng) = (*point)?;
            (keep(lat, lng) && !self.is_row_deleted_bitmap(idx)).then_some(idx)
        })
    }

    /// Returns row indices where the GeoPoint falls within the bounding box (inclusive).
    ///
    /// Returns empty results for non-existent or non-GeoPoint columns,
    /// or when `lat_min > lat_max` or `lng_min > lng_max`.
    #[must_use]
    pub fn filter_geo_bbox(&self, params: &GeoBboxParams<'_>) -> Vec<usize> {
        self.geo_bbox_rows(params).collect()
    }

    /// Bitmap variant of `filter_geo_bbox`.
    ///
    /// Safely skips indices exceeding `u32::MAX`.
    #[must_use]
    pub fn filter_geo_bbox_bitmap(&self, params: &GeoBboxParams<'_>) -> RoaringBitmap {
        self.geo_bbox_rows(params)
            .filter_map(|idx| u32::try_from(idx).ok())
            .collect()
    }
}
