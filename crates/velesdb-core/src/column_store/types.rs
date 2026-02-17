//! Types and enums for column store module.

use thiserror::Error;

/// Errors that can occur in `ColumnStore` operations.
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

// =============================================================================
// EPIC-043 US-001: Vacuum Types
// =============================================================================

/// Configuration for vacuum operation.
#[derive(Debug, Clone)]
pub struct VacuumConfig {
    /// Process tombstones in batches of this size.
    pub batch_size: usize,
    /// Sync to disk after vacuum.
    pub sync: bool,
    /// Yield interval for cooperative multitasking (0 = no yielding).
    pub yield_interval_ms: u64,
}

impl Default for VacuumConfig {
    fn default() -> Self {
        Self {
            batch_size: 10_000,
            sync: true,
            yield_interval_ms: 0,
        }
    }
}

impl VacuumConfig {
    /// Creates a new vacuum config with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: set batch size.
    #[must_use]
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Builder: set sync option.
    #[must_use]
    pub fn with_sync(mut self, sync: bool) -> Self {
        self.sync = sync;
        self
    }
}

/// Statistics from a vacuum operation.
#[derive(Debug, Default, Clone)]
pub struct VacuumStats {
    /// Number of tombstones found.
    pub tombstones_found: usize,
    /// Number of tombstones removed.
    pub tombstones_removed: usize,
    /// Bytes reclaimed (estimated).
    pub bytes_reclaimed: u64,
    /// Duration of the vacuum operation in milliseconds.
    pub duration_ms: u64,
    /// Whether vacuum completed successfully.
    pub completed: bool,
}

// =============================================================================
// EPIC-043 US-003: Auto-Vacuum Configuration
// =============================================================================

/// Configuration for automatic vacuum triggering.
///
/// Based on `PostgreSQL` best practices:
/// - Default threshold: 20% (same as `PostgreSQL` `autovacuum_vacuum_scale_factor`)
/// - Check interval: 300s (5 minutes)
#[derive(Debug, Clone)]
pub struct AutoVacuumConfig {
    /// Enable automatic vacuum.
    pub enabled: bool,
    /// Trigger when deletion ratio exceeds this (0.0-1.0).
    /// `PostgreSQL` default is 0.20 (20%).
    pub threshold_ratio: f64,
    /// Minimum number of deleted rows before considering vacuum.
    /// `PostgreSQL` default is 50.
    pub min_dead_rows: usize,
    /// How often to check for vacuum need (seconds).
    pub check_interval_secs: u64,
}

impl Default for AutoVacuumConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_ratio: 0.20, // PostgreSQL default
            min_dead_rows: 50,     // PostgreSQL default
            check_interval_secs: 300,
        }
    }
}

impl AutoVacuumConfig {
    /// Creates a new auto-vacuum config with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: enable/disable auto-vacuum.
    #[must_use]
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Builder: set threshold ratio (0.0-1.0).
    #[must_use]
    pub fn with_threshold(mut self, threshold: f64) -> Self {
        self.threshold_ratio = threshold.clamp(0.0, 1.0);
        self
    }

    /// Builder: set minimum dead rows.
    #[must_use]
    pub fn with_min_dead_rows(mut self, min: usize) -> Self {
        self.min_dead_rows = min;
        self
    }

    /// Builder: set check interval in seconds.
    #[must_use]
    pub fn with_check_interval(mut self, secs: u64) -> Self {
        self.check_interval_secs = secs;
        self
    }

    /// Checks if vacuum should be triggered based on current stats.
    #[must_use]
    pub fn should_trigger(&self, row_count: usize, deleted_count: usize) -> bool {
        if !self.enabled || row_count == 0 {
            return false;
        }

        // Must have minimum dead rows
        if deleted_count < self.min_dead_rows {
            return false;
        }

        // Check threshold ratio
        let ratio = deleted_count as f64 / row_count as f64;
        ratio >= self.threshold_ratio
    }
}
