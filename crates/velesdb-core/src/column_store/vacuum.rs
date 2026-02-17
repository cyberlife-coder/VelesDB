//! Vacuum and compaction operations for `ColumnStore`.
//!
//! Extracted from `mod.rs` for maintainability (04-06 module splitting).
//! Handles tombstone removal, column compaction, and deletion bitmap operations.

use super::types::{TypedColumn, VacuumConfig, VacuumStats};
use super::ColumnStore;

use roaring::RoaringBitmap;
use std::collections::HashMap;

/// Checks if an index is marked as deleted in the bitmap.
#[inline]
fn is_idx_deleted(deleted: &RoaringBitmap, idx: usize) -> bool {
    u32::try_from(idx).is_ok_and(|i| deleted.contains(i))
}

impl ColumnStore {
    /// Runs vacuum to remove tombstones and compact data.
    ///
    /// This operation removes deleted rows from the column store, reclaiming
    /// space and improving query performance. The operation is done in-place
    /// by building new column vectors without the deleted rows.
    ///
    /// # Arguments
    ///
    /// * `_config` - Vacuum configuration (`batch_size`, sync options)
    ///
    /// # Returns
    ///
    /// Statistics about the vacuum operation.
    pub fn vacuum(&mut self, _config: VacuumConfig) -> VacuumStats {
        let start = std::time::Instant::now();
        let tombstones_found = self.deletion_bitmap.len() as usize;

        // Phase 1: Early exit if no tombstones
        if tombstones_found == 0 {
            return VacuumStats {
                tombstones_found: 0,
                completed: true,
                duration_ms: start.elapsed().as_millis() as u64,
                ..Default::default()
            };
        }

        let mut stats = VacuumStats {
            tombstones_found,
            ..Default::default()
        };

        // Phase 2: Build index mapping (old_idx -> new_idx)
        let mut old_to_new: HashMap<usize, usize> = HashMap::new();
        let mut new_idx = 0;
        for old_idx in 0..self.row_count {
            if !self.is_deleted(old_idx) {
                old_to_new.insert(old_idx, new_idx);
                new_idx += 1;
            }
        }
        let new_row_count = new_idx;

        // Phase 3: Compact each column
        for column in self.columns.values_mut() {
            let (new_col, bytes) = Self::compact_column(column, &self.deletion_bitmap);
            stats.bytes_reclaimed += bytes;
            *column = new_col;
        }

        // Phase 4: Update primary index
        if self.primary_key_column.is_some() {
            let mut new_primary_index: HashMap<i64, usize> = HashMap::new();
            let mut new_row_idx_to_pk: HashMap<usize, i64> = HashMap::new();

            for (pk, old_idx) in &self.primary_index {
                if let Some(&new_idx) = old_to_new.get(old_idx) {
                    new_primary_index.insert(*pk, new_idx);
                    new_row_idx_to_pk.insert(new_idx, *pk);
                }
            }

            self.primary_index = new_primary_index;
            self.row_idx_to_pk = new_row_idx_to_pk;
        }

        // Phase 5: Update row expiry mapping
        let mut new_row_expiry: HashMap<usize, u64> = HashMap::new();
        for (old_idx, expiry) in &self.row_expiry {
            if let Some(&new_idx) = old_to_new.get(old_idx) {
                new_row_expiry.insert(new_idx, *expiry);
            }
        }
        self.row_expiry = new_row_expiry;

        // Phase 6: Clear tombstones and update row count
        stats.tombstones_removed = self.deletion_bitmap.len() as usize;
        self.deletion_bitmap.clear();
        self.row_count = new_row_count;

        stats.completed = true;
        stats.duration_ms = start.elapsed().as_millis() as u64;
        stats
    }

    /// Compacts a single column by removing deleted rows.
    fn compact_column(column: &TypedColumn, deleted: &RoaringBitmap) -> (TypedColumn, u64) {
        let mut bytes_reclaimed = 0u64;
        let deleted_count = deleted.len() as usize;

        match column {
            TypedColumn::Int(data) => {
                let mut new_data = Vec::with_capacity(data.len().saturating_sub(deleted_count));
                for (idx, value) in data.iter().enumerate() {
                    if is_idx_deleted(deleted, idx) {
                        bytes_reclaimed += 8; // i64 size
                    } else {
                        new_data.push(*value);
                    }
                }
                (TypedColumn::Int(new_data), bytes_reclaimed)
            }
            TypedColumn::Float(data) => {
                let mut new_data = Vec::with_capacity(data.len().saturating_sub(deleted_count));
                for (idx, value) in data.iter().enumerate() {
                    if is_idx_deleted(deleted, idx) {
                        bytes_reclaimed += 8; // f64 size
                    } else {
                        new_data.push(*value);
                    }
                }
                (TypedColumn::Float(new_data), bytes_reclaimed)
            }
            TypedColumn::String(data) => {
                let mut new_data = Vec::with_capacity(data.len().saturating_sub(deleted_count));
                for (idx, value) in data.iter().enumerate() {
                    if is_idx_deleted(deleted, idx) {
                        bytes_reclaimed += 4; // StringId size
                    } else {
                        new_data.push(*value);
                    }
                }
                (TypedColumn::String(new_data), bytes_reclaimed)
            }
            TypedColumn::Bool(data) => {
                let mut new_data = Vec::with_capacity(data.len().saturating_sub(deleted_count));
                for (idx, value) in data.iter().enumerate() {
                    if is_idx_deleted(deleted, idx) {
                        bytes_reclaimed += 1; // bool size
                    } else {
                        new_data.push(*value);
                    }
                }
                (TypedColumn::Bool(new_data), bytes_reclaimed)
            }
        }
    }

    /// Returns whether vacuum is recommended based on tombstone ratio.
    ///
    /// # Arguments
    ///
    /// * `threshold` - Ratio of deleted rows to trigger vacuum (0.0-1.0)
    #[must_use]
    pub fn should_vacuum(&self, threshold: f64) -> bool {
        if self.row_count == 0 {
            return false;
        }
        let ratio = self.deletion_bitmap.len() as f64 / self.row_count as f64;
        ratio >= threshold
    }

    // =========================================================================
    // EPIC-043 US-002: RoaringBitmap Filtering
    // =========================================================================

    /// Checks if a row is deleted using `RoaringBitmap` (O(1) lookup).
    ///
    /// Delegates to [`is_deleted`](Self::is_deleted). Kept for backward compatibility.
    #[must_use]
    #[inline]
    pub fn is_row_deleted_bitmap(&self, row_idx: usize) -> bool {
        self.is_deleted(row_idx)
    }

    /// Returns an iterator over live (non-deleted) row indices.
    ///
    /// Uses `RoaringBitmap` for efficient filtering.
    pub fn live_row_indices(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.row_count).filter(|&idx| !self.is_row_deleted_bitmap(idx))
    }

    /// Returns the deletion bitmap for advanced filtering operations.
    #[must_use]
    pub fn deletion_bitmap(&self) -> &RoaringBitmap {
        &self.deletion_bitmap
    }

    /// Returns the number of deleted rows using the bitmap (O(1)).
    #[must_use]
    pub fn deleted_count_bitmap(&self) -> u64 {
        self.deletion_bitmap.len()
    }
}
