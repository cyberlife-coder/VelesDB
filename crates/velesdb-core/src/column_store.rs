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

    /// Returns the number of rows in the store.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.row_count
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

    /// Pushes values for a new row.
    ///
    /// Missing fields will be set to null.
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

    /// Gets a column by name.
    #[must_use]
    pub fn get_column(&self, name: &str) -> Option<&TypedColumn> {
        self.columns.get(name)
    }

    /// Filters rows by equality on an integer column.
    ///
    /// Returns a vector of row indices that match.
    #[must_use]
    pub fn filter_eq_int(&self, column: &str, value: i64) -> Vec<usize> {
        let Some(TypedColumn::Int(col)) = self.columns.get(column) else {
            return Vec::new();
        };

        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| if *v == Some(value) { Some(idx) } else { None })
            .collect()
    }

    /// Filters rows by equality on a string column.
    ///
    /// Returns a vector of row indices that match.
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
                if *v == Some(string_id) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Filters rows by range on an integer column (value > threshold).
    ///
    /// Returns a vector of row indices that match.
    #[must_use]
    pub fn filter_gt_int(&self, column: &str, threshold: i64) -> Vec<usize> {
        let Some(TypedColumn::Int(col)) = self.columns.get(column) else {
            return Vec::new();
        };

        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| match v {
                Some(val) if *val > threshold => Some(idx),
                _ => None,
            })
            .collect()
    }

    /// Filters rows by range on an integer column (value < threshold).
    #[must_use]
    pub fn filter_lt_int(&self, column: &str, threshold: i64) -> Vec<usize> {
        let Some(TypedColumn::Int(col)) = self.columns.get(column) else {
            return Vec::new();
        };

        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| match v {
                Some(val) if *val < threshold => Some(idx),
                _ => None,
            })
            .collect()
    }

    /// Filters rows by range on an integer column (low < value < high).
    #[must_use]
    pub fn filter_range_int(&self, column: &str, low: i64, high: i64) -> Vec<usize> {
        let Some(TypedColumn::Int(col)) = self.columns.get(column) else {
            return Vec::new();
        };

        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| match v {
                Some(val) if *val > low && *val < high => Some(idx),
                _ => None,
            })
            .collect()
    }

    /// Filters rows by IN clause on a string column.
    ///
    /// Returns a vector of row indices that match any of the values.
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
                    Some(id) if id_set.contains(id) => Some(idx),
                    _ => None,
                })
                .collect()
        } else {
            col.iter()
                .enumerate()
                .filter_map(|(idx, v)| match v {
                    Some(id) if ids.contains(id) => Some(idx),
                    _ => None,
                })
                .collect()
        }
    }

    /// Counts rows matching equality on an integer column.
    ///
    /// More efficient than `filter_eq_int().len()` as it doesn't allocate.
    #[must_use]
    pub fn count_eq_int(&self, column: &str, value: i64) -> usize {
        let Some(TypedColumn::Int(col)) = self.columns.get(column) else {
            return 0;
        };

        col.iter().filter(|v| **v == Some(value)).count()
    }

    /// Counts rows matching equality on a string column.
    #[must_use]
    pub fn count_eq_string(&self, column: &str, value: &str) -> usize {
        let Some(TypedColumn::String(col)) = self.columns.get(column) else {
            return 0;
        };

        let Some(string_id) = self.string_table.get_id(value) else {
            return 0;
        };

        col.iter().filter(|v| **v == Some(string_id)).count()
    }

    // =========================================================================
    // Optimized Bitmap-based Filtering (for 100k+ items)
    // =========================================================================

    /// Filters rows by equality on an integer column, returning a bitmap.
    ///
    /// Uses `RoaringBitmap` for memory-efficient storage of matching indices.
    /// Useful for combining multiple filters with AND/OR operations.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn filter_eq_int_bitmap(&self, column: &str, value: i64) -> RoaringBitmap {
        let Some(TypedColumn::Int(col)) = self.columns.get(column) else {
            return RoaringBitmap::new();
        };

        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| {
                if *v == Some(value) {
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
                if *v == Some(string_id) {
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
                Some(val) if *val > low && *val < high => Some(idx as u32),
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
