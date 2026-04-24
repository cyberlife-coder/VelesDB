//! Primary key CRUD operations for `ColumnStore`.
//!
//! Extracted from `mod.rs` for NLOC compliance. Contains insert, update,
//! delete, and validation logic that operates on the primary key index.

use std::collections::HashMap;

use super::haversine;
use super::types::{ColumnStoreError, ColumnValue, TypedColumn};
use super::ColumnStore;

impl ColumnStore {
    /// Inserts a row with primary key validation and index update.
    ///
    /// # Errors
    ///
    /// Returns an error if the primary key is missing, duplicated, or any
    /// provided value does not match the target column type.
    pub fn insert_row(
        &mut self,
        values: &[(&str, ColumnValue)],
    ) -> Result<usize, ColumnStoreError> {
        // Validate GeoPoint coordinates regardless of primary key presence.
        for (_, value) in values {
            if let ColumnValue::GeoPoint(lat, lng) = value {
                haversine::validate_coordinates(*lat, *lng)?;
            }
        }

        let Some(ref pk_col) = self.primary_key_column else {
            self.push_row(values);
            return Ok(self.row_count - 1);
        };

        let pk_value = Self::extract_pk_value(values, pk_col)?;

        if let Some(&existing_idx) = self.primary_index.get(&pk_value) {
            return self.reinsert_or_reject(values, existing_idx, pk_value);
        }

        let row_idx = self.row_count;
        self.push_row(values);
        self.primary_index.insert(pk_value, row_idx);
        self.row_idx_to_pk.insert(row_idx, pk_value);
        Ok(row_idx)
    }

    /// Extracts the integer primary key value from a row's values.
    ///
    /// `pub(super)` because `batch.rs` reuses this for upsert paths.
    pub(super) fn extract_pk_value(
        values: &[(&str, ColumnValue)],
        pk_col: &str,
    ) -> Result<i64, ColumnStoreError> {
        values
            .iter()
            .find(|(name, _)| *name == pk_col)
            .and_then(|(_, value)| {
                if let ColumnValue::Int(v) = value {
                    Some(*v)
                } else {
                    None
                }
            })
            .ok_or(ColumnStoreError::MissingPrimaryKey)
    }

    /// Handles insert into a previously-deleted row slot, or rejects as duplicate.
    fn reinsert_or_reject(
        &mut self,
        values: &[(&str, ColumnValue)],
        existing_idx: usize,
        pk_value: i64,
    ) -> Result<usize, ColumnStoreError> {
        if !self.deleted_rows.contains(&existing_idx) {
            return Err(ColumnStoreError::DuplicateKey(pk_value));
        }

        Self::validate_value_types(&self.columns, values, None)?;
        self.undelete_row(existing_idx);
        self.set_row_values(values, existing_idx, None)?;
        Ok(existing_idx)
    }

    /// Validates that all non-null values match their target column types.
    ///
    /// Optionally skips a column (e.g. primary key). Shared by both `insert_row`
    /// and upsert paths in `batch.rs`.
    pub(super) fn validate_value_types(
        columns: &HashMap<String, TypedColumn>,
        values: &[(&str, ColumnValue)],
        skip_col: Option<&str>,
    ) -> Result<(), ColumnStoreError> {
        for (col_name, value) in values {
            if skip_col.is_some_and(|s| s == *col_name) {
                continue;
            }
            if let ColumnValue::GeoPoint(lat, lng) = value {
                haversine::validate_coordinates(*lat, *lng)?;
            }
            if let Some(col) = columns.get(*col_name) {
                if !matches!(value, ColumnValue::Null) {
                    Self::validate_type_match(col, value)?;
                }
            }
        }
        Ok(())
    }

    /// Marks a tombstoned row as live again.
    fn undelete_row(&mut self, row_idx: usize) {
        self.deleted_rows.remove(&row_idx);
        if let Ok(idx) = u32::try_from(row_idx) {
            self.deletion_bitmap.remove(idx);
        }
        self.row_expiry.remove(&row_idx);
    }

    /// Writes column values for a row, optionally skipping a column (e.g. primary key).
    ///
    /// Missing columns are set to null. Shared by `reinsert_or_reject` and
    /// `write_row_values` (batch module) to eliminate duplication.
    pub(super) fn set_row_values(
        &mut self,
        values: &[(&str, ColumnValue)],
        row_idx: usize,
        skip_col: Option<&str>,
    ) -> Result<(), ColumnStoreError> {
        let value_map: std::collections::HashMap<&str, &ColumnValue> =
            values.iter().map(|(k, v)| (*k, v)).collect();
        let col_names: Vec<String> = self.columns.keys().cloned().collect();
        for col_name in col_names {
            if skip_col.is_some_and(|s| s == col_name) {
                continue;
            }
            if let Some(col) = self.columns.get_mut(&col_name) {
                let val = value_map
                    .get(col_name.as_str())
                    .map_or(ColumnValue::Null, |v| (*v).clone());
                Self::set_column_value(col, row_idx, val)?;
            }
        }
        Ok(())
    }

    /// Gets the row index by primary key value — O(1) lookup.
    #[must_use]
    pub fn get_row_idx_by_pk(&self, pk: i64) -> Option<usize> {
        let row_idx = self.primary_index.get(&pk).copied()?;
        if self.deleted_rows.contains(&row_idx) {
            return None;
        }
        Some(row_idx)
    }

    /// Deletes a row by primary key value.
    ///
    /// Also clears any TTL metadata to prevent false-positive expirations.
    /// Updates both FxHashSet and RoaringBitmap (EPIC-043 US-002).
    pub fn delete_by_pk(&mut self, pk: i64) -> bool {
        let Some(&row_idx) = self.primary_index.get(&pk) else {
            return false;
        };
        if self.deleted_rows.contains(&row_idx) {
            return false;
        }
        self.deleted_rows.insert(row_idx);
        // EPIC-043 US-002: Also update RoaringBitmap for O(1) contains
        if let Ok(idx) = u32::try_from(row_idx) {
            self.deletion_bitmap.insert(idx);
        }
        self.row_expiry.remove(&row_idx);
        true
    }

    /// Updates a single column value for a row identified by primary key — O(1).
    ///
    /// # Errors
    ///
    /// Returns an error if the row does not exist, the column does not exist,
    /// the update targets the primary-key column, or the value type mismatches
    /// the column type.
    pub fn update_by_pk(
        &mut self,
        pk: i64,
        column: &str,
        value: ColumnValue,
    ) -> Result<(), ColumnStoreError> {
        if self
            .primary_key_column
            .as_ref()
            .is_some_and(|pk_col| pk_col == column)
        {
            return Err(ColumnStoreError::PrimaryKeyUpdate);
        }

        let row_idx = self.resolve_live_row(pk)?;

        let col = self
            .columns
            .get_mut(column)
            .ok_or_else(|| ColumnStoreError::ColumnNotFound(column.to_string()))?;

        Self::set_column_value(col, row_idx, value)
    }

    /// Updates multiple columns atomically for a row identified by primary key.
    ///
    /// # Errors
    ///
    /// Returns an error if the row does not exist, one of the columns does not
    /// exist, one update attempts to modify the primary key, or a value type
    /// mismatches its target column type.
    pub fn update_multi_by_pk(
        &mut self,
        pk: i64,
        updates: &[(&str, ColumnValue)],
    ) -> Result<(), ColumnStoreError> {
        let row_idx = self.resolve_live_row(pk)?;
        self.validate_multi_update(updates)?;

        for (col_name, value) in updates {
            let col = self
                .columns
                .get_mut(*col_name)
                .ok_or_else(|| ColumnStoreError::ColumnNotFound((*col_name).to_string()))?;
            Self::set_column_value(col, row_idx, value.clone())?;
        }

        Ok(())
    }

    /// Resolves a primary key to a live (non-deleted) row index.
    ///
    /// `pub(super)` because `batch.rs` reuses this for upsert paths.
    pub(super) fn resolve_live_row(&self, pk: i64) -> Result<usize, ColumnStoreError> {
        let row_idx = *self
            .primary_index
            .get(&pk)
            .ok_or(ColumnStoreError::RowNotFound(pk))?;
        if self.deleted_rows.contains(&row_idx) {
            return Err(ColumnStoreError::RowNotFound(pk));
        }
        Ok(row_idx)
    }

    /// Validates that no update targets the primary key and all types match.
    fn validate_multi_update(
        &self,
        updates: &[(&str, ColumnValue)],
    ) -> Result<(), ColumnStoreError> {
        for (col_name, value) in updates {
            if self
                .primary_key_column
                .as_ref()
                .is_some_and(|pk_col| pk_col == *col_name)
            {
                return Err(ColumnStoreError::PrimaryKeyUpdate);
            }

            let col = self
                .columns
                .get(*col_name)
                .ok_or_else(|| ColumnStoreError::ColumnNotFound((*col_name).to_string()))?;

            if !matches!(value, ColumnValue::Null) {
                Self::validate_type_match(col, value)?;
            }
        }
        Ok(())
    }
}
