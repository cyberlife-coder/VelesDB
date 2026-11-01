//! Index pre-filter for MATCH WHERE evaluation (S4-08).
//!
//! Extracts simple predicates from a WHERE clause and resolves them
//! against graph property indexes. When an index covers a predicate,
//! the result set is used as a pre-filter to avoid per-node brute-force
//! evaluation. Falls back gracefully when no index exists.

use super::where_eval::FLOAT_EQ_TOLERANCE;
use crate::collection::types::Collection;
use crate::velesql::{CompareOp, Condition, GraphPattern, Value};
use std::collections::{HashMap, HashSet};

/// A simple predicate extracted from a WHERE clause leaf.
///
/// Only covers predicates that map directly to a graph range index
/// lookup: equality, ordering comparisons, and BETWEEN ranges.
#[derive(Debug)]
struct ExtractedPredicate {
    /// The alias portion of the column (e.g., `"n"` from `"n.age"`).
    alias: String,
    /// The bare property name (e.g., `"age"` from `"n.age"`).
    property: String,
    /// The kind of index lookup to perform.
    kind: PredicateKind,
}

/// Describes which index lookup to perform.
#[derive(Debug)]
enum PredicateKind {
    /// Equality: `column = value`
    Exact(serde_json::Value),
    /// Strict greater than: `column > value` (Bound::Excluded)
    Gt(serde_json::Value),
    /// Greater than or equal: `column >= value` (Bound::Included)
    Gte(serde_json::Value),
    /// Strict less than: `column < value` (Bound::Excluded)
    Lt(serde_json::Value),
    /// Less than or equal: `column <= value` (Bound::Included)
    Lte(serde_json::Value),
    /// Range: `column BETWEEN low AND high` (both Bound::Included)
    Range(serde_json::Value, serde_json::Value),
}

/// Computes an index-backed pre-filter set for a MATCH WHERE clause.
///
/// Returns `Some(set)` when at least one predicate was resolved via a
/// graph property index. The set contains node IDs that satisfy all
/// indexed predicates (intersection). Returns `None` when no predicate
/// could be accelerated by an index, signalling the caller to fall back
/// to brute-force per-node evaluation.
pub(super) fn compute_index_prefilter(
    collection: &Collection,
    pattern: &GraphPattern,
    where_clause: &Condition,
    params: &HashMap<String, serde_json::Value>,
) -> Option<HashSet<u64>> {
    let predicates = extract_predicates(where_clause, params);
    if predicates.is_empty() {
        return None;
    }

    let alias_to_labels = build_alias_label_map(pattern);

    let mut result_set: Option<HashSet<u64>> = None;

    // A predicate no index covers narrows nothing here: WHERE re-checks every
    // candidate, so it is skipped rather than discarding what the indexed
    // predicates found.
    for pred in &predicates {
        let Some(label) = resolve_label(&pred.alias, &alias_to_labels) else {
            continue;
        };
        let Some(ids) = execute_single_lookup(collection, label, &pred.property, &pred.kind) else {
            continue;
        };
        let id_set: HashSet<u64> = ids.into_iter().collect();
        result_set = Some(intersect_sets(result_set, id_set));
    }

    // Only return if the set is non-empty or at least one index was consulted.
    result_set.filter(|s| !s.is_empty())
}

/// Builds a mapping from node alias to its labels from the graph pattern.
fn build_alias_label_map(pattern: &GraphPattern) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    for node in &pattern.nodes {
        if let Some(ref alias) = node.alias {
            if !node.labels.is_empty() {
                map.insert(alias.clone(), node.labels.clone());
            }
        }
    }
    map
}

/// Resolves the first label for an alias, returning `None` if unmapped.
fn resolve_label<'a>(
    alias: &str,
    alias_to_labels: &'a HashMap<String, Vec<String>>,
) -> Option<&'a str> {
    alias_to_labels
        .get(alias)
        .and_then(|labels| labels.first())
        .map(String::as_str)
}

/// Intersects an optional accumulator with a new set.
fn intersect_sets(acc: Option<HashSet<u64>>, new: HashSet<u64>) -> HashSet<u64> {
    match acc {
        Some(existing) => existing.intersection(&new).copied().collect(),
        None => new,
    }
}

/// Executes a single-property index lookup.
fn execute_single_lookup(
    collection: &Collection,
    label: &str,
    property: &str,
    kind: &PredicateKind,
) -> Option<Vec<u64>> {
    match kind {
        PredicateKind::Exact(val) => collection.graph_range_lookup_exact(label, property, val),
        PredicateKind::Gt(val) => collection.graph_range_lookup_gt(label, property, val),
        PredicateKind::Lt(val) => collection.graph_range_lookup_lt(label, property, val),
        // GTE/LTE use graph_range_lookup with inclusive bounds (Bound::Included).
        PredicateKind::Gte(val) => collection.graph_range_lookup(label, property, Some(val), None),
        PredicateKind::Lte(val) => collection.graph_range_lookup(label, property, None, Some(val)),
        PredicateKind::Range(low, high) => {
            collection.graph_range_lookup(label, property, Some(low), Some(high))
        }
    }
}

// ---------------------------------------------------------------------------
// Predicate extraction from the Condition AST
// ---------------------------------------------------------------------------

/// Walks the condition tree and extracts simple, index-eligible predicates.
///
/// Only extracts from AND-connected leaves (OR and NOT branches are skipped
/// because they cannot be safely pre-filtered with intersection semantics).
fn extract_predicates(
    condition: &Condition,
    params: &HashMap<String, serde_json::Value>,
) -> Vec<ExtractedPredicate> {
    let mut out = Vec::new();
    extract_predicates_inner(condition, params, &mut out);
    out
}

/// Recursive extraction of index-eligible predicates.
///
/// Only descends into AND branches. OR/NOT/Group branches are skipped
/// because their semantics (union, negation) cannot be pre-filtered with
/// a simple intersection of index result sets.
fn extract_predicates_inner(
    condition: &Condition,
    params: &HashMap<String, serde_json::Value>,
    out: &mut Vec<ExtractedPredicate>,
) {
    match condition {
        Condition::Comparison(cmp) => {
            if let Some(pred) = comparison_to_predicate(cmp, params) {
                out.push(pred);
            }
        }
        Condition::Between(btw) => {
            if let Some(pred) = between_to_predicate(btw, params) {
                out.push(pred);
            }
        }
        Condition::And(left, right) => {
            extract_predicates_inner(left, params, out);
            extract_predicates_inner(right, params, out);
        }
        Condition::Group(inner) => {
            extract_predicates_inner(inner, params, out);
        }
        // OR, NOT, and other condition types cannot be pre-filtered
        // with intersection semantics -- skip them.
        _ => {}
    }
}

/// Converts a `Comparison` to an `ExtractedPredicate` if the column
/// has an alias prefix and the value is a concrete literal (not a parameter
/// that failed to resolve).
fn comparison_to_predicate(
    cmp: &crate::velesql::Comparison,
    params: &HashMap<String, serde_json::Value>,
) -> Option<ExtractedPredicate> {
    let (alias, property) = split_alias_property(&cmp.column)?;
    let resolved = Collection::resolve_where_param(&cmp.value, params).ok()?;
    let kind = predicate_kind(cmp.operator, &resolved)?;

    Some(ExtractedPredicate {
        alias: alias.to_string(),
        property: property.to_string(),
        kind,
    })
}

/// Maps a WHERE comparison to the index lookup that keeps every node WHERE
/// accepts — the prefilter only rejects, so the lookup may return more nodes
/// than WHERE keeps, never fewer. Two comparisons need a wider lookup than
/// their operator suggests:
///
/// - **Float equality.** WHERE accepts any value within
///   [`FLOAT_EQ_TOLERANCE`] of a float literal, where the index matches one
///   value exactly. The lookup is the inclusive range of twice the tolerance
///   around it: a WHERE match is less than the tolerance away even after the
///   rounding of WHERE's subtraction, and rounding the range bounds cannot
///   move them past a value that close.
/// - **Strict bounds on an integer.** WHERE compares integers exactly, the
///   index orders every number as an `f64`, and above 2^53 a larger integer
///   can round onto the bound itself (`2^53 + 1` becomes `2^53`). The lookup
///   includes the bound.
///
/// `!=` cannot be answered by an index lookup and maps to `None`.
fn predicate_kind(operator: CompareOp, literal: &Value) -> Option<PredicateKind> {
    let json = literal.to_json();
    let inclusive = is_integer(literal);
    Some(match operator {
        CompareOp::Eq => equality_kind(literal, json),
        CompareOp::Gt if !inclusive => PredicateKind::Gt(json),
        CompareOp::Lt if !inclusive => PredicateKind::Lt(json),
        CompareOp::Gt | CompareOp::Gte => PredicateKind::Gte(json),
        CompareOp::Lt | CompareOp::Lte => PredicateKind::Lte(json),
        CompareOp::NotEq => return None,
    })
}

/// Whether WHERE compares this literal as an exact integer.
const fn is_integer(literal: &Value) -> bool {
    matches!(literal, Value::Integer(_) | Value::UnsignedInteger(_))
}

/// The lookup for `= literal`: exact, except for a float, which is looked up
/// as the inclusive range of twice [`FLOAT_EQ_TOLERANCE`] around it; see
/// [`predicate_kind`].
fn equality_kind(literal: &Value, json: serde_json::Value) -> PredicateKind {
    let Value::Float(center) = *literal else {
        return PredicateKind::Exact(json);
    };
    let half_width = 2.0 * FLOAT_EQ_TOLERANCE;
    PredicateKind::Range(
        serde_json::json!(center - half_width),
        serde_json::json!(center + half_width),
    )
}

/// Converts a `BetweenCondition` to a range predicate.
fn between_to_predicate(
    btw: &crate::velesql::BetweenCondition,
    params: &HashMap<String, serde_json::Value>,
) -> Option<ExtractedPredicate> {
    let (alias, property) = split_alias_property(&btw.column)?;
    let low_resolved = Collection::resolve_where_param(&btw.low, params).ok()?;
    let high_resolved = Collection::resolve_where_param(&btw.high, params).ok()?;

    Some(ExtractedPredicate {
        alias: alias.to_string(),
        property: property.to_string(),
        kind: PredicateKind::Range(low_resolved.to_json(), high_resolved.to_json()),
    })
}

/// Splits `"n.age"` into `("n", "age")`. Returns `None` for bare column names
/// (no alias prefix), since those cannot be resolved to a node label.
fn split_alias_property(column: &str) -> Option<(&str, &str)> {
    let (alias, property) = column.split_once('.')?;
    if alias.is_empty() || property.is_empty() {
        return None;
    }
    Some((alias, property))
}

/// Checks whether a node ID is in the pre-filter set.
///
/// Returns `true` if:
/// - No pre-filter exists (None = no index, allow all), or
/// - The node ID is in the pre-filter set.
#[inline]
pub(super) fn passes_prefilter(prefilter: Option<&HashSet<u64>>, node_id: u64) -> bool {
    match prefilter {
        None => true,
        Some(set) => set.contains(&node_id),
    }
}

#[cfg(test)]
#[path = "index_prefilter_tests.rs"]
mod tests;
