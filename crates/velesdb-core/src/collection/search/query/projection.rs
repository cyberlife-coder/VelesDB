//! SQL projection engine for VelesQL SELECT expressions.
//!
//! Applies `SelectColumns` to `SearchResult` rows, producing JSON objects
//! with only the requested fields. Used by the query pipeline after
//! post-processing (DISTINCT, ORDER BY, LIMIT).

use crate::point::SearchResult;
use crate::velesql::{SelectColumns, SimilarityScoreExpr};

/// Projects a list of `SearchResult` according to the parsed SELECT expressions.
///
/// Returns `serde_json::Value::Object` rows with only the requested fields.
/// The `id` field is always the system point ID (takes precedence over payload).
#[must_use]
pub fn project_results(
    results: &[SearchResult],
    select_exprs: &SelectColumns,
) -> Vec<serde_json::Value> {
    results
        .iter()
        .map(|r| project_single(r, select_exprs))
        .collect()
}

/// Projects a single `SearchResult` into a JSON row.
fn project_single(result: &SearchResult, select_exprs: &SelectColumns) -> serde_json::Value {
    match select_exprs {
        SelectColumns::All | SelectColumns::QualifiedWildcard(_) => project_wildcard(result),
        SelectColumns::Columns(cols) => project_columns(result, cols),
        SelectColumns::SimilarityScore(expr) => project_similarity_only(result, expr),
        SelectColumns::Aggregations(_) => {
            // Aggregations are handled by a separate code path; return empty row.
            serde_json::Value::Object(serde_json::Map::new())
        }
        SelectColumns::Mixed {
            columns,
            aggregations: _,
            similarity_scores,
            qualified_wildcards,
            window_functions,
        } => project_mixed(
            result,
            columns,
            similarity_scores,
            qualified_wildcards,
            window_functions,
        ),
    }
}

/// `SELECT *` or `SELECT alias.*`: returns `{id, ...payload_fields}`.
///
/// Excludes vectors and similarity score. Use `SELECT similarity() AS score, *`
/// to include the score explicitly.
fn project_wildcard(result: &SearchResult) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("id".to_string(), serde_json::Value::from(result.point.id));

    if let Some(serde_json::Value::Object(payload_map)) = result.point.payload.as_ref() {
        for (k, v) in payload_map {
            if k != "id" {
                map.insert(k.clone(), v.clone());
            }
        }
    }

    serde_json::Value::Object(map)
}

/// `SELECT col1, col2 [AS alias]`: extracts only named fields.
fn project_columns(result: &SearchResult, columns: &[crate::velesql::Column]) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    for col in columns {
        let output_key = col.alias.as_deref().unwrap_or(&col.name);
        let value = extract_field_value(result, &col.name);
        map.insert(output_key.to_string(), value);
    }

    serde_json::Value::Object(map)
}

/// `SELECT similarity() [AS alias]`: materializes the score only.
fn project_similarity_only(result: &SearchResult, expr: &SimilarityScoreExpr) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    let key = expr.alias.as_deref().unwrap_or("similarity");
    map.insert(
        key.to_string(),
        serde_json::Value::from(f64::from(result.score)),
    );
    serde_json::Value::Object(map)
}

/// Mixed projection: columns + similarity scores + qualified wildcards + window functions.
///
/// Window function values were injected into the row's payload by
/// [`crate::velesql::window_evaluator`]. The wildcard-expansion step below
/// therefore must skip keys that correspond to window-function aliases —
/// otherwise those values would be read from the payload twice (once by
/// wildcard expansion, once by the explicit window-function loop). The final
/// value would still be correct (the explicit loop wins), but the extra
/// copy is pointless and mis-signals in reviews as suspicious dedup.
fn project_mixed(
    result: &SearchResult,
    columns: &[crate::velesql::Column],
    similarity_scores: &[SimilarityScoreExpr],
    qualified_wildcards: &[String],
    window_functions: &[crate::velesql::WindowFunction],
) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    // Pre-compute the set of window-function aliases so wildcard expansion
    // can skip them in O(1) per payload key.
    let window_aliases: rustc_hash::FxHashSet<&str> = window_functions
        .iter()
        .map(|wf| {
            wf.alias
                .as_deref()
                .unwrap_or(wf.function_type.default_alias())
        })
        .collect();

    if !qualified_wildcards.is_empty() {
        insert_qualified_wildcards(&mut map, result, &window_aliases);
    }
    insert_named_columns(&mut map, result, columns);
    insert_similarity_scores(&mut map, result, similarity_scores);
    insert_window_values(&mut map, result, window_functions);

    serde_json::Value::Object(map)
}

/// Expand a qualified wildcard (`c.*`) into id + every payload field, skipping
/// any key shadowed by a window-function alias.
fn insert_qualified_wildcards(
    map: &mut serde_json::Map<String, serde_json::Value>,
    result: &SearchResult,
    window_aliases: &rustc_hash::FxHashSet<&str>,
) {
    map.insert("id".to_string(), serde_json::Value::from(result.point.id));
    if let Some(serde_json::Value::Object(payload_map)) = result.point.payload.as_ref() {
        for (k, v) in payload_map {
            if k != "id" && !window_aliases.contains(k.as_str()) {
                map.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Insert each explicitly named column (honouring its alias).
fn insert_named_columns(
    map: &mut serde_json::Map<String, serde_json::Value>,
    result: &SearchResult,
    columns: &[crate::velesql::Column],
) {
    for col in columns {
        let output_key = col.alias.as_deref().unwrap_or(&col.name);
        let value = extract_field_value(result, &col.name);
        map.insert(output_key.to_string(), value);
    }
}

/// Insert similarity-score expressions (all resolve to the result score).
fn insert_similarity_scores(
    map: &mut serde_json::Map<String, serde_json::Value>,
    result: &SearchResult,
    similarity_scores: &[SimilarityScoreExpr],
) {
    for expr in similarity_scores {
        let key = expr.alias.as_deref().unwrap_or("similarity");
        map.insert(
            key.to_string(),
            serde_json::Value::from(f64::from(result.score)),
        );
    }
}

/// Insert window-function values (injected into the payload by the window
/// evaluator). This is the single source of truth for window aliases —
/// wildcard expansion deliberately skips them.
fn insert_window_values(
    map: &mut serde_json::Map<String, serde_json::Value>,
    result: &SearchResult,
    window_functions: &[crate::velesql::WindowFunction],
) {
    for wf in window_functions {
        let alias = wf
            .alias
            .as_deref()
            .unwrap_or(wf.function_type.default_alias());
        let value = result
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get(alias))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        map.insert(alias.to_string(), value);
    }
}

/// Extracts a field value from a `SearchResult`, supporting nested paths.
///
/// - `"title"` → `payload["title"]`
/// - `"meta.source"` → `payload["meta"]["source"]`
/// - `"id"` → system point ID (takes precedence over payload)
fn extract_field_value(result: &SearchResult, field_path: &str) -> serde_json::Value {
    if field_path == "id" {
        return serde_json::Value::from(result.point.id);
    }

    let Some(payload) = result.point.payload.as_ref() else {
        return serde_json::Value::Null;
    };

    if field_path.contains('.') {
        // Nested path traversal
        let mut current = payload;
        for segment in field_path.split('.') {
            match current.get(segment) {
                Some(next) => current = next,
                None => return serde_json::Value::Null,
            }
        }
        current.clone()
    } else {
        payload
            .get(field_path)
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
