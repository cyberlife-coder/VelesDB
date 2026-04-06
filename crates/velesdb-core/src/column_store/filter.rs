//! Filter operations for ColumnStore.
//!
//! This module provides efficient filtering methods for column-oriented data,
//! including bitmap-based operations for large datasets.

use roaring::RoaringBitmap;

use super::haversine::haversine_distance;
use super::types::{StringId, TypedColumn};
use super::ColumnStore;

/// Comparison operator for geo-distance filters.
///
/// Mirrors `velesql::CompareOp` but lives in the column-store layer
/// to avoid a dependency on the parser module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

// =========================================================================
// Private scan helpers — eliminate duplication across Vec / bitmap / count
// =========================================================================

/// Scans an integer column, returning indices where `predicate(val)` is true.
///
/// Excludes deleted rows. Used by `filter_eq_int`, `filter_gt_int`, etc.
fn scan_int_column(
    store: &ColumnStore,
    column: &str,
    predicate: impl Fn(i64) -> bool,
) -> Vec<usize> {
    let Some(TypedColumn::Int(col)) = store.columns.get(column) else {
        return Vec::new();
    };
    col.iter()
        .enumerate()
        .filter_map(|(idx, v)| match v {
            Some(val) if predicate(*val) && !store.deleted_rows.contains(&idx) => Some(idx),
            _ => None,
        })
        .collect()
}

/// Scans an integer column, returning a `RoaringBitmap` of matching indices.
///
/// Indices >= `u32::MAX` are safely skipped (not truncated).
fn scan_int_column_bitmap(
    store: &ColumnStore,
    column: &str,
    predicate: impl Fn(i64) -> bool,
) -> RoaringBitmap {
    let Some(TypedColumn::Int(col)) = store.columns.get(column) else {
        return RoaringBitmap::new();
    };
    col.iter()
        .enumerate()
        .filter_map(|(idx, v)| match v {
            Some(val) if predicate(*val) && !store.deleted_rows.contains(&idx) => {
                u32::try_from(idx).ok()
            }
            _ => None,
        })
        .collect()
}

/// Scans a string column for rows whose interned id matches `target_id`.
///
/// Returns indices as a `Vec<usize>`. Excludes deleted rows.
fn scan_string_column(store: &ColumnStore, column: &str, target_id: StringId) -> Vec<usize> {
    let Some(TypedColumn::String(col)) = store.columns.get(column) else {
        return Vec::new();
    };
    col.iter()
        .enumerate()
        .filter_map(|(idx, v)| {
            if *v == Some(target_id) && !store.deleted_rows.contains(&idx) {
                Some(idx)
            } else {
                None
            }
        })
        .collect()
}

/// Scans a string column for rows whose interned id matches `target_id`,
/// returning a `RoaringBitmap`.
fn scan_string_column_bitmap(
    store: &ColumnStore,
    column: &str,
    target_id: StringId,
) -> RoaringBitmap {
    let Some(TypedColumn::String(col)) = store.columns.get(column) else {
        return RoaringBitmap::new();
    };
    col.iter()
        .enumerate()
        .filter_map(|(idx, v)| {
            if *v == Some(target_id) && !store.deleted_rows.contains(&idx) {
                u32::try_from(idx).ok()
            } else {
                None
            }
        })
        .collect()
}

impl ColumnStore {
    /// Filters rows by equality on an integer column.
    ///
    /// Returns a vector of row indices that match. Excludes deleted rows.
    #[must_use]
    pub fn filter_eq_int(&self, column: &str, value: i64) -> Vec<usize> {
        scan_int_column(self, column, |v| v == value)
    }

    /// Filters rows by equality on a string column.
    ///
    /// Returns a vector of row indices that match. Excludes deleted rows.
    #[must_use]
    pub fn filter_eq_string(&self, column: &str, value: &str) -> Vec<usize> {
        let Some(string_id) = self.string_table.get_id(value) else {
            return Vec::new();
        };
        scan_string_column(self, column, string_id)
    }

    /// Filters rows by range on an integer column (value > threshold).
    ///
    /// Returns a vector of row indices that match. Excludes deleted rows.
    #[must_use]
    pub fn filter_gt_int(&self, column: &str, threshold: i64) -> Vec<usize> {
        scan_int_column(self, column, |v| v > threshold)
    }

    /// Filters rows by range on an integer column (value < threshold).
    ///
    /// Excludes deleted rows.
    #[must_use]
    pub fn filter_lt_int(&self, column: &str, threshold: i64) -> Vec<usize> {
        scan_int_column(self, column, |v| v < threshold)
    }

    /// Filters rows by range on an integer column (low < value < high).
    ///
    /// Excludes deleted rows.
    #[must_use]
    pub fn filter_range_int(&self, column: &str, low: i64, high: i64) -> Vec<usize> {
        scan_int_column(self, column, |v| v > low && v < high)
    }

    /// Filters rows by IN clause on a string column.
    ///
    /// Returns a vector of row indices that match any of the values. Excludes deleted rows.
    #[must_use]
    pub fn filter_in_string(&self, column: &str, values: &[&str]) -> Vec<usize> {
        let Some(TypedColumn::String(col)) = self.columns.get(column) else {
            return Vec::new();
        };

        let ids: Vec<StringId> = values
            .iter()
            .filter_map(|s| self.string_table.get_id(s))
            .collect();

        if ids.is_empty() {
            return Vec::new();
        }

        self.scan_string_column_in(col, &ids)
    }

    /// Counts rows matching equality on an integer column.
    ///
    /// More efficient than `filter_eq_int().len()` as it doesn't allocate. Excludes deleted rows.
    #[must_use]
    pub fn count_eq_int(&self, column: &str, value: i64) -> usize {
        let Some(TypedColumn::Int(col)) = self.columns.get(column) else {
            return 0;
        };

        col.iter()
            .enumerate()
            .filter(|(idx, v)| **v == Some(value) && !self.deleted_rows.contains(idx))
            .count()
    }

    /// Counts rows matching equality on a string column. Excludes deleted rows.
    #[must_use]
    pub fn count_eq_string(&self, column: &str, value: &str) -> usize {
        let Some(TypedColumn::String(col)) = self.columns.get(column) else {
            return 0;
        };

        let Some(string_id) = self.string_table.get_id(value) else {
            return 0;
        };

        col.iter()
            .enumerate()
            .filter(|(idx, v)| **v == Some(string_id) && !self.deleted_rows.contains(idx))
            .count()
    }

    // =========================================================================
    // Optimized Bitmap-based Filtering (for 100k+ items)
    // =========================================================================

    /// Filters rows by equality on an integer column, returning a bitmap.
    ///
    /// Uses `RoaringBitmap` for memory-efficient storage of matching indices.
    /// Useful for combining multiple filters with AND/OR operations.
    ///
    /// # Note
    ///
    /// Row indices are safely converted to u32 for `RoaringBitmap`. This limits
    /// stores to ~4B rows. Indices >= `u32::MAX` are safely skipped (not truncated).
    #[must_use]
    pub fn filter_eq_int_bitmap(&self, column: &str, value: i64) -> RoaringBitmap {
        scan_int_column_bitmap(self, column, |v| v == value)
    }

    /// Filters rows by equality on a string column, returning a bitmap.
    ///
    /// Indices >= `u32::MAX` are safely skipped.
    #[must_use]
    pub fn filter_eq_string_bitmap(&self, column: &str, value: &str) -> RoaringBitmap {
        let Some(string_id) = self.string_table.get_id(value) else {
            return RoaringBitmap::new();
        };
        scan_string_column_bitmap(self, column, string_id)
    }

    /// Filters rows by range on an integer column, returning a bitmap.
    ///
    /// Indices >= `u32::MAX` are safely skipped.
    #[must_use]
    pub fn filter_range_int_bitmap(&self, column: &str, low: i64, high: i64) -> RoaringBitmap {
        scan_int_column_bitmap(self, column, |v| v > low && v < high)
    }

    /// Combines two filter results using AND.
    ///
    /// Returns indices that are in both bitmaps.
    #[must_use]
    pub fn bitmap_and(a: &RoaringBitmap, b: &RoaringBitmap) -> RoaringBitmap {
        a & b
    }

    /// Combines two filter results using OR.
    ///
    /// Returns indices that are in either bitmap.
    #[must_use]
    pub fn bitmap_or(a: &RoaringBitmap, b: &RoaringBitmap) -> RoaringBitmap {
        a | b
    }

    /// Scans a string column for rows whose interned id is in `ids`.
    ///
    /// Uses a `FxHashSet` for large id lists (>16) and linear scan for small ones.
    fn scan_string_column_in(&self, col: &[Option<StringId>], ids: &[StringId]) -> Vec<usize> {
        if ids.len() > 16 {
            let id_set: rustc_hash::FxHashSet<StringId> = ids.iter().copied().collect();
            self.filter_column_by(col, |id| id_set.contains(id))
        } else {
            self.filter_column_by(col, |id| ids.contains(id))
        }
    }

    /// Filters a string column by a predicate on the interned id, excluding deleted rows.
    fn filter_column_by(
        &self,
        col: &[Option<StringId>],
        predicate: impl Fn(&StringId) -> bool,
    ) -> Vec<usize> {
        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| match v {
                Some(id) if predicate(id) && !self.deleted_rows.contains(&idx) => Some(idx),
                _ => None,
            })
            .collect()
    }

    // =========================================================================
    // Array CONTAINS Filtering
    // =========================================================================

    /// Returns row indices where the array column contains `value`.
    ///
    /// Returns empty results for non-existent or non-array columns.
    /// Excludes deleted rows and null arrays.
    #[must_use]
    pub fn filter_contains(&self, column: &str, value: &super::types::ColumnValue) -> Vec<usize> {
        let Some(TypedColumn::Array { data, .. }) = self.columns.get(column) else {
            return Vec::new();
        };
        data.iter()
            .enumerate()
            .filter_map(|(idx, row)| {
                let arr = row.as_ref()?;
                if arr.contains(value) && !self.deleted_rows.contains(&idx) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Returns row indices where the array contains at least one of `values`.
    ///
    /// Uses `FxHashSet` optimization when M > 8 values.
    /// Returns empty results for non-existent or non-array columns.
    #[must_use]
    pub fn filter_contains_any(
        &self,
        column: &str,
        values: &[super::types::ColumnValue],
    ) -> Vec<usize> {
        let Some(TypedColumn::Array { data, .. }) = self.columns.get(column) else {
            return Vec::new();
        };
        if values.len() > 8 {
            self.contains_any_hashset(data, values)
        } else {
            self.contains_any_linear(data, values)
        }
    }

    /// Linear scan for `filter_contains_any` with small value lists.
    fn contains_any_linear(
        &self,
        data: &[Option<smallvec::SmallVec<[super::types::ColumnValue; 8]>>],
        values: &[super::types::ColumnValue],
    ) -> Vec<usize> {
        data.iter()
            .enumerate()
            .filter_map(|(idx, row)| {
                let arr = row.as_ref()?;
                if values.iter().any(|v| arr.contains(v)) && !self.deleted_rows.contains(&idx) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// `FxHashSet`-based scan for `filter_contains_any` with large value lists.
    fn contains_any_hashset(
        &self,
        data: &[Option<smallvec::SmallVec<[super::types::ColumnValue; 8]>>],
        values: &[super::types::ColumnValue],
    ) -> Vec<usize> {
        let value_set: Vec<&super::types::ColumnValue> = values.iter().collect();
        data.iter()
            .enumerate()
            .filter_map(|(idx, row)| {
                let arr = row.as_ref()?;
                if arr.iter().any(|e| value_set.contains(&e)) && !self.deleted_rows.contains(&idx) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Returns row indices where the array contains every value in `values`.
    ///
    /// Returns empty results for non-existent or non-array columns.
    #[must_use]
    pub fn filter_contains_all(
        &self,
        column: &str,
        values: &[super::types::ColumnValue],
    ) -> Vec<usize> {
        let Some(TypedColumn::Array { data, .. }) = self.columns.get(column) else {
            return Vec::new();
        };
        data.iter()
            .enumerate()
            .filter_map(|(idx, row)| {
                let arr = row.as_ref()?;
                if values.iter().all(|v| arr.contains(v)) && !self.deleted_rows.contains(&idx) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Bitmap variant of `filter_contains`.
    ///
    /// Safely skips indices exceeding `u32::MAX`.
    #[must_use]
    pub fn filter_contains_bitmap(
        &self,
        column: &str,
        value: &super::types::ColumnValue,
    ) -> RoaringBitmap {
        let Some(TypedColumn::Array { data, .. }) = self.columns.get(column) else {
            return RoaringBitmap::new();
        };
        data.iter()
            .enumerate()
            .filter_map(|(idx, row)| {
                let arr = row.as_ref()?;
                if arr.contains(value) && !self.deleted_rows.contains(&idx) {
                    u32::try_from(idx).ok()
                } else {
                    None
                }
            })
            .collect()
    }

    /// Bitmap variant of `filter_contains_any`.
    ///
    /// Safely skips indices exceeding `u32::MAX`.
    #[must_use]
    pub fn filter_contains_any_bitmap(
        &self,
        column: &str,
        values: &[super::types::ColumnValue],
    ) -> RoaringBitmap {
        let Some(TypedColumn::Array { data, .. }) = self.columns.get(column) else {
            return RoaringBitmap::new();
        };
        data.iter()
            .enumerate()
            .filter_map(|(idx, row)| {
                let arr = row.as_ref()?;
                if values.iter().any(|v| arr.contains(v)) && !self.deleted_rows.contains(&idx) {
                    u32::try_from(idx).ok()
                } else {
                    None
                }
            })
            .collect()
    }

    /// Bitmap variant of `filter_contains_all`.
    ///
    /// Safely skips indices exceeding `u32::MAX`.
    #[must_use]
    pub fn filter_contains_all_bitmap(
        &self,
        column: &str,
        values: &[super::types::ColumnValue],
    ) -> RoaringBitmap {
        let Some(TypedColumn::Array { data, .. }) = self.columns.get(column) else {
            return RoaringBitmap::new();
        };
        data.iter()
            .enumerate()
            .filter_map(|(idx, row)| {
                let arr = row.as_ref()?;
                if values.iter().all(|v| arr.contains(v)) && !self.deleted_rows.contains(&idx) {
                    u32::try_from(idx).ok()
                } else {
                    None
                }
            })
            .collect()
    }

    // =========================================================================
    // GeoPoint Filtering
    // =========================================================================

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
