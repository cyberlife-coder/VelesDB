//! DISTINCT deduplication for query results (EPIC-061/US-003 refactoring).
//!
//! Extracted from mod.rs to reduce file size and improve modularity.

use std::fmt::Write as _;

use crate::point::SearchResult;
use crate::velesql::SelectColumns;
use rustc_hash::FxHashSet;

/// Apply DISTINCT deduplication to results based on selected columns (EPIC-052 US-001).
///
/// Uses HashSet for O(n) complexity and preserves insertion order.
///
/// # Dedup-key contract
///
/// The SELECT-list variants drive the dedup key as follows:
///
/// - `Columns(cols)` → dedup by the listed payload fields only.
/// - `Mixed { columns, qualified_wildcards, similarity_scores, .. }`:
///   - if `qualified_wildcards` is **empty**, dedup by the listed `columns`;
///   - if `qualified_wildcards` is **non-empty**, dedup by the **full payload**
///     (same semantics as `SELECT *`) because a qualified wildcard expands
///     to every payload field — those fields are part of the projected row
///     and must participate in the dedup key, otherwise
///     `SELECT DISTINCT ctx.*, title FROM docs` would collapse rows that
///     differ only by non-title wildcard fields.
///   - `similarity_scores` presence appends the similarity score to the key
///     so rows differing only by score are not collapsed.
///   - `aggregations` are handled by a separate aggregation pipeline and
///     are not part of the DISTINCT key (they don't reach this path).
///   - `window_functions` are deliberately excluded — DISTINCT runs BEFORE
///     window evaluation per the VelesQL pipeline order (see the contract
///     on `apply_select_postprocessing`), so any window-injected field
///     doesn't exist on the payload yet.
/// - `SimilarityScore(_)` → dedup by score only.
/// - `All` / `QualifiedWildcard(_)` → dedup by full payload.
/// - `Aggregations(_)` → no dedup (aggregations collapse rows themselves).
pub fn apply_distinct(results: Vec<SearchResult>, columns: &SelectColumns) -> Vec<SearchResult> {
    let (column_names, include_score) = match columns {
        SelectColumns::Columns(cols) => (cols.iter().map(|c| c.name.clone()).collect(), false),
        SelectColumns::Mixed {
            columns: cols,
            similarity_scores,
            qualified_wildcards,
            ..
        } => {
            // Qualified wildcards expand to every payload field; fall back
            // to "dedup by full payload" (empty column list) so those
            // fields participate in the key.
            let cols_for_dedup = if qualified_wildcards.is_empty() {
                cols.iter().map(|c| c.name.clone()).collect()
            } else {
                Vec::new()
            };
            (cols_for_dedup, !similarity_scores.is_empty())
        }
        SelectColumns::SimilarityScore(_) => (Vec::new(), true),
        // All / QualifiedWildcard → full payload; Aggregations → no dedup
        // (the empty-column path below gives the same "full payload" key,
        // which is harmless for Aggregations since that path never reaches
        // this function in practice).
        SelectColumns::All
        | SelectColumns::Aggregations(_)
        | SelectColumns::QualifiedWildcard(_) => (Vec::new(), false),
    };

    let mut seen: FxHashSet<String> = FxHashSet::default();
    results
        .into_iter()
        .filter(|r| {
            let key = compute_distinct_key(r, &column_names, include_score);
            seen.insert(key)
        })
        .collect()
}

/// Compute a unique key for DISTINCT deduplication.
///
/// Uses canonical JSON representation with sorted keys to ensure
/// logically equal objects produce identical keys.
/// When `include_score` is true, the similarity score is appended to the key
/// so rows differing only by score are not collapsed.
pub fn compute_distinct_key(
    result: &SearchResult,
    columns: &[String],
    include_score: bool,
) -> String {
    let payload = result.point.payload.as_ref();

    let mut key = if columns.is_empty() {
        // SELECT * or SELECT DISTINCT *: use full payload as key
        payload.map_or_else(|| "null".to_string(), canonical_json_string)
    } else {
        // SELECT DISTINCT col1, col2: use specific columns
        columns
            .iter()
            .map(|col| {
                payload
                    .and_then(|p| p.get(col))
                    .map_or_else(|| "null".to_string(), canonical_json_string)
            })
            .collect::<Vec<_>>()
            .join("\x1F") // ASCII Unit Separator
    };

    if include_score {
        // write! into String is infallible; avoids a temporary String allocation
        let _ = write!(key, "\x1F{}", result.score);
    }

    key
}

/// Produce a canonical JSON string with sorted object keys.
///
/// This ensures that logically equal JSON objects produce identical strings,
/// regardless of the original key order (e.g., `{"a":1,"b":2}` == `{"b":2,"a":1}`).
fn canonical_json_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            // Sort keys alphabetically for deterministic output
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort_unstable();
            let pairs: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", k, canonical_json_string(&map[*k])))
                .collect();
            format!("{{{}}}", pairs.join(","))
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(canonical_json_string).collect();
            format!("[{}]", items.join(","))
        }
        // Primitives: use standard JSON representation
        _ => value.to_string(),
    }
}

#[cfg(test)]
#[path = "distinct_unit_tests.rs"]
mod tests;
