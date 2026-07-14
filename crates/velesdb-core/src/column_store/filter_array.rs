//! Array CONTAINS filter operations for `ColumnStore`.
//!
//! Provides `CONTAINS`, `CONTAINS_ANY`, and `CONTAINS_ALL` filters
//! with both `Vec<usize>` and `RoaringBitmap` return variants.
//!
//! All six variants share a single iteration template — [`Self::filter_array_indices`]
//! — parameterized by an array predicate closure and an index mapper. This keeps the
//! three predicate semantics (any/all/single) and the two output shapes (Vec / Bitmap)
//! orthogonal.

use roaring::RoaringBitmap;
use smallvec::SmallVec;

use super::types::{ColumnValue, TypedColumn};
use super::ColumnStore;

/// Storage container for array column rows: `Some(values)` when present, `None` when null.
type ArrayRow = Option<SmallVec<[ColumnValue; 8]>>;

impl ColumnStore {
    /// Shared iteration template for all array-CONTAINS filter variants.
    ///
    /// Iterates over the rows of the array `column`, applying `predicate` to each
    /// non-null row's values, skipping deleted rows, then mapping the surviving
    /// row indices through `map_idx`. The resulting items are collected into
    /// any container `C` implementing [`Default`] + [`FromIterator<T>`].
    ///
    /// Returns `C::default()` when the column is missing or not an array.
    ///
    /// # Type parameters
    ///
    /// * `P` — predicate over a row's array values (`true` keeps the row)
    /// * `M` — maps `row_idx: usize` to the output item `Option<T>` (`None` drops the row)
    /// * `T` — item type (e.g. `usize` for `Vec<usize>`, `u32` for `RoaringBitmap`)
    /// * `C` — output container
    fn filter_array_indices<P, M, T, C>(&self, column: &str, predicate: P, map_idx: M) -> C
    where
        P: Fn(&SmallVec<[ColumnValue; 8]>) -> bool,
        M: Fn(usize) -> Option<T>,
        C: Default + FromIterator<T>,
    {
        let Some(TypedColumn::Array { data, .. }) = self.columns.get(column) else {
            return C::default();
        };
        Self::iter_array_matches(data, &predicate, &self.deletion_bitmap, map_idx)
    }

    /// Inner loop shared by [`Self::filter_array_indices`]. Kept as a free associated
    /// function so callers don't need to capture the full `&self` and the borrow checker
    /// can see precisely which fields are read.
    fn iter_array_matches<P, M, T, C>(
        data: &[ArrayRow],
        predicate: &P,
        deletion_bitmap: &RoaringBitmap,
        map_idx: M,
    ) -> C
    where
        P: Fn(&SmallVec<[ColumnValue; 8]>) -> bool,
        M: Fn(usize) -> Option<T>,
        C: FromIterator<T>,
    {
        data.iter()
            .enumerate()
            .filter_map(|(idx, row)| {
                let arr = row.as_ref()?;
                // Deleted rows live in `deletion_bitmap` (u32-indexed); indices that
                // don't fit `u32` can never be tombstoned, so they count as live.
                let deleted = u32::try_from(idx).is_ok_and(|i| deletion_bitmap.contains(i));
                if predicate(arr) && !deleted {
                    map_idx(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Returns row indices where the array column contains `value`.
    ///
    /// Returns empty results for non-existent or non-array columns.
    /// Excludes deleted rows and null arrays.
    #[must_use]
    pub fn filter_contains(&self, column: &str, value: &ColumnValue) -> Vec<usize> {
        self.filter_array_indices(column, |arr| arr.contains(value), Some)
    }

    /// Returns row indices where the array contains at least one of `values`.
    ///
    /// Returns empty results for non-existent or non-array columns.
    #[must_use]
    pub fn filter_contains_any(&self, column: &str, values: &[ColumnValue]) -> Vec<usize> {
        self.filter_array_indices(column, |arr| values.iter().any(|v| arr.contains(v)), Some)
    }

    /// Returns row indices where the array contains every value in `values`.
    ///
    /// Returns empty results for non-existent or non-array columns.
    #[must_use]
    pub fn filter_contains_all(&self, column: &str, values: &[ColumnValue]) -> Vec<usize> {
        self.filter_array_indices(column, |arr| values.iter().all(|v| arr.contains(v)), Some)
    }

    /// Bitmap variant of [`Self::filter_contains`].
    ///
    /// Safely skips indices exceeding `u32::MAX`.
    #[must_use]
    pub fn filter_contains_bitmap(&self, column: &str, value: &ColumnValue) -> RoaringBitmap {
        self.filter_array_indices(
            column,
            |arr| arr.contains(value),
            |idx| u32::try_from(idx).ok(),
        )
    }

    /// Bitmap variant of [`Self::filter_contains_any`].
    ///
    /// Safely skips indices exceeding `u32::MAX`.
    #[must_use]
    pub fn filter_contains_any_bitmap(
        &self,
        column: &str,
        values: &[ColumnValue],
    ) -> RoaringBitmap {
        self.filter_array_indices(
            column,
            |arr| values.iter().any(|v| arr.contains(v)),
            |idx| u32::try_from(idx).ok(),
        )
    }

    /// Bitmap variant of [`Self::filter_contains_all`].
    ///
    /// Safely skips indices exceeding `u32::MAX`.
    #[must_use]
    pub fn filter_contains_all_bitmap(
        &self,
        column: &str,
        values: &[ColumnValue],
    ) -> RoaringBitmap {
        self.filter_array_indices(
            column,
            |arr| values.iter().all(|v| arr.contains(v)),
            |idx| u32::try_from(idx).ok(),
        )
    }
}
