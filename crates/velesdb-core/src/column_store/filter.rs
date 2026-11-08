//! Scalar filter operations for `ColumnStore`.
//!
//! This module provides efficient filtering methods for scalar column types
//! (integer, string) including bitmap-based operations for large datasets.
//!
//! Array filters live in [`super::filter_array`], geo filters in
//! [`super::filter_geo`].

use roaring::RoaringBitmap;

use super::types::{Masked, StringId, TypedColumn};
use super::ColumnStore;

// =========================================================================
// Private scan helpers — eliminate duplication across Vec / bitmap / count
// =========================================================================

/// Borrows both bitmaps: iterates `validity`'s set bits ascending, dropping
/// rows tombstoned in `deletion_bitmap` — one probe per valid row and zero
/// allocation. The `0..len` + `Option` scan this replaces paid two bitmap
/// probes plus an `Option` match per row; paying a `validity − deletion`
/// copy instead measured slower at 10k–100k rows, where the copied bitmap
/// evicts the values a dense scan then has to re-read.
fn live_indices<'a>(
    validity: &'a RoaringBitmap,
    deletion_bitmap: &'a RoaringBitmap,
) -> impl Iterator<Item = u32> + 'a {
    validity
        .iter()
        .filter(move |&idx| !deletion_bitmap.contains(idx))
}

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
    scan_cells_vec(col, &store.deletion_bitmap, predicate)
}

/// Scans a typed column slice, returning a `RoaringBitmap` of indices where
/// `predicate(val)` is true. Excludes deleted rows (via `deletion_bitmap`).
///
/// Indices >= `u32::MAX` are safely skipped (not truncated).
fn scan_cells_bitmap<T: Copy>(
    cells: &Masked<T>,
    deletion_bitmap: &RoaringBitmap,
    predicate: impl Fn(T) -> bool,
) -> RoaringBitmap {
    let values = cells.values();
    live_indices(cells.validity(), deletion_bitmap)
        .filter(|&idx| predicate(values[idx as usize]))
        .collect()
}

/// `Vec<usize>`-collecting variant of [`scan_cells_bitmap`].
fn scan_cells_vec<T: Copy>(
    cells: &Masked<T>,
    deletion_bitmap: &RoaringBitmap,
    predicate: impl Fn(T) -> bool,
) -> Vec<usize> {
    let values = cells.values();
    live_indices(cells.validity(), deletion_bitmap)
        .filter(|&idx| predicate(values[idx as usize]))
        .map(|idx| idx as usize)
        .collect()
}

/// Alloc-free counting variant of [`scan_cells_bitmap`].
fn count_cells<T: Copy>(
    cells: &Masked<T>,
    deletion_bitmap: &RoaringBitmap,
    predicate: impl Fn(T) -> bool,
) -> usize {
    let values = cells.values();
    live_indices(cells.validity(), deletion_bitmap)
        .filter(|&idx| predicate(values[idx as usize]))
        .count()
}

/// Scans an integer column, returning a `RoaringBitmap` of matching indices.
fn scan_int_column_bitmap(
    store: &ColumnStore,
    column: &str,
    predicate: impl Fn(i64) -> bool,
) -> RoaringBitmap {
    let Some(TypedColumn::Int(col)) = store.columns.get(column) else {
        return RoaringBitmap::new();
    };
    scan_cells_bitmap(col, &store.deletion_bitmap, predicate)
}

/// Scans a float column, returning a `RoaringBitmap` of matching indices.
fn scan_float_column_bitmap(
    store: &ColumnStore,
    column: &str,
    predicate: impl Fn(f64) -> bool,
) -> RoaringBitmap {
    let Some(TypedColumn::Float(col)) = store.columns.get(column) else {
        return RoaringBitmap::new();
    };
    scan_cells_bitmap(col, &store.deletion_bitmap, predicate)
}

/// Scans a bool column, returning a `RoaringBitmap` of rows equal to `value`.
fn scan_bool_column_bitmap(store: &ColumnStore, column: &str, value: bool) -> RoaringBitmap {
    let Some(TypedColumn::Bool(col)) = store.columns.get(column) else {
        return RoaringBitmap::new();
    };
    scan_cells_bitmap(col, &store.deletion_bitmap, |v| v == value)
}

/// Scans a string column for rows whose interned id matches `target_id`.
///
/// Returns indices as a `Vec<usize>`. Excludes deleted rows.
fn scan_string_column(store: &ColumnStore, column: &str, target_id: StringId) -> Vec<usize> {
    let Some(TypedColumn::String(col)) = store.columns.get(column) else {
        return Vec::new();
    };
    scan_cells_vec(col, &store.deletion_bitmap, |id| id == target_id)
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
    scan_cells_bitmap(col, &store.deletion_bitmap, |id| id == target_id)
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
        count_cells(col, &self.deletion_bitmap, |v| v == value)
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
        count_cells(col, &self.deletion_bitmap, |id| id == string_id)
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

    /// Filters a float column by an arbitrary predicate, returning a bitmap.
    ///
    /// Excludes deleted rows. Indices >= `u32::MAX` are safely skipped.
    /// Used by the payload mirror, which stores all JSON numbers as `f64`
    /// to mirror the JSON filter's numeric comparison semantics.
    pub(crate) fn filter_float_bitmap(
        &self,
        column: &str,
        predicate: impl Fn(f64) -> bool,
    ) -> RoaringBitmap {
        scan_float_column_bitmap(self, column, predicate)
    }

    /// Filters a bool column by equality, returning a bitmap.
    ///
    /// Excludes deleted rows. Indices >= `u32::MAX` are safely skipped.
    pub(crate) fn filter_bool_eq_bitmap(&self, column: &str, value: bool) -> RoaringBitmap {
        scan_bool_column_bitmap(self, column, value)
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

    /// Returns a `RoaringBitmap` of row indices matching any value in the IN list.
    ///
    /// Reuses `scan_string_column_in` infrastructure, converting `Vec<usize>` to bitmap.
    /// Excludes deleted rows. Returns empty bitmap for missing/mismatched columns.
    ///
    /// Time complexity: O(rows) — full column scan with `HashSet` lookup per row.
    #[must_use]
    pub fn filter_in_string_bitmap(&self, column: &str, values: &[&str]) -> RoaringBitmap {
        let Some(TypedColumn::String(col)) = self.columns.get(column) else {
            return RoaringBitmap::new();
        };

        let ids: Vec<StringId> = values
            .iter()
            .filter_map(|s| self.string_table.get_id(s))
            .collect();

        if ids.is_empty() {
            return RoaringBitmap::new();
        }

        self.scan_string_column_in_bitmap(col, &ids)
    }

    /// Returns a `RoaringBitmap` of row indices matching any i64 value in the IN list.
    ///
    /// Uses `FxHashSet` for O(1) per-row membership test when `values.len() > 16`,
    /// linear scan otherwise. Excludes deleted rows.
    ///
    /// Time complexity: O(rows) — full column scan.
    #[must_use]
    pub fn filter_in_int_bitmap(&self, column: &str, values: &[i64]) -> RoaringBitmap {
        if values.len() > 16 {
            let set: rustc_hash::FxHashSet<i64> = values.iter().copied().collect();
            scan_int_column_bitmap(self, column, |v| set.contains(&v))
        } else {
            scan_int_column_bitmap(self, column, |v| values.contains(&v))
        }
    }

    /// Scans a string column for rows whose interned id is in `ids`,
    /// returning a `RoaringBitmap`. Excludes deleted rows.
    ///
    /// Uses `FxHashSet` for large id lists (>16) and linear scan for small ones.
    /// Indices >= `u32::MAX` are safely skipped.
    fn scan_string_column_in_bitmap(
        &self,
        col: &Masked<StringId>,
        ids: &[StringId],
    ) -> RoaringBitmap {
        if ids.len() > 16 {
            let id_set: rustc_hash::FxHashSet<StringId> = ids.iter().copied().collect();
            self.filter_column_by_bitmap(col, |id| id_set.contains(id))
        } else {
            self.filter_column_by_bitmap(col, |id| ids.contains(id))
        }
    }

    /// Filters a string column by a predicate on the interned id, returning a bitmap.
    ///
    /// Excludes deleted rows. Indices >= `u32::MAX` are safely skipped.
    fn filter_column_by_bitmap(
        &self,
        col: &Masked<StringId>,
        predicate: impl Fn(&StringId) -> bool,
    ) -> RoaringBitmap {
        let values = col.values();
        live_indices(col.validity(), &self.deletion_bitmap)
            .filter(|&idx| predicate(&values[idx as usize]))
            .collect()
    }

    /// Scans a string column for rows whose interned id is in `ids`.
    ///
    /// Uses a `FxHashSet` for large id lists (>16) and linear scan for small ones.
    fn scan_string_column_in(&self, col: &Masked<StringId>, ids: &[StringId]) -> Vec<usize> {
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
        col: &Masked<StringId>,
        predicate: impl Fn(&StringId) -> bool,
    ) -> Vec<usize> {
        let values = col.values();
        live_indices(col.validity(), &self.deletion_bitmap)
            .filter(|&idx| predicate(&values[idx as usize]))
            .map(|idx| idx as usize)
            .collect()
    }
}
