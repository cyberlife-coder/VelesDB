//! VelesQL query execution via UniFFI for mobile targets.
//!
//! Exposes `execute_query()` on [`VelesDatabase`] so iOS/Android apps can
//! run arbitrary VelesQL statements (SELECT, INSERT, UPDATE, DELETE, MATCH,
//! DDL, TRAIN, SHOW, FLUSH, etc.) through a single entry point.
//!
//! Results are returned as [`QueryResult`], a UniFFI-friendly struct that
//! encodes rows as JSON strings (because UniFFI cannot represent
//! `HashMap<String, serde_json::Value>` directly).

use std::collections::HashMap;

use crate::types::VelesError;

// ============================================================================
// UniFFI-exported types
// ============================================================================

/// Classifies the kind of VelesQL statement that was executed.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum QueryResultKind {
    /// Row-returning query (SELECT, MATCH, SHOW, DESCRIBE).
    Rows,
    /// Data manipulation that returns affected rows (INSERT, UPSERT, UPDATE).
    Mutation,
    /// Deletion that returns affected count.
    Deletion,
    /// DDL statement (CREATE, DROP, ALTER, TRUNCATE).
    Ddl,
    /// TRAIN QUANTIZER.
    Train,
    /// Admin command (FLUSH, ANALYZE).
    Admin,
}

/// A single row in a query result, serialized as JSON for FFI safety.
///
/// UniFFI cannot represent `HashMap<String, serde_json::Value>` directly,
/// so each row is a JSON object string that the mobile client deserializes
/// with its native JSON parser (Swift `JSONSerialization`, Kotlin `Gson`).
#[derive(Debug, Clone, uniffi::Record)]
pub struct QueryResultRow {
    /// Point ID (0 for non-point results like SHOW COLLECTIONS).
    pub id: u64,
    /// Similarity / relevance score (0.0 for non-search results).
    pub score: f32,
    /// Full row data as a JSON object string.
    /// Contains `id`, `score`, and all payload fields merged at top level.
    pub data_json: String,
}

/// Result of executing a VelesQL query via [`crate::VelesDatabase::execute_query`].
#[derive(Debug, Clone, uniffi::Record)]
pub struct QueryResult {
    /// What kind of statement produced this result.
    pub kind: QueryResultKind,
    /// Result rows (empty for DDL/TRAIN/FLUSH that return no data).
    pub rows: Vec<QueryResultRow>,
    /// Number of rows in the result (convenience field for mobile).
    pub row_count: u32,
    /// Human-readable status message (e.g., "3 rows inserted").
    pub message: String,
}

// ============================================================================
// Conversion helpers
// ============================================================================

/// Classifies a parsed query into its [`QueryResultKind`].
pub(crate) fn classify_query(query: &velesdb_core::velesql::Query) -> QueryResultKind {
    if query.is_train() {
        QueryResultKind::Train
    } else if query.is_ddl_query() {
        QueryResultKind::Ddl
    } else if query.is_admin_query() {
        QueryResultKind::Admin
    } else if query.is_dml_query() {
        classify_dml(query)
    } else {
        // SELECT, MATCH, introspection (SHOW/DESCRIBE/EXPLAIN)
        QueryResultKind::Rows
    }
}

/// Distinguishes DELETE from other DML (INSERT/UPSERT/UPDATE).
fn classify_dml(query: &velesdb_core::velesql::Query) -> QueryResultKind {
    use velesdb_core::velesql::DmlStatement;
    match query.dml.as_ref() {
        Some(DmlStatement::Delete(_) | DmlStatement::DeleteEdge(_)) => QueryResultKind::Deletion,
        _ => QueryResultKind::Mutation,
    }
}

/// Converts a core `SearchResult` into a [`QueryResultRow`].
///
/// Flattens the point payload into the top-level JSON object alongside
/// `id` and `score` fields, matching the CLI REPL output format.
pub(crate) fn to_result_row(
    result: &velesdb_core::SearchResult,
) -> Result<QueryResultRow, VelesError> {
    let mut map = serde_json::Map::new();
    map.insert("id".to_string(), serde_json::json!(result.point.id));
    map.insert("score".to_string(), serde_json::json!(result.score));

    if let Some(serde_json::Value::Object(payload)) = &result.point.payload {
        for (k, v) in payload {
            if k != "id" && k != "score" {
                map.insert(k.clone(), v.clone());
            }
        }
    }

    let data_json = serde_json::to_string(&serde_json::Value::Object(map))
        .map_err(|e| VelesError::database(format!("Failed to serialize row to JSON: {e}")))?;

    Ok(QueryResultRow {
        id: result.point.id,
        score: result.score,
        data_json,
    })
}

/// Builds the human-readable message for the query result.
pub(crate) fn build_message(kind: &QueryResultKind, row_count: u32) -> String {
    match kind {
        QueryResultKind::Rows => format!("{row_count} row(s) returned"),
        QueryResultKind::Mutation => format!("{row_count} row(s) affected"),
        QueryResultKind::Deletion => format!("{row_count} row(s) deleted"),
        QueryResultKind::Ddl => "DDL statement executed successfully".to_string(),
        QueryResultKind::Train => "Training completed successfully".to_string(),
        QueryResultKind::Admin => "Admin command executed successfully".to_string(),
    }
}

/// Parses a JSON string into query parameters.
///
/// VelesQL parameters use `$name` syntax. The params map keys should
/// be the bare name (without the `$` prefix).
pub(crate) fn parse_params(
    params_json: Option<String>,
) -> Result<HashMap<String, serde_json::Value>, VelesError> {
    params_json
        .map(|json| {
            serde_json::from_str(&json)
                .map_err(|e| VelesError::database(format!("Invalid params JSON: {e}")))
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod integration_tests;

#[cfg(test)]
#[path = "query_unit_tests.rs"]
mod tests;
