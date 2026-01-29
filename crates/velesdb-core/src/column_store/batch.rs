//! Batch operations for ColumnStore.
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

            if let Some(&row_idx) = self.primary_index.get(&update.pk) {
                if self.deleted_rows.contains(&row_idx) {
                    result
                        .failed
                        .push((update.pk, ColumnStoreError::RowNotFound(update.pk)));
                    continue;
                }
                by_column
                    .entry(update.column.as_str())
                    .or_default()
                    .push((row_idx, update.value.clone()));
            } else {
                result
                    .failed
                    .push((update.pk, ColumnStoreError::RowNotFound(update.pk)));
            }
        }

        let mut row_to_pk: HashMap<usize, i64> = HashMap::new();
        for update in updates {
            if let Some(&row_idx) = self.primary_index.get(&update.pk) {
                row_to_pk.insert(row_idx, update.pk);
            }
        }

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

        result
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
    pub fn set_ttl(&mut self, pk: i64, ttl_seconds: u64) -> Result<(), ColumnStoreError> {
        let row_idx = *self
            .primary_index
            .get(&pk)
            .ok_or(ColumnStoreError::RowNotFound(pk))?;

        if self.deleted_rows.contains(&row_idx) {
            return Err(ColumnStoreError::RowNotFound(pk));
        }

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
    pub fn upsert(
        &mut self,
        values: &[(&str, ColumnValue)],
    ) -> Result<UpsertResult, ColumnStoreError> {
        let Some(ref pk_col) = self.primary_key_column else {
            return Err(ColumnStoreError::MissingPrimaryKey);
        };

        let pk_value = values
            .iter()
            .find(|(name, _)| *name == pk_col.as_str())
            .and_then(|(_, value)| {
                if let ColumnValue::Int(v) = value {
                    Some(*v)
                } else {
                    None
                }
            })
            .ok_or(ColumnStoreError::MissingPrimaryKey)?;

        for (col_name, _) in values {
            if *col_name != pk_col.as_str() && !self.columns.contains_key(*col_name) {
                return Err(ColumnStoreError::ColumnNotFound((*col_name).to_string()));
            }
        }

        if let Some(&row_idx) = self.primary_index.get(&pk_value) {
            if self.deleted_rows.contains(&row_idx) {
                for (col_name, value) in values {
                    if *col_name != pk_col.as_str() {
                        if let Some(col) = self.columns.get(*col_name) {
                            if !matches!(value, ColumnValue::Null) {
                                Self::validate_type_match(col, value)?;
                            }
                        }
                    }
                }
                self.deleted_rows.remove(&row_idx);
                self.row_expiry.remove(&row_idx);
                let value_map: std::collections::HashMap<&str, &ColumnValue> =
                    values.iter().map(|(k, v)| (*k, v)).collect();
                let col_names: Vec<String> = self.columns.keys().cloned().collect();
                for col_name in col_names {
                    if col_name != *pk_col {
                        if let Some(col) = self.columns.get_mut(&col_name) {
                            if let Some(value) = value_map.get(col_name.as_str()) {
                                Self::set_column_value(col, row_idx, (*value).clone())?;
                            } else {
                                Self::set_column_value(col, row_idx, ColumnValue::Null)?;
                            }
                        }
                    }
                }
                return Ok(UpsertResult::Inserted);
            }

            for (col_name, value) in values {
                if *col_name != pk_col.as_str() {
                    if let Some(col) = self.columns.get(*col_name) {
                        if !matches!(value, ColumnValue::Null) {
                            Self::validate_type_match(col, value)?;
                        }
                    }
                }
            }
            for (col_name, value) in values {
                if *col_name != pk_col.as_str() {
                    if let Some(col) = self.columns.get_mut(*col_name) {
                        Self::set_column_value(col, row_idx, value.clone())?;
                    }
                }
            }
            Ok(UpsertResult::Updated)
        } else {
            self.insert_row(values)?;
            Ok(UpsertResult::Inserted)
        }
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
            match col {
                TypedColumn::Int(vec) => {
                    if row_idx >= vec.len() {
                        return Err(ColumnStoreError::IndexOutOfBounds(row_idx));
                    }
                    vec[row_idx] = None;
                }
                TypedColumn::Float(vec) => {
                    if row_idx >= vec.len() {
                        return Err(ColumnStoreError::IndexOutOfBounds(row_idx));
                    }
                    vec[row_idx] = None;
                }
                TypedColumn::String(vec) => {
                    if row_idx >= vec.len() {
                        return Err(ColumnStoreError::IndexOutOfBounds(row_idx));
                    }
                    vec[row_idx] = None;
                }
                TypedColumn::Bool(vec) => {
                    if row_idx >= vec.len() {
                        return Err(ColumnStoreError::IndexOutOfBounds(row_idx));
                    }
                    vec[row_idx] = None;
                }
            }
            return Ok(());
        }

        match (col, value) {
            (TypedColumn::Int(vec), ColumnValue::Int(v)) => {
                if row_idx >= vec.len() {
                    return Err(ColumnStoreError::IndexOutOfBounds(row_idx));
                }
                vec[row_idx] = Some(v);
                Ok(())
            }
            (TypedColumn::Float(vec), ColumnValue::Float(v)) => {
                if row_idx >= vec.len() {
                    return Err(ColumnStoreError::IndexOutOfBounds(row_idx));
                }
                vec[row_idx] = Some(v);
                Ok(())
            }
            (TypedColumn::String(vec), ColumnValue::String(v)) => {
                if row_idx >= vec.len() {
                    return Err(ColumnStoreError::IndexOutOfBounds(row_idx));
                }
                vec[row_idx] = Some(v);
                Ok(())
            }
            (TypedColumn::Bool(vec), ColumnValue::Bool(v)) => {
                if row_idx >= vec.len() {
                    return Err(ColumnStoreError::IndexOutOfBounds(row_idx));
                }
                vec[row_idx] = Some(v);
                Ok(())
            }
            (col, value) => Err(ColumnStoreError::TypeMismatch {
                expected: Self::column_type_name(col),
                actual: Self::value_type_name(&value),
            }),
        }
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
