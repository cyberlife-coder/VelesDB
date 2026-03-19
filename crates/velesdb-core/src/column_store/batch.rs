//! Batch operations for `ColumnStore`.
//!
//! This module provides batch update, TTL expiration, and upsert operations.

use std::collections::HashMap;

use super::types::{
    BatchUpdate, BatchUpdateResult, BatchUpsertResult, ColumnStoreError, ColumnValue, ExpireResult,
    TypedColumn, UpsertResult,
};
use super::ColumnStore;

impl ColumnStore {
    /// Performs batch updates with optimized cache locality.
    pub fn batch_update(&mut self, updates: &[BatchUpdate]) -> BatchUpdateResult {
        let mut result = BatchUpdateResult::default();
        let by_column = self.partition_batch_updates(updates, &mut result);
        let row_to_pk = self.build_row_to_pk_map(updates);
        self.apply_column_updates(by_column, &row_to_pk, &mut result);
        result
    }

    /// Partitions batch updates by column, rejecting invalid ones into `result.failed`.
    fn partition_batch_updates<'a>(
        &self,
        updates: &'a [BatchUpdate],
        result: &mut BatchUpdateResult,
    ) -> HashMap<&'a str, Vec<(usize, ColumnValue)>> {
        let mut by_column: HashMap<&str, Vec<(usize, ColumnValue)>> = HashMap::new();

        for update in updates {
            if self
                .primary_key_column
                .as_ref()
                .is_some_and(|pk_col| pk_col == &update.column)
            {
                result
                    .failed
                    .push((update.pk, ColumnStoreError::PrimaryKeyUpdate));
                continue;
            }

            match self.primary_index.get(&update.pk) {
                Some(&row_idx) if !self.deleted_rows.contains(&row_idx) => {
                    by_column
                        .entry(update.column.as_str())
                        .or_default()
                        .push((row_idx, update.value.clone()));
                }
                _ => {
                    result
                        .failed
                        .push((update.pk, ColumnStoreError::RowNotFound(update.pk)));
                }
            }
        }

        by_column
    }

    /// Builds a reverse mapping from row index to primary key.
    fn build_row_to_pk_map(&self, updates: &[BatchUpdate]) -> HashMap<usize, i64> {
        let mut row_to_pk: HashMap<usize, i64> = HashMap::new();
        for update in updates {
            if let Some(&row_idx) = self.primary_index.get(&update.pk) {
                row_to_pk.insert(row_idx, update.pk);
            }
        }
        row_to_pk
    }

    /// Applies partitioned column updates, recording successes and failures.
    fn apply_column_updates(
        &mut self,
        by_column: HashMap<&str, Vec<(usize, ColumnValue)>>,
        row_to_pk: &HashMap<usize, i64>,
        result: &mut BatchUpdateResult,
    ) {
        for (col_name, col_updates) in by_column {
            if let Some(col) = self.columns.get_mut(col_name) {
                for (row_idx, value) in col_updates {
                    let actual_type = Self::value_type_name(&value);
                    if Self::set_column_value(col, row_idx, value).is_ok() {
                        result.successful += 1;
                    } else {
                        let pk = row_to_pk.get(&row_idx).copied().unwrap_or(0);
                        result.failed.push((
                            pk,
                            ColumnStoreError::TypeMismatch {
                                expected: Self::column_type_name(col),
                                actual: actual_type,
                            },
                        ));
                    }
                }
            } else {
                for (row_idx, _) in col_updates {
                    let pk = row_to_pk.get(&row_idx).copied().unwrap_or(0);
                    result
                        .failed
                        .push((pk, ColumnStoreError::ColumnNotFound(col_name.to_string())));
                }
            }
        }
    }

    /// Batch update with same value for multiple primary keys.
    pub fn batch_update_same_value(
        &mut self,
        pks: &[i64],
        column: &str,
        value: &ColumnValue,
    ) -> BatchUpdateResult {
        let updates: Vec<BatchUpdate> = pks
            .iter()
            .map(|&pk| BatchUpdate {
                pk,
                column: column.to_string(),
                value: value.clone(),
            })
            .collect();
        self.batch_update(&updates)
    }

    /// Sets a TTL (Time To Live) on a row.
    ///
    /// # Errors
    ///
    /// Returns an error when `pk` is missing or points to a deleted row.
    pub fn set_ttl(&mut self, pk: i64, ttl_seconds: u64) -> Result<(), ColumnStoreError> {
        let row_idx = self.resolve_live_row(pk)?;
        let expiry_ts = Self::now_timestamp() + ttl_seconds;
        self.row_expiry.insert(row_idx, expiry_ts);
        Ok(())
    }

    /// Expires all rows that have passed their TTL.
    pub fn expire_rows(&mut self) -> ExpireResult {
        let now = Self::now_timestamp();
        let mut result = ExpireResult::default();

        let expired_rows: Vec<usize> = self
            .row_expiry
            .iter()
            .filter(|(_, &expiry)| expiry <= now)
            .map(|(&row_idx, _)| row_idx)
            .collect();

        for row_idx in expired_rows {
            if let Some(&pk) = self.row_idx_to_pk.get(&row_idx) {
                self.deleted_rows.insert(row_idx);
                // BUG-2 FIX: Also update RoaringBitmap to keep both in sync
                if let Ok(idx) = u32::try_from(row_idx) {
                    self.deletion_bitmap.insert(idx);
                }
                self.row_expiry.remove(&row_idx);
                result.pks.push(pk);
                result.expired_count += 1;
            }
        }

        result
    }

    /// Upsert: inserts a new row or updates an existing one.
    ///
    /// # Errors
    ///
    /// Returns an error when primary key constraints are violated,
    /// when a referenced column does not exist, or when types mismatch.
    pub fn upsert(
        &mut self,
        values: &[(&str, ColumnValue)],
    ) -> Result<UpsertResult, ColumnStoreError> {
        let pk_col = self
            .primary_key_column
            .clone()
            .ok_or(ColumnStoreError::MissingPrimaryKey)?;

        let pk_value = Self::extract_pk_value(values, &pk_col)?;
        Self::validate_columns_exist(&self.columns, values, &pk_col)?;

        if let Some(&row_idx) = self.primary_index.get(&pk_value) {
            self.upsert_existing_row(values, row_idx, &pk_col)
        } else {
            self.insert_row(values)?;
            Ok(UpsertResult::Inserted)
        }
    }

    /// Validates that all non-pk columns referenced in values actually exist.
    fn validate_columns_exist(
        columns: &HashMap<String, TypedColumn>,
        values: &[(&str, ColumnValue)],
        pk_col: &str,
    ) -> Result<(), ColumnStoreError> {
        for (col_name, _) in values {
            if *col_name != pk_col && !columns.contains_key(*col_name) {
                return Err(ColumnStoreError::ColumnNotFound((*col_name).to_string()));
            }
        }
        Ok(())
    }

    /// Handles upsert for an existing row (either deleted or live).
    fn upsert_existing_row(
        &mut self,
        values: &[(&str, ColumnValue)],
        row_idx: usize,
        pk_col: &str,
    ) -> Result<UpsertResult, ColumnStoreError> {
        if self.deleted_rows.contains(&row_idx) {
            Self::validate_value_types(&self.columns, values, Some(pk_col))?;
            self.deleted_rows.remove(&row_idx);
            if let Ok(idx) = u32::try_from(row_idx) {
                self.deletion_bitmap.remove(idx);
            }
            self.row_expiry.remove(&row_idx);
            self.set_row_values(values, row_idx, Some(pk_col))?;
            return Ok(UpsertResult::Inserted);
        }

        Self::validate_value_types(&self.columns, values, Some(pk_col))?;
        self.update_non_pk_values(values, row_idx, pk_col)?;
        Ok(UpsertResult::Updated)
    }

    /// Updates only the non-pk columns that are provided in values.
    fn update_non_pk_values(
        &mut self,
        values: &[(&str, ColumnValue)],
        row_idx: usize,
        pk_col: &str,
    ) -> Result<(), ColumnStoreError> {
        for (col_name, value) in values {
            if *col_name == pk_col {
                continue;
            }
            if let Some(col) = self.columns.get_mut(*col_name) {
                Self::set_column_value(col, row_idx, value.clone())?;
            }
        }
        Ok(())
    }

    /// Batch upsert: inserts or updates multiple rows.
    pub fn batch_upsert(&mut self, rows: &[Vec<(&str, ColumnValue)>]) -> BatchUpsertResult {
        let mut result = BatchUpsertResult::default();

        for row in rows {
            match self.upsert(row) {
                Ok(UpsertResult::Inserted) => result.inserted += 1,
                Ok(UpsertResult::Updated) => result.updated += 1,
                Err(e) => {
                    let pk = row
                        .iter()
                        .find(|(name, _)| {
                            self.primary_key_column
                                .as_ref()
                                .is_some_and(|pk| pk.as_str() == *name)
                        })
                        .and_then(|(_, v)| {
                            if let ColumnValue::Int(pk) = v {
                                Some(*pk)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    result.failed.push((pk, e));
                }
            }
        }

        result
    }

    pub(super) fn validate_type_match(
        col: &TypedColumn,
        value: &ColumnValue,
    ) -> Result<(), ColumnStoreError> {
        let type_matches = matches!(
            (col, value),
            (TypedColumn::Int(_), ColumnValue::Int(_))
                | (TypedColumn::Float(_), ColumnValue::Float(_))
                | (TypedColumn::String(_), ColumnValue::String(_))
                | (TypedColumn::Bool(_), ColumnValue::Bool(_))
                | (_, ColumnValue::Null)
        );

        if type_matches {
            Ok(())
        } else {
            Err(ColumnStoreError::TypeMismatch {
                expected: Self::column_type_name(col),
                actual: Self::value_type_name(value),
            })
        }
    }

    pub(super) fn set_column_value(
        col: &mut TypedColumn,
        row_idx: usize,
        value: ColumnValue,
    ) -> Result<(), ColumnStoreError> {
        if matches!(value, ColumnValue::Null) {
            return Self::set_column_null(col, row_idx);
        }

        match (col, value) {
            (TypedColumn::Int(vec), ColumnValue::Int(v)) => {
                Self::checked_set(vec, row_idx, Some(v))
            }
            (TypedColumn::Float(vec), ColumnValue::Float(v)) => {
                Self::checked_set(vec, row_idx, Some(v))
            }
            (TypedColumn::String(vec), ColumnValue::String(v)) => {
                Self::checked_set(vec, row_idx, Some(v))
            }
            (TypedColumn::Bool(vec), ColumnValue::Bool(v)) => {
                Self::checked_set(vec, row_idx, Some(v))
            }
            (col, value) => Err(ColumnStoreError::TypeMismatch {
                expected: Self::column_type_name(col),
                actual: Self::value_type_name(&value),
            }),
        }
    }

    /// Sets a column cell to null at the given row index.
    fn set_column_null(col: &mut TypedColumn, row_idx: usize) -> Result<(), ColumnStoreError> {
        match col {
            TypedColumn::Int(vec) => Self::checked_set(vec, row_idx, None),
            TypedColumn::Float(vec) => Self::checked_set(vec, row_idx, None),
            TypedColumn::String(vec) => Self::checked_set(vec, row_idx, None),
            TypedColumn::Bool(vec) => Self::checked_set(vec, row_idx, None),
        }
    }

    /// Sets a value at `row_idx` with bounds checking.
    fn checked_set<T>(
        vec: &mut [Option<T>],
        row_idx: usize,
        value: Option<T>,
    ) -> Result<(), ColumnStoreError> {
        if row_idx >= vec.len() {
            return Err(ColumnStoreError::IndexOutOfBounds(row_idx));
        }
        vec[row_idx] = value;
        Ok(())
    }

    pub(super) fn column_type_name(col: &TypedColumn) -> String {
        match col {
            TypedColumn::Int(_) => "Int".to_string(),
            TypedColumn::Float(_) => "Float".to_string(),
            TypedColumn::String(_) => "String".to_string(),
            TypedColumn::Bool(_) => "Bool".to_string(),
        }
    }

    pub(super) fn value_type_name(value: &ColumnValue) -> String {
        match value {
            ColumnValue::Int(_) => "Int".to_string(),
            ColumnValue::Float(_) => "Float".to_string(),
            ColumnValue::String(_) => "String".to_string(),
            ColumnValue::Bool(_) => "Bool".to_string(),
            ColumnValue::Null => "Null".to_string(),
        }
    }

    pub(super) fn now_timestamp() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

// Tests moved to batch_tests.rs per project rules
