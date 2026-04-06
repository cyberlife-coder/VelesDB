//! Filter operations for ColumnStore.
//!
//! This module provides efficient filtering methods for column-oriented data,
//! including bitmap-based operations for large datasets.

use roaring::RoaringBitmap;

use super::types::{StringId, TypedColumn};
use super::ColumnStore;

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
}
