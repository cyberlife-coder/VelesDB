//! Array CONTAINS filter operations for `ColumnStore`.
//!
//! Provides `CONTAINS`, `CONTAINS_ANY`, and `CONTAINS_ALL` filters
//! with both `Vec<usize>` and `RoaringBitmap` return variants.

use roaring::RoaringBitmap;

use super::types::TypedColumn;
use super::ColumnStore;

impl ColumnStore {
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
