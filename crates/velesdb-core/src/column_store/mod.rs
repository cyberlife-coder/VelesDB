//! Column-oriented storage for high-performance metadata filtering.
//!
//! This module provides a columnar storage format for frequently filtered fields,
//! avoiding the overhead of JSON parsing during filter operations.
//!
//! # Performance Goals
//!
//! - Maintain 50M+ items/sec throughput at 100k items (vs 19M/s with JSON)
//! - Cache-friendly sequential memory access
//! - Support for common filter operations: Eq, Gt, Lt, In, Range
//!
//! # Architecture
//!
//! ```text
//! ColumnStore
//! ├── columns: HashMap<field_name, TypedColumn>
//! │   ├── "category" -> StringColumn(Vec<Option<StringId>>)
//! │   ├── "price"    -> IntColumn(Vec<Option<i64>>)
//! │   └── "rating"   -> FloatColumn(Vec<Option<f64>>)
//! └── string_table: StringTable (interning for strings)
//! ```

mod batch;
mod filter;
mod string_table;
mod types;

use rustc_hash::FxHashMap;
use std::collections::HashMap;

pub use string_table::StringTable;
pub use types::{
    BatchUpdate, BatchUpdateResult, BatchUpsertResult, ColumnStoreError, ColumnType, ColumnValue,
    ExpireResult, StringId, TypedColumn, UpsertResult,
};

/// Column store for high-performance filtering.
#[derive(Debug, Default)]
pub struct ColumnStore {
    /// Columns indexed by field name
    pub(crate) columns: HashMap<String, TypedColumn>,
    /// String interning table
    pub(crate) string_table: StringTable,
    /// Number of rows
    row_count: usize,
    /// Primary key column name (if any)
    primary_key_column: Option<String>,
    /// Primary key index: pk_value → row_idx (O(1) lookup)
    primary_index: HashMap<i64, usize>,
    /// Reverse index: row_idx → pk_value (O(1) reverse lookup for expire_rows)
    row_idx_to_pk: HashMap<usize, i64>,
    /// Deleted row indices (tombstones)
    pub(crate) deleted_rows: rustc_hash::FxHashSet<usize>,
    /// Row expiry timestamps: row_idx → expiry_timestamp (US-004 TTL)
    row_expiry: HashMap<usize, u64>,
}

impl ColumnStore {
    /// Creates a new empty column store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a column store with pre-defined indexed fields.
    #[must_use]
    pub fn with_schema(fields: &[(&str, ColumnType)]) -> Self {
        let mut store = Self::new();
        for (name, col_type) in fields {
            store.add_column(name, *col_type);
        }
        store
    }

    /// Creates a column store with a primary key for O(1) lookups.
    ///
    /// # Panics
    ///
    /// Panics if `pk_column` is not found in `fields` or is not of type `Int`.
    #[must_use]
    pub fn with_primary_key(fields: &[(&str, ColumnType)], pk_column: &str) -> Self {
        let pk_field = fields
            .iter()
            .find(|(name, _)| *name == pk_column)
            .unwrap_or_else(|| {
                panic!(
                    "Primary key column '{}' not found in fields: {:?}",
                    pk_column,
                    fields.iter().map(|(n, _)| *n).collect::<Vec<_>>()
                )
            });
        assert!(
            matches!(pk_field.1, ColumnType::Int),
            "Primary key column '{}' must be Int type, got {:?}",
            pk_column,
            pk_field.1
        );

        let mut store = Self::with_schema(fields);
        store.primary_key_column = Some(pk_column.to_string());
        store.primary_index = HashMap::new();
        store
    }

    /// Returns the primary key column name if set.
    #[must_use]
    pub fn primary_key_column(&self) -> Option<&str> {
        self.primary_key_column.as_deref()
    }

    /// Adds a new column to the store.
    pub fn add_column(&mut self, name: &str, col_type: ColumnType) {
        let column = match col_type {
            ColumnType::Int => TypedColumn::new_int(0),
            ColumnType::Float => TypedColumn::new_float(0),
            ColumnType::String => TypedColumn::new_string(0),
            ColumnType::Bool => TypedColumn::new_bool(0),
        };
        self.columns.insert(name.to_string(), column);
    }

    /// Returns the total number of rows in the store (including deleted/tombstoned rows).
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.row_count
    }

    /// Returns the number of active (non-deleted) rows in the store.
    #[must_use]
    pub fn active_row_count(&self) -> usize {
        self.row_count.saturating_sub(self.deleted_rows.len())
    }

    /// Returns the number of deleted (tombstoned) rows.
    #[must_use]
    pub fn deleted_row_count(&self) -> usize {
        self.deleted_rows.len()
    }

    /// Returns the string table for string interning.
    #[must_use]
    pub fn string_table(&self) -> &StringTable {
        &self.string_table
    }

    /// Returns a mutable reference to the string table.
    pub fn string_table_mut(&mut self) -> &mut StringTable {
        &mut self.string_table
    }

    /// Pushes values for a new row (low-level, no validation).
    pub fn push_row_unchecked(&mut self, values: &[(&str, ColumnValue)]) {
        let value_map: FxHashMap<&str, &ColumnValue> =
            values.iter().map(|(k, v)| (*k, v)).collect();

        for (name, column) in &mut self.columns {
            if let Some(value) = value_map.get(name.as_str()) {
                match value {
                    ColumnValue::Null => column.push_null(),
                    ColumnValue::Int(v) => {
                        if let TypedColumn::Int(col) = column {
                            col.push(Some(*v));
                        } else {
                            column.push_null();
                        }
                    }
                    ColumnValue::Float(v) => {
                        if let TypedColumn::Float(col) = column {
                            col.push(Some(*v));
                        } else {
                            column.push_null();
                        }
                    }
                    ColumnValue::String(id) => {
                        if let TypedColumn::String(col) = column {
                            col.push(Some(*id));
                        } else {
                            column.push_null();
                        }
                    }
                    ColumnValue::Bool(v) => {
                        if let TypedColumn::Bool(col) = column {
                            col.push(Some(*v));
                        } else {
                            column.push_null();
                        }
                    }
                }
            } else {
                column.push_null();
            }
        }
        self.row_count += 1;
    }

    /// Convenience alias for [`push_row_unchecked()`](Self::push_row_unchecked).
    #[inline]
    pub fn push_row(&mut self, values: &[(&str, ColumnValue)]) {
        self.push_row_unchecked(values);
    }

    /// Inserts a row with primary key validation and index update.
    pub fn insert_row(
        &mut self,
        values: &[(&str, ColumnValue)],
    ) -> Result<usize, ColumnStoreError> {
        let Some(ref pk_col) = self.primary_key_column else {
            self.push_row(values);
            return Ok(self.row_count - 1);
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

        if let Some(&existing_idx) = self.primary_index.get(&pk_value) {
            if self.deleted_rows.contains(&existing_idx) {
                for (col_name, value) in values {
                    if let Some(col) = self.columns.get(*col_name) {
                        if !matches!(value, ColumnValue::Null) {
                            Self::validate_type_match(col, value)?;
                        }
                    }
                }
                self.deleted_rows.remove(&existing_idx);
                self.row_expiry.remove(&existing_idx);
                let value_map: std::collections::HashMap<&str, &ColumnValue> =
                    values.iter().map(|(k, v)| (*k, v)).collect();
                let col_names: Vec<String> = self.columns.keys().cloned().collect();
                for col_name in col_names {
                    if let Some(col) = self.columns.get_mut(&col_name) {
                        if let Some(value) = value_map.get(col_name.as_str()) {
                            Self::set_column_value(col, existing_idx, (*value).clone())?;
                        } else {
                            Self::set_column_value(col, existing_idx, ColumnValue::Null)?;
                        }
                    }
                }
                return Ok(existing_idx);
            }
            return Err(ColumnStoreError::DuplicateKey(pk_value));
        }

        let row_idx = self.row_count;
        self.push_row(values);
        self.primary_index.insert(pk_value, row_idx);
        self.row_idx_to_pk.insert(row_idx, pk_value);
        Ok(row_idx)
    }

    /// Gets the row index by primary key value - O(1) lookup.
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
    pub fn delete_by_pk(&mut self, pk: i64) -> bool {
        let Some(&row_idx) = self.primary_index.get(&pk) else {
            return false;
        };
        if self.deleted_rows.contains(&row_idx) {
            return false;
        }
        self.deleted_rows.insert(row_idx);
        self.row_expiry.remove(&row_idx);
        true
    }

    /// Updates a single column value for a row identified by primary key - O(1).
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

        let row_idx = *self
            .primary_index
            .get(&pk)
            .ok_or(ColumnStoreError::RowNotFound(pk))?;

        if self.deleted_rows.contains(&row_idx) {
            return Err(ColumnStoreError::RowNotFound(pk));
        }

        let col = self
            .columns
            .get_mut(column)
            .ok_or_else(|| ColumnStoreError::ColumnNotFound(column.to_string()))?;

        Self::set_column_value(col, row_idx, value)
    }

    /// Updates multiple columns atomically for a row identified by primary key.
    ///
    /// # Panics
    ///
    /// This function will not panic under normal operation. The internal expect
    /// is guarded by prior validation that all columns exist.
    pub fn update_multi_by_pk(
        &mut self,
        pk: i64,
        updates: &[(&str, ColumnValue)],
    ) -> Result<(), ColumnStoreError> {
        let row_idx = *self
            .primary_index
            .get(&pk)
            .ok_or(ColumnStoreError::RowNotFound(pk))?;

        if self.deleted_rows.contains(&row_idx) {
            return Err(ColumnStoreError::RowNotFound(pk));
        }

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

        for (col_name, value) in updates {
            let col = self
                .columns
                .get_mut(*col_name)
                .expect("column existence validated above");
            Self::set_column_value(col, row_idx, value.clone())?;
        }

        Ok(())
    }

    /// Gets a column by name.
    #[must_use]
    pub fn get_column(&self, name: &str) -> Option<&TypedColumn> {
        self.columns.get(name)
    }

    /// Returns an iterator over column names.
    pub fn column_names(&self) -> impl Iterator<Item = &str> {
        self.columns.keys().map(String::as_str)
    }

    /// Gets a value from a column at a specific row index as JSON.
    #[must_use]
    pub fn get_value_as_json(&self, column: &str, row_idx: usize) -> Option<serde_json::Value> {
        if self.deleted_rows.contains(&row_idx) {
            return None;
        }

        let col = self.columns.get(column)?;
        match col {
            TypedColumn::Int(v) => v
                .get(row_idx)
                .and_then(|opt| opt.map(|v| serde_json::json!(v))),
            TypedColumn::Float(v) => v
                .get(row_idx)
                .and_then(|opt| opt.map(|v| serde_json::json!(v))),
            TypedColumn::String(v) => v.get(row_idx).and_then(|opt| {
                opt.and_then(|id| self.string_table.get(id).map(|s| serde_json::json!(s)))
            }),
            TypedColumn::Bool(v) => v
                .get(row_idx)
                .and_then(|opt| opt.map(|v| serde_json::json!(v))),
        }
    }
}
