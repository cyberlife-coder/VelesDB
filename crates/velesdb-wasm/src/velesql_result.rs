//! VelesQL query result types for WASM (S4-13).
//!
//! Mirrors the mobile `QueryResult` surface (`velesdb-mobile::query`) so that
//! JavaScript/TypeScript callers see the same semantic shape as Swift/Kotlin
//! clients. `wasm_bindgen` does not support rich Rust enums, so [`QueryResult`]
//! and [`QueryResultRow`] are exposed as opaque handles whose fields are
//! accessed through `#[wasm_bindgen(getter)]` methods.
//!
//! Rows are serialized as JSON strings (identical to Mobile), which keeps the
//! FFI boundary simple: the caller deserializes `data_json` with `JSON.parse`.

use wasm_bindgen::prelude::*;

/// Classifies a VelesQL statement outcome at the Rust layer.
///
/// Not exposed to JavaScript directly — the wasm-bindgen surface uses
/// [`QueryResult::kind()`] which returns a stable string (`"rows"`,
/// `"mutation"`, `"deletion"`, `"ddl"`, `"train"`, `"admin"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryResultKind {
    /// Row-returning query (SELECT, SHOW, DESCRIBE).
    Rows,
    /// Data manipulation that returns affected rows (INSERT, UPSERT, UPDATE).
    Mutation,
    /// Deletion that returns affected count.
    Deletion,
    /// DDL statement (CREATE, DROP, TRUNCATE).
    Ddl,
    /// TRAIN QUANTIZER — not supported in WASM, included for API parity.
    Train,
    /// Admin command (FLUSH, ANALYZE).
    Admin,
}

impl QueryResultKind {
    /// Stable string identifier exposed across the FFI boundary.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Rows => "rows",
            Self::Mutation => "mutation",
            Self::Deletion => "deletion",
            Self::Ddl => "ddl",
            Self::Train => "train",
            Self::Admin => "admin",
        }
    }
}

/// A single row returned by [`WasmDatabase::execute_query`](crate::WasmDatabase).
///
/// The full row (id, score, and all payload fields merged at top level) is
/// serialized as a JSON object string in [`QueryResultRow::data_json`]. This
/// matches the Mobile bindings contract and lets the JavaScript caller do a
/// single `JSON.parse()` per row.
#[wasm_bindgen]
#[derive(Debug)]
pub struct QueryResultRow {
    id: u64,
    score: f32,
    data_json: String,
}

#[wasm_bindgen]
impl QueryResultRow {
    /// Point ID (0 for non-point results such as `SHOW COLLECTIONS`).
    #[wasm_bindgen(getter)]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Similarity / relevance score (0.0 when no vector search ran).
    #[wasm_bindgen(getter)]
    pub fn score(&self) -> f32 {
        self.score
    }

    /// Full row content as a JSON object string.
    ///
    /// Always contains at least `id` and `score`; all payload fields are
    /// merged at the top level (excluding shadowing of `id`/`score`).
    #[wasm_bindgen(getter, js_name = dataJson)]
    pub fn data_json(&self) -> String {
        self.data_json.clone()
    }
}

impl QueryResultRow {
    /// Builds a row from the core primitives.
    ///
    /// Never leaks its arguments on the JS side — the JSON is formed inside
    /// the constructor.
    pub(crate) fn build(
        id: u64,
        score: f32,
        payload: Option<&serde_json::Value>,
    ) -> Result<Self, String> {
        let mut map = serde_json::Map::new();
        map.insert("id".to_string(), serde_json::json!(id));
        map.insert("score".to_string(), serde_json::json!(score));
        if let Some(serde_json::Value::Object(obj)) = payload {
            for (k, v) in obj {
                if k != "id" && k != "score" {
                    map.insert(k.clone(), v.clone());
                }
            }
        }
        let data_json = serde_json::to_string(&serde_json::Value::Object(map))
            .map_err(|e| format!("Failed to serialize row to JSON: {e}"))?;
        Ok(Self {
            id,
            score,
            data_json,
        })
    }

    /// Builds a "synthetic" row for introspection results where there is no
    /// underlying point (e.g. `SHOW COLLECTIONS`).
    ///
    /// The JSON object is used verbatim as `data_json`; its `id` field (if any)
    /// is preserved. `id` is set to 0 and `score` to 0.0 because there is no
    /// vector involved.
    pub(crate) fn synthetic(data: serde_json::Value) -> Result<Self, String> {
        let data_json = serde_json::to_string(&data)
            .map_err(|e| format!("Failed to serialize synthetic row: {e}"))?;
        Ok(Self {
            id: 0,
            score: 0.0,
            data_json,
        })
    }

    /// Returns the JSON payload of this row (for native-target tests).
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn data_json_ref(&self) -> &str {
        &self.data_json
    }
}

/// Result of executing a VelesQL query via
/// [`WasmDatabase::execute_query`](crate::WasmDatabase).
///
/// Getters expose the payload to JavaScript:
/// - [`kind`](Self::kind) — statement class (`"rows"`, `"mutation"`, ...)
/// - [`row_count`](Self::row_count) — number of rows in the result
/// - [`message`](Self::message) — human-readable status message
/// - [`row`](Self::row) / [`rows_json`](Self::rows_json) — row accessors
#[wasm_bindgen]
#[derive(Debug)]
pub struct QueryResult {
    kind: QueryResultKind,
    rows: Vec<QueryResultRow>,
    message: String,
}

#[wasm_bindgen]
impl QueryResult {
    /// Statement class, as a stable string.
    ///
    /// One of: `"rows"`, `"mutation"`, `"deletion"`, `"ddl"`, `"train"`,
    /// `"admin"`. Callers can branch on this to decide how to interpret
    /// [`rows_json`](Self::rows_json).
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> String {
        self.kind.as_str().to_string()
    }

    /// Number of rows in the result (`0` for DDL / admin / empty results).
    #[wasm_bindgen(getter, js_name = rowCount)]
    pub fn row_count(&self) -> u32 {
        // u32 is enough: a single query will never return > 4 billion rows.
        u32::try_from(self.rows.len()).unwrap_or(u32::MAX)
    }

    /// Human-readable status message (for display / logging).
    #[wasm_bindgen(getter)]
    pub fn message(&self) -> String {
        self.message.clone()
    }

    /// All rows as a single JSON array string.
    ///
    /// Each element is the row's `dataJson` object. Prefer this over
    /// iterating with [`row`](Self::row) when the caller wants a single
    /// `JSON.parse` call per query.
    #[wasm_bindgen(getter, js_name = rowsJson)]
    pub fn rows_json(&self) -> String {
        let mut out = String::from("[");
        for (idx, row) in self.rows.iter().enumerate() {
            if idx > 0 {
                out.push(',');
            }
            out.push_str(&row.data_json);
        }
        out.push(']');
        out
    }

    /// Row at the given index, or `undefined` when out of range.
    ///
    /// Transfers ownership to JavaScript — do not reuse the same index.
    #[wasm_bindgen]
    pub fn row(&self, index: usize) -> Option<QueryResultRow> {
        self.rows.get(index).map(|r| QueryResultRow {
            id: r.id,
            score: r.score,
            data_json: r.data_json.clone(),
        })
    }
}

impl QueryResult {
    /// Builds the final result, attaching the human-readable status message.
    pub(crate) fn from_parts(kind: QueryResultKind, rows: Vec<QueryResultRow>) -> Self {
        let count = u32::try_from(rows.len()).unwrap_or(u32::MAX);
        Self {
            kind,
            rows,
            message: build_message(kind, count),
        }
    }

    /// Returns the kind for native tests.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn kind_enum(&self) -> QueryResultKind {
        self.kind
    }

    /// Returns the raw row slice for native tests.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn rows_ref(&self) -> &[QueryResultRow] {
        &self.rows
    }
}

/// Builds the human-readable status message for a result.
pub(crate) fn build_message(kind: QueryResultKind, row_count: u32) -> String {
    match kind {
        QueryResultKind::Rows => format!("{row_count} row(s) returned"),
        QueryResultKind::Mutation => format!("{row_count} row(s) affected"),
        QueryResultKind::Deletion => format!("{row_count} row(s) deleted"),
        QueryResultKind::Ddl => "DDL statement executed successfully".to_string(),
        QueryResultKind::Train => "Training completed successfully".to_string(),
        QueryResultKind::Admin => "Admin command executed successfully".to_string(),
    }
}

/// Classifies a parsed query into its [`QueryResultKind`].
///
/// Inspects the AST flags on [`velesdb_core::velesql::Query`] in priority
/// order (TRAIN > DDL > Admin > DML > default). MATCH is treated as row-
/// returning for parity with Mobile, though WASM rejects MATCH before
/// classification is consumed.
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
        QueryResultKind::Rows
    }
}

/// Distinguishes DELETE / row-returning / row-affecting DML variants.
fn classify_dml(query: &velesdb_core::velesql::Query) -> QueryResultKind {
    use velesdb_core::velesql::DmlStatement;
    match query.dml.as_ref() {
        Some(DmlStatement::Delete(_) | DmlStatement::DeleteEdge(_)) => QueryResultKind::Deletion,
        Some(DmlStatement::SelectEdges(_)) => QueryResultKind::Rows,
        _ => QueryResultKind::Mutation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velesdb_core::velesql::Parser;

    #[test]
    fn test_classify_select() {
        let q = Parser::parse("SELECT * FROM docs LIMIT 10").expect("test: parse");
        assert_eq!(classify_query(&q), QueryResultKind::Rows);
    }

    #[test]
    fn test_classify_insert() {
        let q = Parser::parse("INSERT INTO docs (id, t) VALUES (1, 'a')").expect("test: parse");
        assert_eq!(classify_query(&q), QueryResultKind::Mutation);
    }

    #[test]
    fn test_classify_update() {
        let q = Parser::parse("UPDATE docs SET t = 'x' WHERE id = 1").expect("test: parse");
        assert_eq!(classify_query(&q), QueryResultKind::Mutation);
    }

    #[test]
    fn test_classify_delete() {
        let q = Parser::parse("DELETE FROM docs WHERE id = 1").expect("test: parse");
        assert_eq!(classify_query(&q), QueryResultKind::Deletion);
    }

    #[test]
    fn test_classify_ddl() {
        let q = Parser::parse("CREATE COLLECTION c (dimension = 4, metric = 'cosine')")
            .expect("test: parse");
        assert_eq!(classify_query(&q), QueryResultKind::Ddl);
    }

    #[test]
    fn test_classify_admin_flush() {
        let q = Parser::parse("FLUSH FULL").expect("test: parse");
        assert_eq!(classify_query(&q), QueryResultKind::Admin);
    }

    #[test]
    fn test_classify_show() {
        let q = Parser::parse("SHOW COLLECTIONS").expect("test: parse");
        assert_eq!(classify_query(&q), QueryResultKind::Rows);
    }

    #[test]
    fn test_build_message_rows() {
        assert_eq!(build_message(QueryResultKind::Rows, 3), "3 row(s) returned");
    }

    #[test]
    fn test_build_message_mutation() {
        assert_eq!(
            build_message(QueryResultKind::Mutation, 2),
            "2 row(s) affected"
        );
    }

    #[test]
    fn test_build_message_deletion() {
        assert_eq!(
            build_message(QueryResultKind::Deletion, 1),
            "1 row(s) deleted"
        );
    }

    #[test]
    fn test_build_message_ddl() {
        assert_eq!(
            build_message(QueryResultKind::Ddl, 0),
            "DDL statement executed successfully"
        );
    }

    #[test]
    fn test_kind_as_str_is_stable() {
        assert_eq!(QueryResultKind::Rows.as_str(), "rows");
        assert_eq!(QueryResultKind::Mutation.as_str(), "mutation");
        assert_eq!(QueryResultKind::Deletion.as_str(), "deletion");
        assert_eq!(QueryResultKind::Ddl.as_str(), "ddl");
        assert_eq!(QueryResultKind::Train.as_str(), "train");
        assert_eq!(QueryResultKind::Admin.as_str(), "admin");
    }

    #[test]
    fn test_row_build_without_payload() {
        let row = QueryResultRow::build(42, 0.95, None).expect("test: build");
        assert_eq!(row.id, 42);
        assert!((row.score - 0.95).abs() < f32::EPSILON);
        assert!(row.data_json.contains("\"id\":42"));
        assert!(row.data_json.contains("\"score\":"));
    }

    #[test]
    fn test_row_build_with_payload_merges_top_level() {
        let payload = serde_json::json!({"title": "hello", "tag": "t"});
        let row = QueryResultRow::build(7, 0.5, Some(&payload)).expect("test: build");
        assert!(row.data_json.contains("\"title\":\"hello\""));
        assert!(row.data_json.contains("\"tag\":\"t\""));
        assert!(row.data_json.contains("\"id\":7"));
    }

    #[test]
    fn test_row_build_with_payload_does_not_shadow_id_or_score() {
        // Payload keys that would conflict with id/score must be filtered out.
        let payload = serde_json::json!({"id": 999, "score": -1.0, "ok": true});
        let row = QueryResultRow::build(42, 0.3, Some(&payload)).expect("test: build");
        assert!(row.data_json.contains("\"id\":42"));
        assert!(!row.data_json.contains("\"id\":999"));
        assert!(row.data_json.contains("\"ok\":true"));
    }

    #[test]
    fn test_from_parts_assembles_message_and_count() {
        let rows = vec![
            QueryResultRow::build(1, 0.0, None).expect("test: row 1"),
            QueryResultRow::build(2, 0.0, None).expect("test: row 2"),
        ];
        let result = QueryResult::from_parts(QueryResultKind::Rows, rows);
        assert_eq!(result.message, "2 row(s) returned");
        assert_eq!(result.rows.len(), 2);
    }

    #[test]
    fn test_synthetic_row_preserves_data() {
        let row = QueryResultRow::synthetic(serde_json::json!({"name": "docs", "dim": 4}))
            .expect("test: synthetic");
        assert_eq!(row.id, 0);
        assert_eq!(row.score, 0.0);
        assert!(row.data_json.contains("\"name\":\"docs\""));
        assert!(row.data_json.contains("\"dim\":4"));
    }
}
