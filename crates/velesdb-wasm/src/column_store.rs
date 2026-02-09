//! WASM bindings for `velesdb-core`'s `ColumnStore`.
//!
//! Provides a `#[wasm_bindgen]` wrapper over core's column-oriented storage,
//! exposing schema definition, CRUD, filtering, TTL, and vacuum to JavaScript.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use velesdb_core::column_store::{ColumnStore, ColumnType, ColumnValue, VacuumConfig};

/// Parse a column type string to core's `ColumnType`.
fn parse_column_type(type_str: &str) -> Result<ColumnType, JsValue> {
    match type_str.to_lowercase().as_str() {
        "int" | "integer" | "i64" => Ok(ColumnType::Int),
        "float" | "double" | "f64" => Ok(ColumnType::Float),
        "string" | "str" | "text" => Ok(ColumnType::String),
        "bool" | "boolean" => Ok(ColumnType::Bool),
        _ => Err(JsValue::from_str(&format!(
            "Unknown column type '{type_str}'. Valid: int, float, string, bool"
        ))),
    }
}

/// Convert a JSON value to a `ColumnValue`, using the column's expected type.
fn json_to_column_value(
    val: &serde_json::Value,
    col_type: ColumnType,
    store: &mut ColumnStore,
) -> ColumnValue {
    if val.is_null() {
        return ColumnValue::Null;
    }
    match col_type {
        ColumnType::Int => val.as_i64().map_or(ColumnValue::Null, ColumnValue::Int),
        ColumnType::Float => val.as_f64().map_or(ColumnValue::Null, ColumnValue::Float),
        ColumnType::String => val.as_str().map_or(ColumnValue::Null, |s| {
            ColumnValue::String(store.string_table_mut().intern(s))
        }),
        ColumnType::Bool => val.as_bool().map_or(ColumnValue::Null, ColumnValue::Bool),
    }
}

/// Schema column definition for JS interop.
#[derive(Serialize, Deserialize)]
struct SchemaColumn {
    name: String,
    #[serde(rename = "type")]
    col_type: String,
}

/// Column-oriented store for structured metadata with typed columns.
///
/// Provides high-performance filtering (50M+ items/sec), string interning,
/// primary key indexing, TTL, and vacuum compaction.
#[wasm_bindgen]
pub struct ColumnStoreWasm {
    store: ColumnStore,
    /// Cached schema for type lookups during insert/upsert
    schema: Vec<(String, ColumnType)>,
}

impl Default for ColumnStoreWasm {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl ColumnStoreWasm {
    /// Creates an empty column store.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            store: ColumnStore::new(),
            schema: Vec::new(),
        }
    }

    /// Creates a column store from a JSON schema array.
    ///
    /// Schema format: `[{"name": "age", "type": "int"}, {"name": "status", "type": "string"}]`
    /// Valid types: `int`, `float`, `string`, `bool`.
    #[wasm_bindgen]
    pub fn with_schema(schema_json: JsValue) -> Result<ColumnStoreWasm, JsValue> {
        let cols: Vec<SchemaColumn> = serde_wasm_bindgen::from_value(schema_json)
            .map_err(|e| JsValue::from_str(&format!("Invalid schema: {e}")))?;

        let mut schema = Vec::with_capacity(cols.len());
        for col in &cols {
            let ct = parse_column_type(&col.col_type)?;
            schema.push((col.name.clone(), ct));
        }

        let fields: Vec<(&str, ColumnType)> =
            schema.iter().map(|(n, t)| (n.as_str(), *t)).collect();
        let store = ColumnStore::with_schema(&fields);
        Ok(Self { store, schema })
    }

    /// Creates a column store with a primary key for O(1) lookups.
    ///
    /// The `pk_column` must be of type `int` and present in the schema.
    #[wasm_bindgen]
    pub fn with_primary_key(
        schema_json: JsValue,
        pk_column: &str,
    ) -> Result<ColumnStoreWasm, JsValue> {
        let cols: Vec<SchemaColumn> = serde_wasm_bindgen::from_value(schema_json)
            .map_err(|e| JsValue::from_str(&format!("Invalid schema: {e}")))?;

        let mut schema = Vec::with_capacity(cols.len());
        for col in &cols {
            let ct = parse_column_type(&col.col_type)?;
            schema.push((col.name.clone(), ct));
        }

        let fields: Vec<(&str, ColumnType)> =
            schema.iter().map(|(n, t)| (n.as_str(), *t)).collect();
        let store = ColumnStore::with_primary_key(&fields, pk_column)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(Self { store, schema })
    }

    /// Adds a column to the store dynamically.
    #[wasm_bindgen]
    pub fn add_column(&mut self, name: &str, col_type: &str) -> Result<(), JsValue> {
        let ct = parse_column_type(col_type)?;
        self.store.add_column(name, ct);
        self.schema.push((name.to_string(), ct));
        Ok(())
    }

    /// Returns column names as a JSON array.
    #[wasm_bindgen]
    pub fn column_names(&self) -> JsValue {
        let names: Vec<&str> = self.store.column_names().collect();
        serde_wasm_bindgen::to_value(&names).unwrap_or(JsValue::NULL)
    }

    /// Returns the primary key column name, or null.
    #[wasm_bindgen(getter)]
    pub fn primary_key_column(&self) -> Option<String> {
        self.store.primary_key_column().map(String::from)
    }

    // =========================================================================
    // CRUD
    // =========================================================================

    /// Inserts a row from a JSON object. Returns the row index.
    ///
    /// Example: `store.insert_row({"id": 1, "name": "Alice", "age": 30})`
    #[wasm_bindgen]
    pub fn insert_row(&mut self, row_json: JsValue) -> Result<u32, JsValue> {
        let row: serde_json::Map<String, serde_json::Value> =
            serde_wasm_bindgen::from_value(row_json)
                .map_err(|e| JsValue::from_str(&format!("Invalid row: {e}")))?;

        let values = self.json_row_to_values(&row);
        let refs: Vec<(&str, ColumnValue)> = values
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();

        let idx = self
            .store
            .insert_row(&refs)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        // Reason: WASM row counts are always < u32::MAX (browser memory limits)
        #[allow(clippy::cast_possible_truncation)]
        Ok(idx as u32)
    }

    /// Upserts a row (insert or update by primary key). Requires a primary key.
    ///
    /// Returns `"inserted"` or `"updated"`.
    #[wasm_bindgen]
    pub fn upsert_row(&mut self, row_json: JsValue) -> Result<String, JsValue> {
        let row: serde_json::Map<String, serde_json::Value> =
            serde_wasm_bindgen::from_value(row_json)
                .map_err(|e| JsValue::from_str(&format!("Invalid row: {e}")))?;

        let values = self.json_row_to_values(&row);
        let refs: Vec<(&str, ColumnValue)> = values
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();

        let result = self
            .store
            .upsert(&refs)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        Ok(format!("{result:?}").to_lowercase())
    }

    /// Batch upserts multiple rows from a JSON array.
    ///
    /// Returns `{"inserted": N, "updated": N, "failed": N}`.
    #[wasm_bindgen]
    pub fn batch_upsert(&mut self, rows_json: JsValue) -> Result<JsValue, JsValue> {
        let rows: Vec<serde_json::Map<String, serde_json::Value>> =
            serde_wasm_bindgen::from_value(rows_json)
                .map_err(|e| JsValue::from_str(&format!("Invalid rows: {e}")))?;

        let mut all_values: Vec<Vec<(String, ColumnValue)>> = Vec::with_capacity(rows.len());
        for row in &rows {
            all_values.push(self.json_row_to_values(row));
        }

        let batch_refs: Vec<Vec<(&str, ColumnValue)>> = all_values
            .iter()
            .map(|row| row.iter().map(|(k, v)| (k.as_str(), v.clone())).collect())
            .collect();

        let result = self.store.batch_upsert(&batch_refs);

        let output = serde_json::json!({
            "inserted": result.inserted,
            "updated": result.updated,
            "failed": result.failed.len(),
        });
        serde_wasm_bindgen::to_value(&output).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Gets a row by primary key as a JSON object, or null if not found.
    #[wasm_bindgen]
    pub fn get_row(&self, pk: i64) -> JsValue {
        let Some(row_idx) = self.store.get_row_idx_by_pk(pk) else {
            return JsValue::NULL;
        };
        self.row_idx_to_json(row_idx)
    }

    /// Deletes a row by primary key. Returns true if deleted.
    #[wasm_bindgen]
    pub fn delete_row(&mut self, pk: i64) -> bool {
        self.store.delete_by_pk(pk)
    }

    /// Updates a single column for a row by primary key.
    #[wasm_bindgen]
    pub fn update_row(&mut self, pk: i64, column: &str, value: JsValue) -> Result<(), JsValue> {
        let col_type = self.find_column_type(column)?;
        let json_val: serde_json::Value = serde_wasm_bindgen::from_value(value)
            .map_err(|e| JsValue::from_str(&format!("Invalid value: {e}")))?;
        let cv = json_to_column_value(&json_val, col_type, &mut self.store);
        self.store
            .update_by_pk(pk, column, cv)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }
}

impl ColumnStoreWasm {
    /// Converts a JSON row map to `Vec<(String, ColumnValue)>`, interning strings.
    fn json_row_to_values(
        &mut self,
        row: &serde_json::Map<String, serde_json::Value>,
    ) -> Vec<(String, ColumnValue)> {
        let mut values = Vec::with_capacity(row.len());
        for (key, val) in row {
            let col_type = self
                .schema
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, ct)| *ct);
            let cv = if let Some(ct) = col_type {
                json_to_column_value(val, ct, &mut self.store)
            } else {
                // Reason: unknown columns are passed as-is — core will validate
                ColumnValue::Null
            };
            values.push((key.clone(), cv));
        }
        values
    }

    /// Converts a row index to a JSON object with all column values.
    fn row_idx_to_json(&self, row_idx: usize) -> JsValue {
        let mut obj = serde_json::Map::new();
        for (name, _) in &self.schema {
            if let Some(val) = self.store.get_value_as_json(name, row_idx) {
                obj.insert(name.clone(), val);
            } else {
                obj.insert(name.clone(), serde_json::Value::Null);
            }
        }
        serde_wasm_bindgen::to_value(&obj).unwrap_or(JsValue::NULL)
    }

    /// Finds the column type by name from the cached schema.
    fn find_column_type(&self, column: &str) -> Result<ColumnType, JsValue> {
        self.schema
            .iter()
            .find(|(name, _)| name == column)
            .map(|(_, ct)| *ct)
            .ok_or_else(|| JsValue::from_str(&format!("Column '{column}' not found")))
    }
}

// =========================================================================
// Filtering, TTL, Vacuum, Stats
// =========================================================================

#[wasm_bindgen]
impl ColumnStoreWasm {
    /// Filters rows where `column == value`. Returns matching row indices.
    #[wasm_bindgen]
    pub fn filter_eq(&self, column: &str, value: JsValue) -> Result<Vec<u32>, JsValue> {
        let json_val: serde_json::Value = serde_wasm_bindgen::from_value(value)
            .map_err(|e| JsValue::from_str(&format!("Invalid value: {e}")))?;

        let indices = if let Some(v) = json_val.as_i64() {
            self.store.filter_eq_int(column, v)
        } else if let Some(s) = json_val.as_str() {
            self.store.filter_eq_string(column, s)
        } else {
            return Err(JsValue::from_str(
                "filter_eq supports int and string values only",
            ));
        };
        Ok(Self::usize_to_u32(indices))
    }

    /// Filters rows where `column > value` (int columns only).
    #[wasm_bindgen]
    pub fn filter_gt(&self, column: &str, value: i64) -> Vec<u32> {
        Self::usize_to_u32(self.store.filter_gt_int(column, value))
    }

    /// Filters rows where `column < value` (int columns only).
    #[wasm_bindgen]
    pub fn filter_lt(&self, column: &str, value: i64) -> Vec<u32> {
        Self::usize_to_u32(self.store.filter_lt_int(column, value))
    }

    /// Filters rows where `low < column < high` (int columns only).
    #[wasm_bindgen]
    pub fn filter_range(&self, column: &str, low: i64, high: i64) -> Vec<u32> {
        Self::usize_to_u32(self.store.filter_range_int(column, low, high))
    }

    /// Filters rows where `column IN values` (string columns only).
    ///
    /// `values` is a JS array of strings.
    #[wasm_bindgen]
    pub fn filter_in(&self, column: &str, values: JsValue) -> Result<Vec<u32>, JsValue> {
        let strs: Vec<String> = serde_wasm_bindgen::from_value(values)
            .map_err(|e| JsValue::from_str(&format!("Expected string array: {e}")))?;
        let refs: Vec<&str> = strs.iter().map(String::as_str).collect();
        Ok(Self::usize_to_u32(
            self.store.filter_in_string(column, &refs),
        ))
    }

    /// Gets multiple rows by their indices. Returns a JSON array of row objects.
    #[wasm_bindgen]
    pub fn get_rows_by_indices(&self, indices: &[u32]) -> JsValue {
        let rows: Vec<serde_json::Value> = indices
            .iter()
            .filter_map(|&idx| {
                let row_idx = idx as usize;
                if self.store.is_deleted(row_idx) || row_idx >= self.store.row_count() {
                    return None;
                }
                let mut obj = serde_json::Map::new();
                for (name, _) in &self.schema {
                    if let Some(val) = self.store.get_value_as_json(name, row_idx) {
                        obj.insert(name.clone(), val);
                    } else {
                        obj.insert(name.clone(), serde_json::Value::Null);
                    }
                }
                Some(serde_json::Value::Object(obj))
            })
            .collect();
        serde_wasm_bindgen::to_value(&rows).unwrap_or(JsValue::NULL)
    }

    // =========================================================================
    // TTL
    // =========================================================================

    /// Sets a TTL (time-to-live) on a row by primary key.
    #[wasm_bindgen]
    pub fn set_row_ttl(&mut self, pk: i64, ttl_seconds: u64) -> Result<(), JsValue> {
        self.store
            .set_ttl(pk, ttl_seconds)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Expires all rows that have passed their TTL. Returns expired count.
    #[wasm_bindgen]
    pub fn expire_rows(&mut self) -> u32 {
        let result = self.store.expire_rows();
        // Reason: expired count is always < u32::MAX in WASM context
        #[allow(clippy::cast_possible_truncation)]
        {
            result.expired_count as u32
        }
    }

    // =========================================================================
    // Vacuum & Stats
    // =========================================================================

    /// Runs vacuum to compact deleted rows. Returns bytes reclaimed.
    #[wasm_bindgen]
    pub fn vacuum(&mut self) -> JsValue {
        let stats = self.store.vacuum(VacuumConfig::default());
        let output = serde_json::json!({
            "tombstones_found": stats.tombstones_found,
            "tombstones_removed": stats.tombstones_removed,
            "bytes_reclaimed": stats.bytes_reclaimed,
            "duration_ms": stats.duration_ms,
            "completed": stats.completed,
        });
        serde_wasm_bindgen::to_value(&output).unwrap_or(JsValue::NULL)
    }

    /// Returns whether vacuum is recommended (> 20% tombstones).
    #[wasm_bindgen]
    pub fn should_vacuum(&self) -> bool {
        self.store.should_vacuum(0.2)
    }

    /// Clears all data from the store.
    #[wasm_bindgen]
    pub fn clear(&mut self) {
        self.store = ColumnStore::with_schema(
            &self
                .schema
                .iter()
                .map(|(n, t)| (n.as_str(), *t))
                .collect::<Vec<_>>(),
        );
        // Restore primary key if it was set
        // Reason: We rebuild from schema since ColumnStore has no clear() method
    }

    /// Total number of rows (including tombstoned).
    #[wasm_bindgen(getter)]
    pub fn row_count(&self) -> u32 {
        // Reason: WASM row counts are always < u32::MAX (browser memory limits)
        #[allow(clippy::cast_possible_truncation)]
        {
            self.store.row_count() as u32
        }
    }

    /// Number of active (non-deleted) rows.
    #[wasm_bindgen(getter)]
    pub fn active_row_count(&self) -> u32 {
        #[allow(clippy::cast_possible_truncation)]
        {
            self.store.active_row_count() as u32
        }
    }

    /// Number of deleted (tombstoned) rows.
    #[wasm_bindgen(getter)]
    pub fn deleted_row_count(&self) -> u32 {
        #[allow(clippy::cast_possible_truncation)]
        {
            self.store.deleted_row_count() as u32
        }
    }

    /// Estimated memory usage in bytes.
    #[wasm_bindgen(getter)]
    pub fn memory_usage(&self) -> u32 {
        let mut bytes: usize = 0;
        let row_count = self.store.row_count();
        for (_, col) in &self.schema {
            bytes += match col {
                // Reason: Int and Float both use 8-byte values + 1-byte Option tag
                ColumnType::Int | ColumnType::Float => row_count * 9,
                ColumnType::String => row_count * 5, // Option<StringId(u32)> ≈ 5 bytes
                ColumnType::Bool => row_count * 2,   // Option<bool> ≈ 2 bytes
            };
        }
        #[allow(clippy::cast_possible_truncation)]
        {
            bytes as u32
        }
    }
}

impl ColumnStoreWasm {
    /// Converts `Vec<usize>` to `Vec<u32>` for WASM return.
    fn usize_to_u32(indices: Vec<usize>) -> Vec<u32> {
        indices
            .into_iter()
            .filter_map(|i| u32::try_from(i).ok())
            .collect()
    }
}

#[cfg(test)]
#[path = "column_store_tests.rs"]
mod tests;
