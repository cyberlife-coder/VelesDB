//! Column projection + window-function bridge for the WASM SELECT pipeline (#3b).
//!
//! Mirrors velesdb-core's projection engine so `SELECT col`, `SELECT col AS
//! alias`, `SELECT *`, and window functions behave identically to the REST
//! surface — instead of always emitting `id + score + full payload`:
//!
//! - [`to_search_results`] adapts the WASM `OwnedScanRow` triples into core
//!   [`SearchResult`] values (the type the window evaluator and the projection
//!   logic consume).
//! - [`inject_window_functions`] runs core's
//!   [`velesdb_core::velesql::window_evaluator::evaluate`], which injects each
//!   window alias (`ROW_NUMBER`/`RANK`/`DENSE_RANK`) into the row payload
//!   **before** ORDER BY/projection, matching the core pipeline position.
//! - [`project`] reproduces `projection::project_single`'s column extraction.
//!   That function lives in `velesdb_core::collection`, which is gated behind
//!   the `persistence` feature WASM never enables, so it cannot be called
//!   directly; this module mirrors its semantics field-for-field to keep the
//!   output identical (`id`-precedence, dotted-path lookup, alias handling,
//!   similarity-score materialization, window-alias dedup).

use velesdb_core::point::{Point, SearchResult};
use velesdb_core::velesql::{
    Column, SelectColumns, SelectStatement, SimilarityScoreExpr, WindowFunction,
};

use crate::velesql_result::QueryResultRow;
use crate::velesql_scan::OwnedScanRow;

type JsonMap = serde_json::Map<String, serde_json::Value>;

/// Adapts scanned `(id, score, payload)` triples into core [`SearchResult`]s.
///
/// The vector is left empty: the WASM finalize path has already scored every
/// row, and neither projection nor window evaluation reads `point.vector`.
pub(crate) fn to_search_results(rows: Vec<OwnedScanRow>) -> Vec<SearchResult> {
    rows.into_iter()
        .map(|(id, score, payload)| SearchResult::new(Point::new(id, Vec::new(), payload), score))
        .collect()
}

/// Runs core window evaluation, injecting window-function aliases into each
/// row payload before ORDER BY / projection. No-op when the SELECT has none.
pub(crate) fn inject_window_functions(
    stmt: &SelectStatement,
    results: &mut [SearchResult],
) -> Result<(), String> {
    let Some(window_functions) = extract_window_functions(&stmt.columns) else {
        return Ok(());
    };
    velesdb_core::velesql::window_evaluator::evaluate(results, window_functions)
        .map_err(|e| format!("Window function evaluation failed: {e}"))
}

/// Returns the window functions in a `SELECT` list, if any (mirrors core's
/// `Collection::extract_window_functions`).
fn extract_window_functions(columns: &SelectColumns) -> Option<&[WindowFunction]> {
    match columns {
        SelectColumns::Mixed {
            window_functions, ..
        } if !window_functions.is_empty() => Some(window_functions),
        _ => None,
    }
}

/// Projects each result by the SELECT column list, preserving the real `id` /
/// `score` for the JS getters.
pub(crate) fn project(
    stmt: &SelectStatement,
    results: &[SearchResult],
) -> Result<Vec<QueryResultRow>, String> {
    results
        .iter()
        .map(|result| {
            let row = project_single(result, &stmt.columns);
            QueryResultRow::from_projected(result.point.id, result.score, &row)
        })
        .collect()
}

/// Projects one result into a JSON row — mirror of `projection::project_single`.
fn project_single(result: &SearchResult, columns: &SelectColumns) -> serde_json::Value {
    match columns {
        SelectColumns::All | SelectColumns::QualifiedWildcard(_) => project_wildcard(result),
        SelectColumns::Columns(cols) => project_columns(result, cols),
        SelectColumns::SimilarityScore(expr) => project_similarity_only(result, expr),
        // Aggregations are handled by the aggregation pipeline, not here.
        SelectColumns::Aggregations(_) => serde_json::Value::Object(JsonMap::new()),
        SelectColumns::Mixed {
            columns,
            similarity_scores,
            qualified_wildcards,
            window_functions,
            ..
        } => project_mixed(
            result,
            columns,
            similarity_scores,
            qualified_wildcards,
            window_functions,
        ),
        // `SelectColumns` is non_exhaustive; an unknown future variant falls
        // back to the wildcard shape (id + payload) rather than dropping rows.
        _ => project_wildcard(result),
    }
}

/// `SELECT *` / `alias.*`: `{id, ...payload}` (excludes the score).
fn project_wildcard(result: &SearchResult) -> serde_json::Value {
    let mut map = JsonMap::new();
    insert_wildcard_fields(&mut map, result, &[]);
    serde_json::Value::Object(map)
}

/// `SELECT col1, col2 [AS alias]`: only the named fields.
fn project_columns(result: &SearchResult, columns: &[Column]) -> serde_json::Value {
    let mut map = JsonMap::new();
    insert_named_columns(&mut map, result, columns);
    serde_json::Value::Object(map)
}

/// `SELECT similarity() [AS alias]`: the score only.
fn project_similarity_only(result: &SearchResult, expr: &SimilarityScoreExpr) -> serde_json::Value {
    let mut map = JsonMap::new();
    let key = expr.alias.as_deref().unwrap_or("similarity");
    map.insert(key.to_string(), score_value(result));
    serde_json::Value::Object(map)
}

/// Mixed projection: qualified wildcards + columns + similarity scores +
/// window functions (window aliases were injected into the payload upstream).
fn project_mixed(
    result: &SearchResult,
    columns: &[Column],
    similarity_scores: &[SimilarityScoreExpr],
    qualified_wildcards: &[String],
    window_functions: &[WindowFunction],
) -> serde_json::Value {
    let mut map = JsonMap::new();
    let window_aliases: Vec<&str> = window_functions.iter().map(window_alias).collect();
    if !qualified_wildcards.is_empty() {
        insert_wildcard_fields(&mut map, result, &window_aliases);
    }
    insert_named_columns(&mut map, result, columns);
    for expr in similarity_scores {
        let key = expr.alias.as_deref().unwrap_or("similarity");
        map.insert(key.to_string(), score_value(result));
    }
    for wf in window_functions {
        let alias = window_alias(wf);
        let value = payload_field(result, alias).unwrap_or(serde_json::Value::Null);
        map.insert(alias.to_string(), value);
    }
    serde_json::Value::Object(map)
}

/// Inserts `id` + every payload field, skipping any key shadowed by a
/// window-function alias.
fn insert_wildcard_fields(map: &mut JsonMap, result: &SearchResult, skip: &[&str]) {
    map.insert("id".to_string(), serde_json::Value::from(result.point.id));
    if let Some(serde_json::Value::Object(payload)) = result.point.payload.as_ref() {
        for (k, v) in payload {
            if k != "id" && !skip.contains(&k.as_str()) {
                map.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Inserts each named column under its alias (or its own name).
fn insert_named_columns(map: &mut JsonMap, result: &SearchResult, columns: &[Column]) {
    for col in columns {
        let key = col.alias.as_deref().unwrap_or(&col.name);
        map.insert(key.to_string(), extract_field_value(result, &col.name));
    }
}

/// Extracts a field value (supporting dotted paths); `id` takes precedence
/// over any payload `id`.
fn extract_field_value(result: &SearchResult, field_path: &str) -> serde_json::Value {
    if field_path == "id" {
        return serde_json::Value::from(result.point.id);
    }
    payload_field(result, field_path).unwrap_or(serde_json::Value::Null)
}

/// Looks up a (possibly dotted) payload field.
fn payload_field(result: &SearchResult, path: &str) -> Option<serde_json::Value> {
    let payload = result.point.payload.as_ref()?;
    crate::filter::get_nested_field(payload, path).cloned()
}

/// The materialized search score as a JSON number.
fn score_value(result: &SearchResult) -> serde_json::Value {
    serde_json::Value::from(f64::from(result.score))
}

/// The output alias of a window function.
fn window_alias(wf: &WindowFunction) -> &str {
    wf.alias
        .as_deref()
        .unwrap_or_else(|| wf.function_type.default_alias())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<OwnedScanRow> {
        vec![
            (1, 0.9, Some(serde_json::json!({"cat": "a", "title": "X"}))),
            (2, 0.5, Some(serde_json::json!({"cat": "b", "title": "Y"}))),
        ]
    }

    #[test]
    fn test_to_search_results_preserves_id_score_payload() {
        let results = to_search_results(rows());
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].point.id, 1);
        assert!((results[0].score - 0.9).abs() < f32::EPSILON);
        assert_eq!(results[1].point.payload.as_ref().unwrap()["cat"], "b");
    }

    #[test]
    fn test_project_specific_columns_only() {
        let mut stmt = SelectStatement::empty();
        stmt.columns = SelectColumns::Columns(vec![velesdb_core::velesql::Column::new("cat")]);
        let results = to_search_results(rows());
        let out = project(&stmt, &results).expect("test: project");
        let body: serde_json::Value =
            serde_json::from_str(out[0].data_json_ref()).expect("test: json");
        let obj = body.as_object().expect("test: obj");
        assert_eq!(obj.keys().collect::<Vec<_>>(), vec!["cat"]);
        // Real id/score are preserved on the row handle even though the body
        // only carries the projected column.
        assert_eq!(out[0].id(), 1);
    }

    #[test]
    fn test_project_alias_renames() {
        let mut stmt = SelectStatement::empty();
        stmt.columns = SelectColumns::Columns(vec![velesdb_core::velesql::Column::with_alias(
            "title", "name",
        )]);
        let results = to_search_results(rows());
        let out = project(&stmt, &results).expect("test: project");
        let body: serde_json::Value =
            serde_json::from_str(out[0].data_json_ref()).expect("test: json");
        assert!(body.get("name").is_some());
        assert!(body.get("title").is_none());
    }

    #[test]
    fn test_inject_window_functions_noop_without_window() {
        let stmt = SelectStatement::empty();
        let mut results = to_search_results(rows());
        inject_window_functions(&stmt, &mut results).expect("test: noop");
        // Payload untouched.
        assert_eq!(results[0].point.payload.as_ref().unwrap()["cat"], "a");
    }
}
