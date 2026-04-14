//! Index pre-filter for MATCH WHERE evaluation (S4-08, S4-09).
//!
//! Extracts simple predicates from a WHERE clause and resolves them
//! against graph property indexes. When an index covers a predicate,
//! the result set is used as a pre-filter to avoid per-node brute-force
//! evaluation. Falls back gracefully when no index exists.

use crate::collection::types::Collection;
use crate::velesql::{CompareOp, Condition, GraphPattern};
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
///
/// **S4-09**: When multiple equality predicates target the same label,
/// `composite_index_lookup` is attempted first. Individual single-property
/// lookups are used as fallback.
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

    // S4-09: group equality predicates by (alias, label) for composite lookup.
    let composite_result = try_composite_lookup(collection, &predicates, &alias_to_labels);

    let mut result_set: Option<HashSet<u64>> = composite_result;

    for pred in &predicates {
        // Skip equality predicates already covered by composite lookup.
        if matches!(pred.kind, PredicateKind::Exact(_)) && result_set.is_some() {
            continue;
        }

        let label = resolve_label(&pred.alias, &alias_to_labels)?;
        let ids = execute_single_lookup(collection, label, &pred.property, &pred.kind)?;
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

/// S4-09: attempts a composite index lookup for multiple equality predicates
/// on the same label. Returns `Some(set)` if a composite index exists and
/// covers all equality predicates for at least one label.
fn try_composite_lookup(
    collection: &Collection,
    predicates: &[ExtractedPredicate],
    alias_to_labels: &HashMap<String, Vec<String>>,
) -> Option<HashSet<u64>> {
    // Group equality predicates by alias.
    let mut eq_by_alias: HashMap<&str, Vec<(&str, &serde_json::Value)>> = HashMap::new();
    for pred in predicates {
        if let PredicateKind::Exact(ref val) = pred.kind {
            eq_by_alias
                .entry(&pred.alias)
                .or_default()
                .push((&pred.property, val));
        }
    }

    // Only attempt composite lookup when 2+ equality predicates share an alias.
    for (alias, props) in &eq_by_alias {
        if props.len() < 2 {
            continue;
        }
        let label = resolve_label(alias, alias_to_labels)?;
        let prop_names: Vec<&str> = props.iter().map(|(name, _)| *name).collect();
        let values: Vec<serde_json::Value> = props.iter().map(|(_, v)| (*v).clone()).collect();

        if let Some(ids) = collection.composite_index_lookup(label, &prop_names, &values) {
            return Some(ids.into_iter().collect());
        }
    }
    None
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
    let json_val = resolved.to_json();

    let kind = match cmp.operator {
        CompareOp::Eq => PredicateKind::Exact(json_val),
        CompareOp::Gt => PredicateKind::Gt(json_val),
        CompareOp::Gte => PredicateKind::Gte(json_val),
        CompareOp::Lt => PredicateKind::Lt(json_val),
        CompareOp::Lte => PredicateKind::Lte(json_val),
        CompareOp::NotEq => return None, // Negation cannot be pre-filtered
    };

    Some(ExtractedPredicate {
        alias: alias.to_string(),
        property: property.to_string(),
        kind,
    })
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
mod tests {
    use super::*;
    use crate::velesql::{Comparison, Condition, NodePattern, Value};

    #[test]
    fn test_split_alias_property_valid() {
        assert_eq!(split_alias_property("n.age"), Some(("n", "age")));
        assert_eq!(
            split_alias_property("doc.metadata.category"),
            Some(("doc", "metadata.category"))
        );
    }

    #[test]
    fn test_split_alias_property_no_dot() {
        assert_eq!(split_alias_property("age"), None);
    }

    #[test]
    fn test_split_alias_property_empty_parts() {
        assert_eq!(split_alias_property(".age"), None);
        assert_eq!(split_alias_property("n."), None);
    }

    #[test]
    fn test_extract_predicates_single_eq() {
        let cond = Condition::Comparison(Comparison {
            column: "n.name".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("Alice".to_string()),
        });
        let params = HashMap::new();
        let preds = extract_predicates(&cond, &params);
        assert_eq!(preds.len(), 1);
        assert_eq!(preds[0].alias, "n");
        assert_eq!(preds[0].property, "name");
        assert!(matches!(preds[0].kind, PredicateKind::Exact(_)));
    }

    #[test]
    fn test_extract_predicates_and_chain() {
        let left = Condition::Comparison(Comparison {
            column: "n.age".to_string(),
            operator: CompareOp::Gt,
            value: Value::Integer(30),
        });
        let right = Condition::Comparison(Comparison {
            column: "n.name".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("Bob".to_string()),
        });
        let cond = Condition::And(Box::new(left), Box::new(right));
        let params = HashMap::new();
        let preds = extract_predicates(&cond, &params);
        assert_eq!(preds.len(), 2);
    }

    #[test]
    fn test_extract_predicates_or_skipped() {
        let left = Condition::Comparison(Comparison {
            column: "n.age".to_string(),
            operator: CompareOp::Gt,
            value: Value::Integer(30),
        });
        let right = Condition::Comparison(Comparison {
            column: "n.name".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("Bob".to_string()),
        });
        let cond = Condition::Or(Box::new(left), Box::new(right));
        let params = HashMap::new();
        let preds = extract_predicates(&cond, &params);
        assert!(
            preds.is_empty(),
            "OR branches cannot be pre-filtered with intersection"
        );
    }

    #[test]
    fn test_extract_predicates_not_eq_skipped() {
        let cond = Condition::Comparison(Comparison {
            column: "n.name".to_string(),
            operator: CompareOp::NotEq,
            value: Value::String("Alice".to_string()),
        });
        let params = HashMap::new();
        let preds = extract_predicates(&cond, &params);
        assert!(preds.is_empty(), "NotEq cannot be pre-filtered");
    }

    #[test]
    fn test_extract_predicates_bare_column_skipped() {
        let cond = Condition::Comparison(Comparison {
            column: "age".to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(30),
        });
        let params = HashMap::new();
        let preds = extract_predicates(&cond, &params);
        assert!(
            preds.is_empty(),
            "Bare column names without alias prefix are skipped"
        );
    }

    #[test]
    fn test_build_alias_label_map() {
        let pattern = GraphPattern {
            name: None,
            nodes: vec![
                NodePattern::new().with_alias("n").with_label("Person"),
                NodePattern::new().with_alias("m").with_label("Company"),
            ],
            relationships: Vec::new(),
        };
        let map = build_alias_label_map(&pattern);
        assert_eq!(
            map.get("n").map(|v| v.as_slice()),
            Some(["Person".to_string()].as_slice())
        );
        assert_eq!(
            map.get("m").map(|v| v.as_slice()),
            Some(["Company".to_string()].as_slice())
        );
    }

    #[test]
    fn test_passes_prefilter_none() {
        assert!(passes_prefilter(None, 42), "None = no filter, allow all");
    }

    #[test]
    fn test_passes_prefilter_some_contains() {
        let set: HashSet<u64> = [1, 2, 3].into_iter().collect();
        assert!(passes_prefilter(Some(&set), 2));
        assert!(!passes_prefilter(Some(&set), 99));
    }

    #[test]
    fn test_intersect_sets_none_acc() {
        let new: HashSet<u64> = [1, 2, 3].into_iter().collect();
        let result = intersect_sets(None, new.clone());
        assert_eq!(result, new);
    }

    #[test]
    fn test_intersect_sets_some_acc() {
        let acc: HashSet<u64> = [1, 2, 3].into_iter().collect();
        let new: HashSet<u64> = [2, 3, 4].into_iter().collect();
        let result = intersect_sets(Some(acc), new);
        let expected: HashSet<u64> = [2, 3].into_iter().collect();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_between_to_predicate() {
        let btw = crate::velesql::BetweenCondition {
            column: "n.age".to_string(),
            low: Value::Integer(20),
            high: Value::Integer(40),
        };
        let params = HashMap::new();
        let pred = between_to_predicate(&btw, &params);
        assert!(pred.is_some());
        let pred = pred.expect("test: should have predicate");
        assert_eq!(pred.alias, "n");
        assert_eq!(pred.property, "age");
        assert!(matches!(pred.kind, PredicateKind::Range(_, _)));
    }

    // Regression tests for Devin finding: GTE/LTE must use inclusive bounds,
    // not strict GT/LT (which would exclude boundary values from prefilter).

    #[test]
    fn test_gte_maps_to_gte_not_gt() {
        let cond = Condition::Comparison(Comparison {
            column: "n.age".to_string(),
            operator: CompareOp::Gte,
            value: Value::Integer(30),
        });
        let params = HashMap::new();
        let preds = extract_predicates(&cond, &params);
        assert_eq!(preds.len(), 1);
        assert!(
            matches!(preds[0].kind, PredicateKind::Gte(_)),
            "GTE must map to PredicateKind::Gte (inclusive), not Gt (exclusive)"
        );
    }

    #[test]
    fn test_lte_maps_to_lte_not_lt() {
        let cond = Condition::Comparison(Comparison {
            column: "n.price".to_string(),
            operator: CompareOp::Lte,
            value: Value::Float(99.99),
        });
        let params = HashMap::new();
        let preds = extract_predicates(&cond, &params);
        assert_eq!(preds.len(), 1);
        assert!(
            matches!(preds[0].kind, PredicateKind::Lte(_)),
            "LTE must map to PredicateKind::Lte (inclusive), not Lt (exclusive)"
        );
    }

    #[test]
    fn test_strict_gt_maps_to_gt() {
        let cond = Condition::Comparison(Comparison {
            column: "n.score".to_string(),
            operator: CompareOp::Gt,
            value: Value::Float(0.5),
        });
        let params = HashMap::new();
        let preds = extract_predicates(&cond, &params);
        assert_eq!(preds.len(), 1);
        assert!(
            matches!(preds[0].kind, PredicateKind::Gt(_)),
            "Strict GT must map to PredicateKind::Gt (exclusive)"
        );
    }
}
