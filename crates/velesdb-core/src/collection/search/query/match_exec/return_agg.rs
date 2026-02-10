//! RETURN aggregation for MATCH query results (VP-005).
//!
//! Detects aggregation functions (COUNT, AVG, SUM, MIN, MAX) in RETURN items,
//! classifies items into grouping keys vs aggregations, and computes grouped
//! aggregation results following OpenCypher implicit grouping semantics.

use super::{parse_property_path, MatchResult};
use crate::collection::types::Collection;
use crate::storage::PayloadStorage;
use crate::velesql::{ReturnClause, ReturnItem};
use std::collections::HashMap;

/// Recognized aggregation types for RETURN expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AggType {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

/// A parsed aggregation from a RETURN expression.
#[derive(Debug, Clone)]
struct ReturnAggregation {
    agg_type: AggType,
    /// Column inside the aggregation, e.g. "p.success_rate" or "*".
    column: String,
    /// Key to use in the result map (alias or original expression).
    result_key: String,
}

/// Parses a RETURN expression to detect aggregation functions.
///
/// Returns `Some((agg_type, column))` for expressions like `COUNT(*)`, `AVG(p.score)`.
/// Returns `None` for plain property paths like `d.name`.
fn parse_return_aggregation(expression: &str) -> Option<(AggType, String)> {
    let trimmed = expression.trim();

    // Find opening paren
    let open = trimmed.find('(')?;
    let close = trimmed.rfind(')')?;
    if close <= open {
        return None;
    }

    let func_name = trimmed[..open].trim().to_uppercase();
    let arg = trimmed[open + 1..close].trim().to_string();

    let agg_type = match func_name.as_str() {
        "COUNT" => AggType::Count,
        "SUM" => AggType::Sum,
        "AVG" => AggType::Avg,
        "MIN" => AggType::Min,
        "MAX" => AggType::Max,
        _ => return None,
    };

    Some((agg_type, arg))
}

/// Classifies RETURN items into grouping keys and aggregations.
///
/// Non-aggregation items become implicit grouping keys (OpenCypher standard).
fn classify_return_items(items: &[ReturnItem]) -> (Vec<&ReturnItem>, Vec<ReturnAggregation>) {
    let mut grouping_keys: Vec<&ReturnItem> = Vec::new();
    let mut aggregations: Vec<ReturnAggregation> = Vec::new();

    for item in items {
        if let Some((agg_type, column)) = parse_return_aggregation(&item.expression) {
            let result_key = item
                .alias
                .clone()
                .unwrap_or_else(|| item.expression.clone());
            aggregations.push(ReturnAggregation {
                agg_type,
                column,
                result_key,
            });
        } else {
            grouping_keys.push(item);
        }
    }

    (grouping_keys, aggregations)
}

/// Resolves a dot-qualified column (e.g. "p.success_rate") from bindings + payloads.
///
/// Splits on first dot to get (alias, property), resolves alias from bindings,
/// then fetches the property from that node's payload.
fn resolve_column_value(
    column: &str,
    bindings: &HashMap<String, u64>,
    payload_storage: &dyn PayloadStorage,
) -> Option<serde_json::Value> {
    let (alias, property) = parse_property_path(column)?;
    let &node_id = bindings.get(alias)?;
    let payload = payload_storage.retrieve(node_id).ok()??;
    let map = payload.as_object()?;
    Collection::get_nested_property(map, property).cloned()
}

/// Builds a grouping key string from a `MatchResult` for the given grouping items.
///
/// The key is a concatenation of JSON-serialized values for each grouping column.
fn build_group_key(
    result: &MatchResult,
    grouping_keys: &[&ReturnItem],
    payload_storage: &dyn PayloadStorage,
) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(grouping_keys.len());
    for item in grouping_keys {
        let value = resolve_column_value(&item.expression, &result.bindings, payload_storage)
            .unwrap_or(serde_json::Value::Null);
        parts.push(value.to_string());
    }
    parts.join("\x1F") // Unit separator — unlikely to collide with real data
}

impl Collection {
    /// Checks RETURN clause for aggregation and computes grouped results if present.
    ///
    /// Returns `None` if no aggregation functions are detected (caller uses normal projection).
    /// Returns `Some(aggregated_results)` if aggregation is present.
    ///
    /// OpenCypher implicit grouping: non-aggregated items become grouping keys.
    pub(crate) fn aggregate_match_results(
        &self,
        results: &[MatchResult],
        return_clause: &ReturnClause,
    ) -> Option<Vec<MatchResult>> {
        let (grouping_keys, aggregations) = classify_return_items(&return_clause.items);

        // No aggregation detected → return None (normal projection path)
        if aggregations.is_empty() {
            return None;
        }

        let payload_storage = self.payload_storage.read();

        // Group results by grouping key values
        // Preserves insertion order via Vec of (key_string, group_members)
        let mut group_order: Vec<String> = Vec::new();
        let mut groups: HashMap<String, Vec<&MatchResult>> = HashMap::new();

        for result in results {
            let key = build_group_key(result, &grouping_keys, &*payload_storage);
            if !groups.contains_key(&key) {
                group_order.push(key.clone());
            }
            groups.entry(key).or_default().push(result);
        }

        // Build aggregated results — one MatchResult per group
        let mut aggregated: Vec<MatchResult> = Vec::with_capacity(group_order.len());

        for group_key in &group_order {
            let members = &groups[group_key];
            // Reason: first member provides representative node_id/bindings for the group
            let representative = members[0];

            let mut projected: HashMap<String, serde_json::Value> = HashMap::new();

            // Project grouping key values from representative
            for item in &grouping_keys {
                let key = item
                    .alias
                    .clone()
                    .unwrap_or_else(|| item.expression.clone());
                let value = resolve_column_value(
                    &item.expression,
                    &representative.bindings,
                    &*payload_storage,
                )
                .unwrap_or(serde_json::Value::Null);
                projected.insert(key, value);
            }

            // Compute each aggregation over the group
            for agg in &aggregations {
                let value = compute_aggregation(agg, members, &*payload_storage);
                projected.insert(agg.result_key.clone(), value);
            }

            let mut agg_result =
                MatchResult::new(representative.node_id, representative.depth, Vec::new());
            agg_result.bindings.clone_from(&representative.bindings);
            agg_result.projected = projected;
            aggregated.push(agg_result);
        }

        Some(aggregated)
    }
}

/// Computes a single aggregation over a group of MatchResults.
fn compute_aggregation(
    agg: &ReturnAggregation,
    members: &[&MatchResult],
    payload_storage: &dyn PayloadStorage,
) -> serde_json::Value {
    match agg.agg_type {
        AggType::Count => {
            if agg.column == "*" {
                serde_json::json!(members.len())
            } else {
                // COUNT(column) = number of non-null values
                let count = members
                    .iter()
                    .filter(|m| {
                        resolve_column_value(&agg.column, &m.bindings, payload_storage).is_some()
                    })
                    .count();
                serde_json::json!(count)
            }
        }
        AggType::Sum => {
            let sum: f64 = members
                .iter()
                .filter_map(|m| {
                    resolve_column_value(&agg.column, &m.bindings, payload_storage)
                        .and_then(|v| v.as_f64())
                })
                .sum();
            serde_json::json!(sum)
        }
        AggType::Avg => {
            let values: Vec<f64> = members
                .iter()
                .filter_map(|m| {
                    resolve_column_value(&agg.column, &m.bindings, payload_storage)
                        .and_then(|v| v.as_f64())
                })
                .collect();
            if values.is_empty() {
                serde_json::Value::Null
            } else {
                let sum: f64 = values.iter().sum();
                // Reason: values.len() is always > 0 here (checked above)
                #[allow(clippy::cast_precision_loss)]
                let avg = sum / values.len() as f64;
                serde_json::json!(avg)
            }
        }
        AggType::Min => {
            let min = members
                .iter()
                .filter_map(|m| {
                    resolve_column_value(&agg.column, &m.bindings, payload_storage)
                        .and_then(|v| v.as_f64())
                })
                .fold(f64::INFINITY, f64::min);
            if min == f64::INFINITY {
                serde_json::Value::Null
            } else {
                serde_json::json!(min)
            }
        }
        AggType::Max => {
            let max = members
                .iter()
                .filter_map(|m| {
                    resolve_column_value(&agg.column, &m.bindings, payload_storage)
                        .and_then(|v| v.as_f64())
                })
                .fold(f64::NEG_INFINITY, f64::max);
            if max == f64::NEG_INFINITY {
                serde_json::Value::Null
            } else {
                serde_json::json!(max)
            }
        }
    }
}
