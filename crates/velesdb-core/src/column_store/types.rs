//! Types and enums for column store module.

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
pub struct StringId(pub(crate) u32);

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

/// Result of an expire operation.
#[derive(Debug, Default)]
pub struct ExpireResult {
    /// Number of expired rows.
    pub expired_count: usize,
    /// Primary keys of expired rows.
    pub pks: Vec<i64>,
}

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
