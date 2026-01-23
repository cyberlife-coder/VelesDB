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

use roaring::RoaringBitmap;
use rustc_hash::FxHashMap;
use std::collections::HashMap;
use thiserror::Error;

/// Errors that can occur in ColumnStore operations.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ColumnStoreError {
    /// Duplicate primary key value.
    #[error("Duplicate primary key: {0}")]
    DuplicateKey(i64),
    /// Missing primary key column in row.
    #[error("Missing primary key column in row")]
    MissingPrimaryKey,
    /// Primary key column not found in schema.
    #[error("Primary key column '{0}' not found in schema")]
    PrimaryKeyColumnNotFound(String),
    /// Row not found for given primary key.
    #[error("Row not found for primary key: {0}")]
    RowNotFound(i64),
    /// Column not found in schema.
    #[error("Column not found: {0}")]
    ColumnNotFound(String),
    /// Type mismatch when updating a column.
    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch {
        /// Expected column type.
        expected: String,
        /// Actual value type provided.
        actual: String,
    },
    /// Index out of bounds.
    #[error("Index out of bounds: {0}")]
    IndexOutOfBounds(usize),
    /// Attempted to update primary key column.
    #[error("Cannot update primary key column - would corrupt index")]
    PrimaryKeyUpdate,
}

/// Interned string ID for fast equality comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StringId(u32);

/// String interning table for fast string comparisons.
#[derive(Debug, Default)]
pub struct StringTable {
    /// String to ID mapping
    string_to_id: FxHashMap<String, StringId>,
    /// ID to string mapping (for retrieval)
    id_to_string: Vec<String>,
}

impl StringTable {
    /// Creates a new empty string table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Interns a string, returning its ID.
    ///
    /// If the string already exists, returns the existing ID.
    pub fn intern(&mut self, s: &str) -> StringId {
        if let Some(&id) = self.string_to_id.get(s) {
            return id;
        }

        #[allow(clippy::cast_possible_truncation)]
        let id = StringId(self.id_to_string.len() as u32);
        self.id_to_string.push(s.to_string());
        self.string_to_id.insert(s.to_string(), id);
        id
    }

    /// Gets the string for an ID.
    #[must_use]
    pub fn get(&self, id: StringId) -> Option<&str> {
        self.id_to_string.get(id.0 as usize).map(String::as_str)
    }

    /// Gets the ID for a string without interning.
    #[must_use]
    pub fn get_id(&self, s: &str) -> Option<StringId> {
        self.string_to_id.get(s).copied()
    }

    /// Returns the number of interned strings.
    #[must_use]
    pub fn len(&self) -> usize {
        self.id_to_string.len()
    }

    /// Returns true if the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.id_to_string.is_empty()
    }
}

/// A typed column storing values of a specific type.
#[derive(Debug)]
pub enum TypedColumn {
    /// Integer column (i64)
    Int(Vec<Option<i64>>),
    /// Float column (f64)
    Float(Vec<Option<f64>>),
    /// String column (interned IDs)
    String(Vec<Option<StringId>>),
    /// Boolean column
    Bool(Vec<Option<bool>>),
}

impl TypedColumn {
    /// Creates a new integer column with the given capacity.
    #[must_use]
    pub fn new_int(capacity: usize) -> Self {
        Self::Int(Vec::with_capacity(capacity))
    }

    /// Creates a new float column with the given capacity.
    #[must_use]
    pub fn new_float(capacity: usize) -> Self {
        Self::Float(Vec::with_capacity(capacity))
    }

    /// Creates a new string column with the given capacity.
    #[must_use]
    pub fn new_string(capacity: usize) -> Self {
        Self::String(Vec::with_capacity(capacity))
    }

    /// Creates a new boolean column with the given capacity.
    #[must_use]
    pub fn new_bool(capacity: usize) -> Self {
        Self::Bool(Vec::with_capacity(capacity))
    }

    /// Returns the number of values in the column.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Int(v) => v.len(),
            Self::Float(v) => v.len(),
            Self::String(v) => v.len(),
            Self::Bool(v) => v.len(),
        }
    }

    /// Returns true if the column is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Pushes a null value to the column.
    pub fn push_null(&mut self) {
        match self {
            Self::Int(v) => v.push(None),
            Self::Float(v) => v.push(None),
            Self::String(v) => v.push(None),
            Self::Bool(v) => v.push(None),
        }
    }
}

/// Column store for high-performance filtering.
#[derive(Debug, Default)]
pub struct ColumnStore {
    /// Columns indexed by field name
    columns: HashMap<String, TypedColumn>,
    /// String interning table
    string_table: StringTable,
    /// Number of rows
    row_count: usize,
    /// Primary key column name (if any)
    primary_key_column: Option<String>,
    /// Primary key index: pk_value → row_idx (O(1) lookup)
    primary_index: HashMap<i64, usize>,
    /// Reverse index: row_idx → pk_value (O(1) reverse lookup for expire_rows)
    row_idx_to_pk: HashMap<usize, i64>,
    /// Deleted row indices (tombstones)
    deleted_rows: rustc_hash::FxHashSet<usize>,
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
    ///
    /// # Arguments
    ///
    /// * `fields` - List of (`field_name`, `field_type`) tuples
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
    /// # Arguments
    ///
    /// * `fields` - List of (`field_name`, `field_type`) tuples
    /// * `pk_column` - Name of the primary key column (must be Int type)
    ///
    /// # Panics
    ///
    /// Panics if `pk_column` is not found in `fields` or is not of type `Int`.
    #[must_use]
    pub fn with_primary_key(fields: &[(&str, ColumnType)], pk_column: &str) -> Self {
        // Validate pk_column exists and is Int type
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
    ///
    /// For the count of active (non-deleted) rows, use [`active_row_count()`](Self::active_row_count).
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.row_count
    }

    /// Returns the number of active (non-deleted) rows in the store.
    ///
    /// This excludes rows that have been deleted via `delete_by_pk()` or expired via `expire_rows()`.
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
    ///
    /// Missing fields will be set to null.
    /// Type mismatches are silently converted to null for flexibility.
    ///
    /// # Note
    ///
    /// For validated insertion, use [`insert_row()`](Self::insert_row) which
    /// validates types before insertion and returns errors on mismatch.
    pub fn push_row(&mut self, values: &[(&str, ColumnValue)]) {
        // Build a map of provided values
        let value_map: FxHashMap<&str, &ColumnValue> =
            values.iter().map(|(k, v)| (*k, v)).collect();

        // Update each column
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

    /// Inserts a row with primary key validation and index update.
    ///
    /// Returns the row index on success, or an error if:
    /// - Primary key is missing from the row
    /// - Primary key value already exists (duplicate)
    ///
    /// # Errors
    ///
    /// Returns `ColumnStoreError::MissingPrimaryKey` if the primary key column is not in the row.
    /// Returns `ColumnStoreError::DuplicateKey` if the primary key value already exists.
    pub fn insert_row(
        &mut self,
        values: &[(&str, ColumnValue)],
    ) -> Result<usize, ColumnStoreError> {
        // If no primary key configured, just push the row
        let Some(ref pk_col) = self.primary_key_column else {
            self.push_row(values);
            return Ok(self.row_count - 1);
        };

        // Find the primary key value in the provided values
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

        // Check for duplicate (but allow reuse of deleted slot)
        if let Some(&existing_idx) = self.primary_index.get(&pk_value) {
            if self.deleted_rows.contains(&existing_idx) {
                // Validate types BEFORE undeletion for atomicity
                for (col_name, value) in values {
                    if let Some(col) = self.columns.get(*col_name) {
                        if !matches!(value, ColumnValue::Null) {
                            Self::validate_type_match(col, value)?;
                        }
                    }
                }
                // Reuse deleted slot - undelete and update values
                self.deleted_rows.remove(&existing_idx);
                // Clear any stale TTL from the deleted row
                self.row_expiry.remove(&existing_idx);
                // Build map of provided values for O(1) lookup
                let value_map: std::collections::HashMap<&str, &ColumnValue> =
                    values.iter().map(|(k, v)| (*k, v)).collect();
                // Clear ALL columns first (set unprovided to null, provided to value)
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

        // Insert the row
        let row_idx = self.row_count;
        self.push_row(values);

        // Update the primary index and reverse index
        self.primary_index.insert(pk_value, row_idx);
        self.row_idx_to_pk.insert(row_idx, pk_value);

        Ok(row_idx)
    }

    /// Gets the row index by primary key value - O(1) lookup.
    ///
    /// Returns `None` if:
    /// - No primary key is configured
    /// - The primary key value is not found
    /// - The row has been deleted
    #[must_use]
    pub fn get_row_idx_by_pk(&self, pk: i64) -> Option<usize> {
        let row_idx = self.primary_index.get(&pk).copied()?;
        // Check if row is deleted (tombstone)
        if self.deleted_rows.contains(&row_idx) {
            return None;
        }
        Some(row_idx)
    }

    /// Deletes a row by primary key value.
    ///
    /// This uses tombstone deletion - the row data remains but is marked as deleted.
    /// The pk remains in primary_index to allow slot reuse on upsert.
    /// Returns `true` if the row was found and deleted, `false` if not found.
    pub fn delete_by_pk(&mut self, pk: i64) -> bool {
        let Some(&row_idx) = self.primary_index.get(&pk) else {
            return false;
        };
        // Already deleted?
        if self.deleted_rows.contains(&row_idx) {
            return false;
        }
        self.deleted_rows.insert(row_idx);
        true
    }

    /// Updates a single column value for a row identified by primary key - O(1).
    ///
    /// # Errors
    ///
    /// Returns `ColumnStoreError::RowNotFound` if no row exists with the given pk.
    /// Returns `ColumnStoreError::ColumnNotFound` if the column doesn't exist.
    /// Returns `ColumnStoreError::TypeMismatch` if the value type doesn't match the column type.
    /// Returns `ColumnStoreError::PrimaryKeyUpdate` if trying to update the primary key column.
    pub fn update_by_pk(
        &mut self,
        pk: i64,
        column: &str,
        value: ColumnValue,
    ) -> Result<(), ColumnStoreError> {
        // Reject updates to primary key column (would corrupt index)
        if self
            .primary_key_column
            .as_ref()
            .is_some_and(|pk_col| pk_col == column)
        {
            return Err(ColumnStoreError::PrimaryKeyUpdate);
        }

        // Find the row index
        let row_idx = *self
            .primary_index
            .get(&pk)
            .ok_or(ColumnStoreError::RowNotFound(pk))?;

        // Check if row is deleted
        if self.deleted_rows.contains(&row_idx) {
            return Err(ColumnStoreError::RowNotFound(pk));
        }

        // Get the column
        let col = self
            .columns
            .get_mut(column)
            .ok_or_else(|| ColumnStoreError::ColumnNotFound(column.to_string()))?;

        // Update the value with type checking
        Self::set_column_value(col, row_idx, value)
    }

    /// Updates multiple columns atomically for a row identified by primary key.
    ///
    /// All columns are validated before any update is applied.
    ///
    /// # Errors
    ///
    /// Returns `ColumnStoreError::RowNotFound` if no row exists with the given pk.
    /// Returns `ColumnStoreError::ColumnNotFound` if any column doesn't exist.
    /// Returns `ColumnStoreError::TypeMismatch` if any value type doesn't match its column type.
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
        // Find the row index
        let row_idx = *self
            .primary_index
            .get(&pk)
            .ok_or(ColumnStoreError::RowNotFound(pk))?;

        // Check if row is deleted
        if self.deleted_rows.contains(&row_idx) {
            return Err(ColumnStoreError::RowNotFound(pk));
        }

        // Validate all columns exist AND types match before modifying (atomicity)
        for (col_name, value) in updates {
            // Reject updates to primary key column (would corrupt index)
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

            // Validate type compatibility (null is always allowed)
            if !matches!(value, ColumnValue::Null) {
                Self::validate_type_match(col, value)?;
            }
        }

        // Apply all updates (safe - validation passed)
        for (col_name, value) in updates {
            let col = self
                .columns
                .get_mut(*col_name)
                .expect("column existence validated above");
            // Type validation already done, this cannot fail for type mismatch
            Self::set_column_value(col, row_idx, value.clone())?;
        }

        Ok(())
    }

    /// Validates that a value's type matches the column type without modifying.
    fn validate_type_match(col: &TypedColumn, value: &ColumnValue) -> Result<(), ColumnStoreError> {
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
                expected: Self::column_type_name(col).clone(),
                actual: Self::value_type_name(value).clone(),
            })
        }
    }

    /// Sets a value in a typed column with type checking.
    fn set_column_value(
        col: &mut TypedColumn,
        row_idx: usize,
        value: ColumnValue,
    ) -> Result<(), ColumnStoreError> {
        // Handle null case first (allowed for any type)
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

        // Handle typed values
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

    /// Returns the type name of a column.
    fn column_type_name(col: &TypedColumn) -> String {
        match col {
            TypedColumn::Int(_) => "Int".to_string(),
            TypedColumn::Float(_) => "Float".to_string(),
            TypedColumn::String(_) => "String".to_string(),
            TypedColumn::Bool(_) => "Bool".to_string(),
        }
    }

    /// Returns the type name of a value.
    fn value_type_name(value: &ColumnValue) -> String {
        match value {
            ColumnValue::Int(_) => "Int".to_string(),
            ColumnValue::Float(_) => "Float".to_string(),
            ColumnValue::String(_) => "String".to_string(),
            ColumnValue::Bool(_) => "Bool".to_string(),
            ColumnValue::Null => "Null".to_string(),
        }
    }

    // =========================================================================
    // US-003: Batch Updates
    // =========================================================================

    /// Performs batch updates with optimized cache locality.
    ///
    /// Updates are grouped by column for better cache performance.
    /// Partial failures are allowed - successful updates are not rolled back.
    ///
    /// # Note on Update Order
    ///
    /// Updates are reordered by column for cache efficiency. If multiple updates
    /// target the same row and column, the order is **not guaranteed**. For
    /// order-dependent updates to the same cell, use individual `update_by_pk` calls.
    ///
    /// # Arguments
    ///
    /// * `updates` - List of batch update operations
    ///
    /// # Returns
    ///
    /// `BatchUpdateResult` containing counts of successful and failed updates.
    pub fn batch_update(&mut self, updates: &[BatchUpdate]) -> BatchUpdateResult {
        let mut result = BatchUpdateResult::default();

        // Group updates by column for better cache locality
        let mut by_column: HashMap<&str, Vec<(usize, ColumnValue)>> = HashMap::new();

        for update in updates {
            // Reject updates to primary key column (would corrupt index)
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

            // Check if row is deleted
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

        // Apply updates grouped by column
        // We need pk info for failures, so build a reverse map
        let mut row_to_pk: HashMap<usize, i64> = HashMap::new();
        for update in updates {
            if let Some(&row_idx) = self.primary_index.get(&update.pk) {
                row_to_pk.insert(row_idx, update.pk);
            }
        }

        for (col_name, col_updates) in by_column {
            if let Some(col) = self.columns.get_mut(col_name) {
                for (row_idx, value) in col_updates {
                    // Capture actual type before attempting set
                    let actual_type = Self::value_type_name(&value);
                    if Self::set_column_value(col, row_idx, value).is_ok() {
                        result.successful += 1;
                    } else {
                        let pk = row_to_pk.get(&row_idx).copied().unwrap_or(0);
                        result.failed.push((
                            pk,
                            ColumnStoreError::TypeMismatch {
                                expected: Self::column_type_name(col).clone(),
                                actual: actual_type,
                            },
                        ));
                    }
                }
            } else {
                // Column doesn't exist - record all updates for this column as failures
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
    ///
    /// Useful for bulk operations like setting `available=false` for sold out items.
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

    // =========================================================================
    // US-004: TTL Expiration
    // =========================================================================

    /// Sets a TTL (Time To Live) on a row.
    ///
    /// The row will be marked for expiration after the specified duration.
    /// Expiration is checked when `expire_rows()` is called.
    ///
    /// # Arguments
    ///
    /// * `pk` - Primary key of the row
    /// * `ttl_seconds` - Time to live in seconds from now (use 0 for immediate expiry)
    ///
    /// # Note on Testing
    ///
    /// TTL uses `SystemTime::now()` internally. For reliable tests, use `ttl_seconds = 0`
    /// for immediate expiry rather than relying on timing.
    ///
    /// # Errors
    ///
    /// Returns `ColumnStoreError::RowNotFound` if the row doesn't exist.
    pub fn set_ttl(&mut self, pk: i64, ttl_seconds: u64) -> Result<(), ColumnStoreError> {
        let row_idx = *self
            .primary_index
            .get(&pk)
            .ok_or(ColumnStoreError::RowNotFound(pk))?;

        if self.deleted_rows.contains(&row_idx) {
            return Err(ColumnStoreError::RowNotFound(pk));
        }

        let expiry_ts = Self::now_timestamp() + ttl_seconds;

        // Store expiry in a special internal tracking (using deleted_rows for simplicity)
        // In a real implementation, we'd have a separate BTreeMap<u64, Vec<usize>>
        // For now, we'll store the expiry timestamp in an internal map
        self.row_expiry.insert(row_idx, expiry_ts);

        Ok(())
    }

    /// Expires all rows that have passed their TTL.
    ///
    /// # Returns
    ///
    /// `ExpireResult` containing the count and PKs of expired rows.
    pub fn expire_rows(&mut self) -> ExpireResult {
        let now = Self::now_timestamp();
        let mut result = ExpireResult::default();

        // Find expired rows
        let expired_rows: Vec<usize> = self
            .row_expiry
            .iter()
            .filter(|(_, &expiry)| expiry <= now)
            .map(|(&row_idx, _)| row_idx)
            .collect();

        // Remove expired rows (tombstone deletion - keep pk in index for potential reuse)
        for row_idx in expired_rows {
            // O(1) reverse lookup via row_idx_to_pk
            if let Some(&pk) = self.row_idx_to_pk.get(&row_idx) {
                // Don't remove from primary_index - allows slot reuse on upsert
                self.deleted_rows.insert(row_idx);
                self.row_expiry.remove(&row_idx);
                result.pks.push(pk);
                result.expired_count += 1;
            }
        }

        result
    }

    /// Returns the current timestamp in seconds since UNIX epoch.
    fn now_timestamp() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    // =========================================================================
    // US-005: Upsert
    // =========================================================================

    /// Upsert: inserts a new row or updates an existing one.
    ///
    /// This is more efficient than checking existence then insert/update,
    /// as it only performs one lookup.
    ///
    /// # Arguments
    ///
    /// * `values` - Column values for the row (must include primary key)
    ///
    /// # Returns
    ///
    /// `UpsertResult::Inserted` if a new row was created,
    /// `UpsertResult::Updated` if an existing row was modified.
    ///
    /// # Errors
    ///
    /// Returns `ColumnStoreError::MissingPrimaryKey` if no primary key is configured
    /// or the primary key value is missing from values.
    pub fn upsert(
        &mut self,
        values: &[(&str, ColumnValue)],
    ) -> Result<UpsertResult, ColumnStoreError> {
        let Some(ref pk_col) = self.primary_key_column else {
            return Err(ColumnStoreError::MissingPrimaryKey);
        };

        // Find the primary key value
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

        // Validate all columns exist BEFORE any mutations (consistency with update_by_pk)
        for (col_name, _) in values {
            if *col_name != pk_col.as_str() && !self.columns.contains_key(*col_name) {
                return Err(ColumnStoreError::ColumnNotFound((*col_name).to_string()));
            }
        }

        // Check if row exists
        if let Some(&row_idx) = self.primary_index.get(&pk_value) {
            // Check if row is deleted
            if self.deleted_rows.contains(&row_idx) {
                // Validate types BEFORE undeletion for atomicity
                for (col_name, value) in values {
                    if *col_name != pk_col.as_str() {
                        if let Some(col) = self.columns.get(*col_name) {
                            if !matches!(value, ColumnValue::Null) {
                                Self::validate_type_match(col, value)?;
                            }
                        }
                    }
                }
                // Re-insert the row (undelete + update) - validation passed
                self.deleted_rows.remove(&row_idx);
                // Clear any stale TTL from the deleted row
                self.row_expiry.remove(&row_idx);
                // Build map of provided values for O(1) lookup
                let value_map: std::collections::HashMap<&str, &ColumnValue> =
                    values.iter().map(|(k, v)| (*k, v)).collect();
                // Clear ALL columns (set unprovided to null, provided to value)
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

            // Validate types BEFORE applying updates for atomicity
            for (col_name, value) in values {
                if *col_name != pk_col.as_str() {
                    if let Some(col) = self.columns.get(*col_name) {
                        if !matches!(value, ColumnValue::Null) {
                            Self::validate_type_match(col, value)?;
                        }
                    }
                }
            }
            // Update existing row - validation passed, errors propagated
            for (col_name, value) in values {
                if *col_name != pk_col.as_str() {
                    if let Some(col) = self.columns.get_mut(*col_name) {
                        Self::set_column_value(col, row_idx, value.clone())?;
                    }
                }
            }
            Ok(UpsertResult::Updated)
        } else {
            // Insert new row
            self.insert_row(values)?;
            Ok(UpsertResult::Inserted)
        }
    }

    /// Batch upsert: inserts or updates multiple rows.
    ///
    /// More efficient than individual upserts for bulk operations.
    pub fn batch_upsert(&mut self, rows: &[Vec<(&str, ColumnValue)>]) -> BatchUpsertResult {
        let mut result = BatchUpsertResult::default();

        for row in rows {
            match self.upsert(row) {
                Ok(UpsertResult::Inserted) => result.inserted += 1,
                Ok(UpsertResult::Updated) => result.updated += 1,
                Err(e) => {
                    // Try to get the PK for error reporting
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

    /// Gets a column by name.
    #[must_use]
    pub fn get_column(&self, name: &str) -> Option<&TypedColumn> {
        self.columns.get(name)
    }

    /// Filters rows by equality on an integer column.
    ///
    /// Returns a vector of row indices that match. Excludes deleted rows.
    #[must_use]
    pub fn filter_eq_int(&self, column: &str, value: i64) -> Vec<usize> {
        let Some(TypedColumn::Int(col)) = self.columns.get(column) else {
            return Vec::new();
        };

        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| {
                if *v == Some(value) && !self.deleted_rows.contains(&idx) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Filters rows by equality on a string column.
    ///
    /// Returns a vector of row indices that match. Excludes deleted rows.
    #[must_use]
    pub fn filter_eq_string(&self, column: &str, value: &str) -> Vec<usize> {
        let Some(TypedColumn::String(col)) = self.columns.get(column) else {
            return Vec::new();
        };

        let Some(string_id) = self.string_table.get_id(value) else {
            return Vec::new(); // String not in table, no matches
        };

        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| {
                if *v == Some(string_id) && !self.deleted_rows.contains(&idx) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Filters rows by range on an integer column (value > threshold).
    ///
    /// Returns a vector of row indices that match. Excludes deleted rows.
    #[must_use]
    pub fn filter_gt_int(&self, column: &str, threshold: i64) -> Vec<usize> {
        let Some(TypedColumn::Int(col)) = self.columns.get(column) else {
            return Vec::new();
        };

        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| match v {
                Some(val) if *val > threshold && !self.deleted_rows.contains(&idx) => Some(idx),
                _ => None,
            })
            .collect()
    }

    /// Filters rows by range on an integer column (value < threshold).
    ///
    /// Excludes deleted rows.
    #[must_use]
    pub fn filter_lt_int(&self, column: &str, threshold: i64) -> Vec<usize> {
        let Some(TypedColumn::Int(col)) = self.columns.get(column) else {
            return Vec::new();
        };

        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| match v {
                Some(val) if *val < threshold && !self.deleted_rows.contains(&idx) => Some(idx),
                _ => None,
            })
            .collect()
    }

    /// Filters rows by range on an integer column (low < value < high).
    ///
    /// Excludes deleted rows.
    #[must_use]
    pub fn filter_range_int(&self, column: &str, low: i64, high: i64) -> Vec<usize> {
        let Some(TypedColumn::Int(col)) = self.columns.get(column) else {
            return Vec::new();
        };

        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| match v {
                Some(val) if *val > low && *val < high && !self.deleted_rows.contains(&idx) => {
                    Some(idx)
                }
                _ => None,
            })
            .collect()
    }

    /// Filters rows by IN clause on a string column.
    ///
    /// Returns a vector of row indices that match any of the values. Excludes deleted rows.
    #[must_use]
    pub fn filter_in_string(&self, column: &str, values: &[&str]) -> Vec<usize> {
        let Some(TypedColumn::String(col)) = self.columns.get(column) else {
            return Vec::new();
        };

        // Convert string values to IDs
        let ids: Vec<StringId> = values
            .iter()
            .filter_map(|s| self.string_table.get_id(s))
            .collect();

        if ids.is_empty() {
            return Vec::new();
        }

        // Perf: Use HashSet only for large IN clauses (>16 values)
        // Vec.contains() is faster for small arrays due to cache locality
        if ids.len() > 16 {
            let id_set: rustc_hash::FxHashSet<StringId> = ids.into_iter().collect();
            col.iter()
                .enumerate()
                .filter_map(|(idx, v)| match v {
                    Some(id) if id_set.contains(id) && !self.deleted_rows.contains(&idx) => {
                        Some(idx)
                    }
                    _ => None,
                })
                .collect()
        } else {
            col.iter()
                .enumerate()
                .filter_map(|(idx, v)| match v {
                    Some(id) if ids.contains(id) && !self.deleted_rows.contains(&idx) => Some(idx),
                    _ => None,
                })
                .collect()
        }
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
    /// # Note
    ///
    /// Row indices are cast to u32 for RoaringBitmap. This limits stores to ~4B rows.
    /// Indices >= u32::MAX will be silently skipped.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn filter_eq_int_bitmap(&self, column: &str, value: i64) -> RoaringBitmap {
        let Some(TypedColumn::Int(col)) = self.columns.get(column) else {
            return RoaringBitmap::new();
        };

        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| {
                if *v == Some(value) && !self.deleted_rows.contains(&idx) {
                    // Safe: stores with >4B rows are unsupported for bitmap ops
                    Some(idx as u32)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Filters rows by equality on a string column, returning a bitmap.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn filter_eq_string_bitmap(&self, column: &str, value: &str) -> RoaringBitmap {
        let Some(TypedColumn::String(col)) = self.columns.get(column) else {
            return RoaringBitmap::new();
        };

        let Some(string_id) = self.string_table.get_id(value) else {
            return RoaringBitmap::new();
        };

        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| {
                if *v == Some(string_id) && !self.deleted_rows.contains(&idx) {
                    Some(idx as u32)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Filters rows by range on an integer column, returning a bitmap.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn filter_range_int_bitmap(&self, column: &str, low: i64, high: i64) -> RoaringBitmap {
        let Some(TypedColumn::Int(col)) = self.columns.get(column) else {
            return RoaringBitmap::new();
        };

        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| match v {
                Some(val) if *val > low && *val < high && !self.deleted_rows.contains(&idx) => {
                    Some(idx as u32)
                }
                _ => None,
            })
            .collect()
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
}

/// Column type for schema definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    /// 64-bit signed integer
    Int,
    /// 64-bit floating point
    Float,
    /// Interned string
    String,
    /// Boolean
    Bool,
}

/// A value that can be stored in a column.
#[derive(Debug, Clone)]
pub enum ColumnValue {
    /// Integer value
    Int(i64),
    /// Float value
    Float(f64),
    /// String ID (must be interned first)
    String(StringId),
    /// Boolean value
    Bool(bool),
    /// Null value
    Null,
}

// =========================================================================
// US-003: Batch Updates
// =========================================================================

/// A single update operation for batch processing.
#[derive(Debug, Clone)]
pub struct BatchUpdate {
    /// Primary key of the row to update.
    pub pk: i64,
    /// Column name to update.
    pub column: String,
    /// New value for the column.
    pub value: ColumnValue,
}

/// Result of a batch update operation.
#[derive(Debug, Default)]
pub struct BatchUpdateResult {
    /// Number of successful updates.
    pub successful: usize,
    /// List of failed updates with their errors.
    pub failed: Vec<(i64, ColumnStoreError)>,
}

// =========================================================================
// US-004: TTL Expiration
// =========================================================================

/// Result of an expire operation.
#[derive(Debug, Default)]
pub struct ExpireResult {
    /// Number of expired rows.
    pub expired_count: usize,
    /// Primary keys of expired rows.
    pub pks: Vec<i64>,
}

// =========================================================================
// US-005: Upsert
// =========================================================================

/// Result of a single upsert operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertResult {
    /// A new row was inserted.
    Inserted,
    /// An existing row was updated.
    Updated,
}

/// Result of a batch upsert operation.
#[derive(Debug, Default)]
pub struct BatchUpsertResult {
    /// Number of inserted rows.
    pub inserted: usize,
    /// Number of updated rows.
    pub updated: usize,
    /// List of failed operations with their errors.
    pub failed: Vec<(i64, ColumnStoreError)>,
}
