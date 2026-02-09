//! Builds a ColumnStore from a Collection's stored points.
//!
//! This module provides the bridge between Collection (point-oriented storage)
//! and ColumnStore (column-oriented storage) for JOIN operations.
//! The point ID is used as the primary key for O(1) lookups during JOINs.

use crate::collection::Collection;
use crate::column_store::{ColumnStore, ColumnType, ColumnValue};
use crate::error::Result;

/// Maximum number of rows to materialize from a collection into a ColumnStore.
/// Prevents OOM on very large collections during JOIN operations.
const DEFAULT_MAX_ROWS: usize = 100_000;

/// Builds a ColumnStore from a Collection's stored point payloads.
///
/// Scans points in the collection and extracts payload fields into typed columns.
/// The point ID becomes the integer primary key (`"id"` column) for O(1) JOIN lookups.
///
/// # Type Inference
///
/// Column types are inferred from the first non-null value encountered per field:
/// - JSON Number (integer) → `ColumnType::Int`
/// - JSON Number (float) → `ColumnType::Float`
/// - JSON String → `ColumnType::String`
/// - JSON Bool → `ColumnType::Bool`
/// - JSON Array/Object → skipped (not supported in ColumnStore)
///
/// # Arguments
///
/// * `collection` - Source collection to extract payloads from
/// * `max_rows` - Maximum number of points to materialize (0 = use default)
///
/// # Errors
///
/// Returns an error if the ColumnStore cannot be created (e.g., schema conflict).
pub fn column_store_from_collection(
    collection: &Collection,
    max_rows: usize,
) -> Result<ColumnStore> {
    let effective_max = if max_rows == 0 {
        DEFAULT_MAX_ROWS
    } else {
        max_rows
    };

    let all_ids = collection.all_ids();
    let ids_to_fetch: Vec<u64> = all_ids.into_iter().take(effective_max).collect();

    if ids_to_fetch.is_empty() {
        return Ok(ColumnStore::new());
    }

    // Retrieve points with payloads
    let points = collection.get(&ids_to_fetch);

    // Phase 1: Infer schema from all payloads (scan first non-null value per field)
    let mut schema: Vec<(String, ColumnType)> = Vec::new();
    let mut seen_fields: std::collections::HashSet<String> = std::collections::HashSet::new();

    for maybe_point in &points {
        let Some(point) = maybe_point else { continue };
        let Some(ref payload) = point.payload else {
            continue;
        };
        let Some(obj) = payload.as_object() else {
            continue;
        };

        for (key, value) in obj {
            if seen_fields.contains(key.as_str()) {
                continue;
            }
            if let Some(col_type) = infer_column_type(value) {
                schema.push((key.clone(), col_type));
                seen_fields.insert(key.clone());
            }
        }
    }

    // Add "id" column as primary key (always Int)
    if !seen_fields.contains("id") {
        schema.insert(0, ("id".to_string(), ColumnType::Int));
    }

    // Phase 2: Build ColumnStore with schema + PK
    let schema_refs: Vec<(&str, ColumnType)> =
        schema.iter().map(|(n, t)| (n.as_str(), *t)).collect();
    let mut store = ColumnStore::with_primary_key(&schema_refs, "id")?;

    // Phase 3: Insert rows
    for maybe_point in &points {
        let Some(point) = maybe_point else { continue };

        let mut row_values: Vec<(&str, ColumnValue)> = Vec::new();

        // Reason: safe cast — point IDs within i64 range for PK index.
        // IDs > i64::MAX are extremely rare in practice.
        let id_value = i64::try_from(point.id).unwrap_or(i64::MAX);
        row_values.push(("id", ColumnValue::Int(id_value)));

        if let Some(ref payload) = point.payload {
            if let Some(obj) = payload.as_object() {
                for (key, col_type) in &schema {
                    if key == "id" {
                        continue; // Already added above
                    }
                    let value: ColumnValue = obj.get(key.as_str()).map_or(ColumnValue::Null, |v| {
                        json_to_column_value(v, *col_type, &mut store)
                    });
                    row_values.push((key.as_str(), value));
                }
            }
        }

        // Ignore duplicate key errors (shouldn't happen with unique point IDs)
        let _ = store.insert_row(&row_values);
    }

    Ok(store)
}

/// Infers a `ColumnType` from a JSON value.
///
/// Returns `None` for arrays and objects (not supported in ColumnStore).
fn infer_column_type(value: &serde_json::Value) -> Option<ColumnType> {
    match value {
        serde_json::Value::Bool(_) => Some(ColumnType::Bool),
        serde_json::Value::Number(n) => {
            if n.is_i64() {
                Some(ColumnType::Int)
            } else {
                Some(ColumnType::Float)
            }
        }
        serde_json::Value::String(_) => Some(ColumnType::String),
        serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            None
        }
    }
}

/// Converts a JSON value to a `ColumnValue` for the given column type.
///
/// Uses the ColumnStore's string table for string interning.
fn json_to_column_value(
    value: &serde_json::Value,
    col_type: ColumnType,
    store: &mut ColumnStore,
) -> ColumnValue {
    match (value, col_type) {
        (serde_json::Value::Number(n), ColumnType::Int) => {
            n.as_i64().map_or(ColumnValue::Null, ColumnValue::Int)
        }
        (serde_json::Value::Number(n), ColumnType::Float) => {
            n.as_f64().map_or(ColumnValue::Null, ColumnValue::Float)
        }
        (serde_json::Value::String(s), ColumnType::String) => {
            let id = store.string_table_mut().intern(s);
            ColumnValue::String(id)
        }
        (serde_json::Value::Bool(b), ColumnType::Bool) => ColumnValue::Bool(*b),
        _ => ColumnValue::Null, // Null or type mismatch → null
    }
}
