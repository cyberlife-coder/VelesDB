//! DISTINCT deduplication for query results (EPIC-061/US-003 refactoring).
//!
//! Lives in `velesql`, beside [`super::window_evaluator`], rather than in the
//! `persistence`-gated collection query engine, so the WASM executor dedups
//! through the same key: one DISTINCT contract for both engines (conformance
//! `projection_cases` in `conformance/velesql_executor_cases.json`).

use std::fmt::Write as _;

use super::group_key::normalise;
use super::json_path::ProjectedField;
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
#[must_use]
pub fn apply_distinct(results: Vec<SearchResult>, columns: &SelectColumns) -> Vec<SearchResult> {
    let (columns, include_score) = key_columns(columns);
    let mut seen: FxHashSet<String> = FxHashSet::default();
    results
        .into_iter()
        .filter(|r| {
            let fields = columns.iter().map(|col| ProjectedField::Source(&col.name));
            let key = projected_key(r, fields, include_score);
            seen.insert(key)
        })
        .collect()
}

/// The fields a projected row is keyed by under `columns` (see the
/// [`apply_distinct`] contract): the listed columns, or none for "the full
/// payload", and whether the similarity score is part of the key.
pub(super) fn key_columns(columns: &SelectColumns) -> (&[super::Column], bool) {
    match columns {
        SelectColumns::Columns(cols) => (cols, false),
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
                cols.as_slice()
            } else {
                &[]
            };
            (cols_for_dedup, !similarity_scores.is_empty())
        }
        SelectColumns::SimilarityScore(_) => (&[], true),
        // All / QualifiedWildcard → full payload; Aggregations → no dedup
        // (the empty-column path gives the same "full payload" key, which is
        // harmless for Aggregations since that path never reaches DISTINCT).
        SelectColumns::All
        | SelectColumns::Aggregations(_)
        | SelectColumns::QualifiedWildcard(_) => (&[], false),
    }
}

/// Compute a unique key for DISTINCT deduplication.
///
/// Uses canonical JSON representation with sorted keys to ensure
/// logically equal objects produce identical keys.
/// When `include_score` is true, the similarity score is appended to the key
/// so rows differing only by score are not collapsed.
pub(crate) fn compute_distinct_key(
    result: &SearchResult,
    columns: &[String],
    include_score: bool,
) -> String {
    projected_key(
        result,
        columns.iter().map(|name| ProjectedField::Source(name)),
        include_score,
    )
}

/// Set operators key the published projection, while SELECT DISTINCT keys
/// its input columns before window evaluation. Both share one value encoding.
pub(super) fn projected_key<'a>(
    result: &SearchResult,
    fields: impl Iterator<Item = ProjectedField<'a>>,
    include_score: bool,
) -> String {
    projected_values_key(
        result,
        &mut fields.map(|field| field.read(result)),
        include_score,
    )
}

/// Both SELECT DISTINCT and set operators encode their own projected values
/// with the same separator and numeric normalisation.
pub(super) fn projected_values_key(
    result: &SearchResult,
    values: &mut dyn Iterator<Item = std::borrow::Cow<'_, serde_json::Value>>,
    include_score: bool,
) -> String {
    let mut key = if let Some(first) = values.next() {
        let mut key = canonical_value(normalise(first));
        for value in values {
            key.push('\x1f');
            key.push_str(&canonical_value(normalise(value)));
        }
        key
    } else {
        result
            .point
            .payload
            .as_ref()
            .map_or_else(|| "null".to_owned(), canonical_payload)
    };

    if include_score {
        // write! into String is infallible; avoids a temporary String allocation
        let _ = write!(key, "\x1F{}", result.score);
    }

    key
}

/// JSON object order is part of the wire key even when serde_json is built
/// with its insertion-order feature. The normalised projection is already owned.
fn canonical_value(mut value: serde_json::Value) -> String {
    value.sort_all_objects();
    value.to_string()
}

/// Full-payload keys can serialize an already ordered document by reference.
pub(super) fn canonical_payload(value: &serde_json::Value) -> String {
    if ordered_objects(value) {
        value.to_string()
    } else {
        canonical_value(value.clone())
    }
}

fn ordered_objects(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            map.keys().is_sorted() && map.values().all(ordered_objects)
        }
        serde_json::Value::Array(list) => list.iter().all(ordered_objects),
        _ => true,
    }
}

#[cfg(test)]
#[path = "distinct_evaluator_unit_tests.rs"]
mod tests;
