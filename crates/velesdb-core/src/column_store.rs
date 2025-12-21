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

        col.iter()
            .enumerate()
            .filter_map(|(idx, v)| match v {
                Some(id) if ids.contains(id) => Some(idx),
                _ => None,
            })
            .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // TDD Tests for StringTable
    // =========================================================================

    #[test]
    fn test_string_table_intern() {
        // Arrange
        let mut table = StringTable::new();

        // Act
        let id1 = table.intern("hello");
        let id2 = table.intern("world");
        let id3 = table.intern("hello"); // Same as id1

        // Assert
        assert_eq!(id1, id3);
        assert_ne!(id1, id2);
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn test_string_table_get() {
        // Arrange
        let mut table = StringTable::new();
        let id = table.intern("test");

        // Act & Assert
        assert_eq!(table.get(id), Some("test"));
    }

    #[test]
    fn test_string_table_get_id() {
        // Arrange
        let mut table = StringTable::new();
        table.intern("existing");

        // Act & Assert
        assert!(table.get_id("existing").is_some());
        assert!(table.get_id("missing").is_none());
    }

    // =========================================================================
    // TDD Tests for ColumnStore - Basic Operations
    // =========================================================================

    #[test]
    fn test_column_store_new() {
        // Arrange & Act
        let store = ColumnStore::new();

        // Assert
        assert_eq!(store.row_count(), 0);
    }

    #[test]
    fn test_column_store_with_schema() {
        // Arrange & Act
        let store = ColumnStore::with_schema(&[
            ("category", ColumnType::String),
            ("price", ColumnType::Int),
        ]);

        // Assert
        assert!(store.get_column("category").is_some());
        assert!(store.get_column("price").is_some());
        assert!(store.get_column("missing").is_none());
    }

    #[test]
    fn test_column_store_push_row() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[
            ("category", ColumnType::String),
            ("price", ColumnType::Int),
        ]);

        let cat_id = store.string_table_mut().intern("tech");

        // Act
        store.push_row(&[
            ("category", ColumnValue::String(cat_id)),
            ("price", ColumnValue::Int(100)),
        ]);

        // Assert
        assert_eq!(store.row_count(), 1);
    }

    // =========================================================================
    // TDD Tests for ColumnStore - Filtering
    // =========================================================================

    #[test]
    fn test_filter_eq_int() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Int(200))]);
        store.push_row(&[("price", ColumnValue::Int(100))]);

        // Act
        let matches = store.filter_eq_int("price", 100);

        // Assert
        assert_eq!(matches, vec![0, 2]);
    }

    #[test]
    fn test_filter_eq_string() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("category", ColumnType::String)]);

        let tech_id = store.string_table_mut().intern("tech");
        let science_id = store.string_table_mut().intern("science");

        store.push_row(&[("category", ColumnValue::String(tech_id))]);
        store.push_row(&[("category", ColumnValue::String(science_id))]);
        store.push_row(&[("category", ColumnValue::String(tech_id))]);

        // Act
        let matches = store.filter_eq_string("category", "tech");

        // Assert
        assert_eq!(matches, vec![0, 2]);
    }

    #[test]
    fn test_filter_gt_int() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(50))]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Int(150))]);

        // Act
        let matches = store.filter_gt_int("price", 75);

        // Assert
        assert_eq!(matches, vec![1, 2]);
    }

    #[test]
    fn test_filter_lt_int() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(50))]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Int(150))]);

        // Act
        let matches = store.filter_lt_int("price", 100);

        // Assert
        assert_eq!(matches, vec![0]);
    }

    #[test]
    fn test_filter_range_int() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(50))]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Int(150))]);
        store.push_row(&[("price", ColumnValue::Int(200))]);

        // Act
        let matches = store.filter_range_int("price", 75, 175);

        // Assert
        assert_eq!(matches, vec![1, 2]);
    }

    #[test]
    fn test_filter_in_string() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("category", ColumnType::String)]);

        let tech_id = store.string_table_mut().intern("tech");
        let science_id = store.string_table_mut().intern("science");
        let art_id = store.string_table_mut().intern("art");

        store.push_row(&[("category", ColumnValue::String(tech_id))]);
        store.push_row(&[("category", ColumnValue::String(science_id))]);
        store.push_row(&[("category", ColumnValue::String(art_id))]);
        store.push_row(&[("category", ColumnValue::String(tech_id))]);

        // Act
        let matches = store.filter_in_string("category", &["tech", "art"]);

        // Assert
        assert_eq!(matches, vec![0, 2, 3]);
    }

    #[test]
    fn test_filter_with_null_values() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Null)]);
        store.push_row(&[("price", ColumnValue::Int(100))]);

        // Act
        let matches = store.filter_eq_int("price", 100);

        // Assert - nulls should not match
        assert_eq!(matches, vec![0, 2]);
    }

    #[test]
    fn test_filter_missing_column() {
        // Arrange
        let store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);

        // Act
        let matches = store.filter_eq_int("missing", 100);

        // Assert
        assert!(matches.is_empty());
    }

    // =========================================================================
    // TDD Tests for ColumnStore - Count Operations
    // =========================================================================

    #[test]
    fn test_count_eq_int() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Int(200))]);
        store.push_row(&[("price", ColumnValue::Int(100))]);

        // Act
        let count = store.count_eq_int("price", 100);

        // Assert
        assert_eq!(count, 2);
    }

    #[test]
    fn test_count_eq_string() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("category", ColumnType::String)]);

        let tech_id = store.string_table_mut().intern("tech");
        let science_id = store.string_table_mut().intern("science");

        store.push_row(&[("category", ColumnValue::String(tech_id))]);
        store.push_row(&[("category", ColumnValue::String(science_id))]);
        store.push_row(&[("category", ColumnValue::String(tech_id))]);

        // Act
        let count = store.count_eq_string("category", "tech");

        // Assert
        assert_eq!(count, 2);
    }

    // =========================================================================
    // TDD Tests for Bitmap Operations
    // =========================================================================

    #[test]
    fn test_filter_eq_int_bitmap() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Int(200))]);
        store.push_row(&[("price", ColumnValue::Int(100))]);

        // Act
        let bitmap = store.filter_eq_int_bitmap("price", 100);

        // Assert
        assert!(bitmap.contains(0));
        assert!(!bitmap.contains(1));
        assert!(bitmap.contains(2));
        assert_eq!(bitmap.len(), 2);
    }

    #[test]
    fn test_filter_eq_string_bitmap() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("category", ColumnType::String)]);

        let tech_id = store.string_table_mut().intern("tech");
        let science_id = store.string_table_mut().intern("science");

        store.push_row(&[("category", ColumnValue::String(tech_id))]);
        store.push_row(&[("category", ColumnValue::String(science_id))]);
        store.push_row(&[("category", ColumnValue::String(tech_id))]);

        // Act
        let bitmap = store.filter_eq_string_bitmap("category", "tech");

        // Assert
        assert!(bitmap.contains(0));
        assert!(!bitmap.contains(1));
        assert!(bitmap.contains(2));
        assert_eq!(bitmap.len(), 2);
    }

    #[test]
    fn test_filter_range_int_bitmap() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(50))]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Int(150))]);
        store.push_row(&[("price", ColumnValue::Int(200))]);

        // Act
        let bitmap = store.filter_range_int_bitmap("price", 75, 175);

        // Assert
        assert!(!bitmap.contains(0));
        assert!(bitmap.contains(1));
        assert!(bitmap.contains(2));
        assert!(!bitmap.contains(3));
        assert_eq!(bitmap.len(), 2);
    }

    #[test]
    fn test_bitmap_and() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[
            ("price", ColumnType::Int),
            ("category", ColumnType::String),
        ]);

        let tech_id = store.string_table_mut().intern("tech");
        let science_id = store.string_table_mut().intern("science");

        store.push_row(&[
            ("price", ColumnValue::Int(100)),
            ("category", ColumnValue::String(tech_id)),
        ]);
        store.push_row(&[
            ("price", ColumnValue::Int(200)),
            ("category", ColumnValue::String(tech_id)),
        ]);
        store.push_row(&[
            ("price", ColumnValue::Int(100)),
            ("category", ColumnValue::String(science_id)),
        ]);

        // Act
        let price_bitmap = store.filter_eq_int_bitmap("price", 100);
        let category_bitmap = store.filter_eq_string_bitmap("category", "tech");
        let combined = ColumnStore::bitmap_and(&price_bitmap, &category_bitmap);

        // Assert - only row 0 matches both conditions
        assert!(combined.contains(0));
        assert!(!combined.contains(1));
        assert!(!combined.contains(2));
        assert_eq!(combined.len(), 1);
    }

    #[test]
    fn test_bitmap_or() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[
            ("price", ColumnType::Int),
            ("category", ColumnType::String),
        ]);

        let tech_id = store.string_table_mut().intern("tech");
        let science_id = store.string_table_mut().intern("science");

        store.push_row(&[
            ("price", ColumnValue::Int(100)),
            ("category", ColumnValue::String(tech_id)),
        ]);
        store.push_row(&[
            ("price", ColumnValue::Int(200)),
            ("category", ColumnValue::String(science_id)),
        ]);
        store.push_row(&[
            ("price", ColumnValue::Int(300)),
            ("category", ColumnValue::String(science_id)),
        ]);

        // Act
        let price_bitmap = store.filter_eq_int_bitmap("price", 100);
        let category_bitmap = store.filter_eq_string_bitmap("category", "science");
        let combined = ColumnStore::bitmap_or(&price_bitmap, &category_bitmap);

        // Assert - rows 0, 1, 2 match (0 for price, 1 and 2 for category)
        assert!(combined.contains(0));
        assert!(combined.contains(1));
        assert!(combined.contains(2));
        assert_eq!(combined.len(), 3);
    }

    #[test]
    fn test_filter_bitmap_missing_column() {
        // Arrange
        let store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);

        // Act
        let bitmap = store.filter_eq_int_bitmap("missing", 100);

        // Assert
        assert!(bitmap.is_empty());
    }

    #[test]
    fn test_filter_bitmap_missing_string_value() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("category", ColumnType::String)]);
        let tech_id = store.string_table_mut().intern("tech");
        store.push_row(&[("category", ColumnValue::String(tech_id))]);

        // Act - search for a string that was never interned
        let bitmap = store.filter_eq_string_bitmap("category", "nonexistent");

        // Assert
        assert!(bitmap.is_empty());
    }

    #[test]
    fn test_count_eq_string_missing_value() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("category", ColumnType::String)]);
        let tech_id = store.string_table_mut().intern("tech");
        store.push_row(&[("category", ColumnValue::String(tech_id))]);

        // Act - count a string that was never interned
        let count = store.count_eq_string("category", "nonexistent");

        // Assert
        assert_eq!(count, 0);
    }

    #[test]
    fn test_add_column() {
        // Arrange
        let mut store = ColumnStore::new();

        // Act
        store.add_column("price", ColumnType::Int);
        store.add_column("rating", ColumnType::Float);

        // Assert
        assert!(store.get_column("price").is_some());
        assert!(store.get_column("rating").is_some());
    }
}
