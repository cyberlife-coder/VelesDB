//! Vector-search GROUP BY post-processing (Issue #511).
//!
//! Groups `Vec<SearchResult>` by a parent field after vector search,
//! computing score aggregations (MAX_SIM, AVG_SIM) and FIRST projections.
//! This is architecturally distinct from `execute_aggregate` which scans
//! all payloads and returns `serde_json::Value`.

use crate::point::SearchResult;
use crate::velesql::{AggregateArg, AggregateFunction, AggregateType, SelectStatement};
use rustc_hash::FxHashMap;

/// Configuration for vector-search GROUP BY post-processing.
pub(crate) struct VectorGroupByConfig<'a> {
    /// Column(s) to group by (from `GroupByClause`).
    pub group_by_columns: &'a [String],
    /// Aggregate functions requested in SELECT.
    pub aggregations: &'a [AggregateFunction],
    /// LIMIT hint for `FxHashMap` pre-allocation.
    pub limit_hint: Option<usize>,
}

/// Per-group accumulator for single-pass grouping.
///
/// Stack-friendly: 20 bytes total, no heap allocation per group.
struct GroupAccumulator {
    /// Maximum score seen in this group (for MAX_SIM).
    max_score: f32,
    /// Sum of scores (for AVG_SIM = sum / count).
    sum_scores: f32,
    /// Number of chunks in this group.
    count: u32,
    /// Index into the original results `Vec` of the highest-scoring chunk.
    best_chunk_idx: usize,
}

impl GroupAccumulator {
    /// Creates a new accumulator seeded with the first chunk.
    fn new(score: f32, idx: usize) -> Self {
        Self {
            max_score: score,
            sum_scores: score,
            count: 1,
            best_chunk_idx: idx,
        }
    }

    /// Accumulates a chunk into this group.
    fn accumulate(&mut self, score: f32, idx: usize) {
        self.sum_scores += score;
        self.count += 1;
        if score > self.max_score {
            self.max_score = score;
            self.best_chunk_idx = idx;
        }
    }
}

/// Detects whether a query requires vector-search GROUP BY post-processing.
///
/// Returns `true` when the SELECT has a `group_by` clause AND the WHERE clause
/// contains a vector NEAR search condition.
pub(crate) fn is_vector_group_by_query(stmt: &SelectStatement) -> bool {
    let has_group_by = stmt
        .group_by
        .as_ref()
        .is_some_and(|g| !g.columns.is_empty());
    let has_vector_near = stmt
        .where_clause
        .as_ref()
        .is_some_and(crate::velesql::Condition::has_vector_search);
    has_group_by && has_vector_near
}

/// Extracts the score aggregation strategy from the aggregate functions.
///
/// Scans for `MAX(score)` or `AVG(score)` (detected as `Column("score")`
/// with `Max` or `Avg` type). Defaults to `Max` if both are present.
fn extract_score_strategy(aggregations: &[AggregateFunction]) -> Option<AggregateType> {
    let mut found_max = false;
    let mut found_avg = false;
    for agg in aggregations {
        if is_score_aggregate(agg) {
            match agg.function_type {
                AggregateType::Max => found_max = true,
                AggregateType::Avg => found_avg = true,
                _ => {}
            }
        }
    }
    if found_max {
        Some(AggregateType::Max)
    } else if found_avg {
        Some(AggregateType::Avg)
    } else {
        None
    }
}

/// Returns `true` if this aggregate targets the score pseudo-column.
fn is_score_aggregate(agg: &AggregateFunction) -> bool {
    matches!(
        (&agg.function_type, &agg.argument),
        (AggregateType::Max | AggregateType::Avg, AggregateArg::Column(col))
            if col.eq_ignore_ascii_case("score")
    ) || matches!(agg.argument, AggregateArg::Score)
}

/// Extracts the group key value from a result's payload.
fn extract_group_key(
    payload: Option<&serde_json::Value>,
    group_by_columns: &[String],
) -> Option<String> {
    let payload = payload?;
    let mut key_parts = Vec::with_capacity(group_by_columns.len());
    for col in group_by_columns {
        let val = payload.get(col)?;
        key_parts.push(val.to_string());
    }
    Some(key_parts.join("|"))
}

/// Groups vector search results by a parent field, computing score aggregations.
///
/// Single-pass O(N) algorithm using `FxHashMap`. Returns one `SearchResult` per
/// group with the aggregated score and projected payload fields.
pub(crate) fn group_search_results(
    results: &[SearchResult],
    config: &VectorGroupByConfig<'_>,
) -> Vec<SearchResult> {
    if results.is_empty() {
        return Vec::new();
    }

    let score_strategy = extract_score_strategy(config.aggregations);
    let capacity = config.limit_hint.unwrap_or(64).min(results.len());
    let mut groups: FxHashMap<String, GroupAccumulator> =
        FxHashMap::with_capacity_and_hasher(capacity, rustc_hash::FxBuildHasher);

    // Single-pass accumulation.
    for (idx, result) in results.iter().enumerate() {
        let Some(key) = extract_group_key(result.point.payload.as_ref(), config.group_by_columns)
        else {
            tracing::debug!(
                id = result.point.id,
                fields = ?config.group_by_columns,
                "Skipping chunk: missing group-by field"
            );
            continue;
        };
        groups
            .entry(key)
            .and_modify(|acc| acc.accumulate(result.score, idx))
            .or_insert_with(|| GroupAccumulator::new(result.score, idx));
    }

    // Build grouped results.
    groups
        .into_values()
        .map(|acc| {
            build_grouped_result(
                &acc,
                config.group_by_columns,
                config.aggregations,
                results,
                score_strategy,
            )
        })
        .collect()
}

/// Builds a single grouped `SearchResult` from a `GroupAccumulator`.
///
/// Constructs the payload with the group key, FIRST projections (from the
/// best chunk), and sets the score to the aggregated value.
fn build_grouped_result(
    acc: &GroupAccumulator,
    group_by_columns: &[String],
    aggregations: &[AggregateFunction],
    original_results: &[SearchResult],
    score_strategy: Option<AggregateType>,
) -> SearchResult {
    let best = &original_results[acc.best_chunk_idx];
    let best_payload = best.point.payload.as_ref();

    let mut payload = serde_json::Map::new();

    // Insert group key values.
    if let Some(bp) = best_payload {
        for col in group_by_columns {
            if let Some(val) = bp.get(col) {
                payload.insert(col.clone(), val.clone());
            }
        }
    }

    // Insert aggregation results.
    insert_aggregation_values(&mut payload, acc, aggregations, best_payload);

    // Compute aggregated score.
    let score = compute_group_score(acc, score_strategy);

    let mut point = best.point.clone();
    point.payload = Some(serde_json::Value::Object(payload));

    SearchResult {
        point,
        score,
        component_scores: None,
    }
}

/// Inserts aggregation values (FIRST projections, score aliases) into the payload.
fn insert_aggregation_values(
    payload: &mut serde_json::Map<String, serde_json::Value>,
    acc: &GroupAccumulator,
    aggregations: &[AggregateFunction],
    best_payload: Option<&serde_json::Value>,
) {
    for agg in aggregations {
        let key = aggregation_result_key(agg);
        let value = compute_agg_value(agg, acc, best_payload);
        payload.insert(key, value);
    }
}

/// Computes the result key for an aggregation function.
fn aggregation_result_key(agg: &AggregateFunction) -> String {
    if let Some(ref alias) = agg.alias {
        return alias.clone();
    }
    if is_score_aggregate(agg) {
        let prefix = match agg.function_type {
            AggregateType::Max => "max",
            AggregateType::Avg => "avg",
            _ => "agg",
        };
        return format!("{prefix}_score");
    }
    if let AggregateArg::Column(col) = &agg.argument {
        let prefix = match agg.function_type {
            AggregateType::First => "first",
            AggregateType::Count => "count",
            AggregateType::Sum => "sum",
            AggregateType::Avg => "avg",
            AggregateType::Min => "min",
            AggregateType::Max => "max",
        };
        return format!("{prefix}_{col}");
    }
    format!("{:?}_{}", agg.function_type, arg_name(&agg.argument))
}

/// Returns a display name for an aggregate argument.
fn arg_name(arg: &AggregateArg) -> &str {
    match arg {
        AggregateArg::Wildcard => "*",
        AggregateArg::Column(col) => col.as_str(),
        AggregateArg::Score => "score",
    }
}

/// Computes the value for a single aggregation in a group.
fn compute_agg_value(
    agg: &AggregateFunction,
    acc: &GroupAccumulator,
    best_payload: Option<&serde_json::Value>,
) -> serde_json::Value {
    if is_score_aggregate(agg) {
        return match agg.function_type {
            AggregateType::Max => serde_json::json!(acc.max_score),
            AggregateType::Avg => {
                // Reason: count is bounded by result set size, precision loss acceptable
                #[allow(clippy::cast_precision_loss)]
                let avg = if acc.count > 0 {
                    acc.sum_scores / acc.count as f32
                } else {
                    0.0
                };
                serde_json::json!(avg)
            }
            _ => serde_json::Value::Null,
        };
    }
    if matches!(agg.function_type, AggregateType::First) {
        if let AggregateArg::Column(col) = &agg.argument {
            return best_payload
                .and_then(|p| p.get(col))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
        }
    }
    serde_json::Value::Null
}

/// Computes the group score based on the aggregation strategy.
fn compute_group_score(acc: &GroupAccumulator, strategy: Option<AggregateType>) -> f32 {
    match strategy {
        Some(AggregateType::Avg) => {
            // Reason: count is bounded by result set size, precision loss acceptable
            #[allow(clippy::cast_precision_loss)]
            if acc.count > 0 {
                acc.sum_scores / acc.count as f32
            } else {
                0.0
            }
        }
        _ => acc.max_score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point::Point;

    /// Creates a `SearchResult` with the given id, score, and payload.
    fn make_result(id: u64, score: f32, payload: serde_json::Value) -> SearchResult {
        SearchResult {
            point: Point {
                id,
                vector: vec![0.0; 4],
                payload: Some(payload),
                sparse_vectors: None,
            },
            score,
            component_scores: None,
        }
    }

    fn max_score_agg(alias: Option<&str>) -> AggregateFunction {
        AggregateFunction {
            function_type: AggregateType::Max,
            argument: AggregateArg::Column("score".to_string()),
            alias: alias.map(String::from),
        }
    }

    fn avg_score_agg(alias: Option<&str>) -> AggregateFunction {
        AggregateFunction {
            function_type: AggregateType::Avg,
            argument: AggregateArg::Column("score".to_string()),
            alias: alias.map(String::from),
        }
    }

    fn first_agg(col: &str, alias: Option<&str>) -> AggregateFunction {
        AggregateFunction {
            function_type: AggregateType::First,
            argument: AggregateArg::Column(col.to_string()),
            alias: alias.map(String::from),
        }
    }

    #[test]
    fn test_group_accumulator_single_chunk() {
        let results = vec![make_result(
            1,
            0.9,
            serde_json::json!({"parent": "A", "text": "hello"}),
        )];
        let aggs = vec![max_score_agg(Some("relevance"))];
        let config = VectorGroupByConfig {
            group_by_columns: &["parent".to_string()],
            aggregations: &aggs,
            limit_hint: Some(10),
        };
        let grouped = group_search_results(&results, &config);
        assert_eq!(grouped.len(), 1);
        assert!((grouped[0].score - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn test_group_accumulator_multiple_chunks() {
        let results = vec![
            make_result(1, 0.5, serde_json::json!({"parent": "A", "text": "low"})),
            make_result(2, 0.9, serde_json::json!({"parent": "A", "text": "high"})),
            make_result(3, 0.7, serde_json::json!({"parent": "A", "text": "mid"})),
            make_result(4, 0.8, serde_json::json!({"parent": "B", "text": "only"})),
        ];
        let aggs = vec![
            max_score_agg(Some("relevance")),
            first_agg("text", Some("excerpt")),
        ];
        let config = VectorGroupByConfig {
            group_by_columns: &["parent".to_string()],
            aggregations: &aggs,
            limit_hint: Some(10),
        };
        let grouped = group_search_results(&results, &config);
        assert_eq!(grouped.len(), 2);

        let group_a = grouped
            .iter()
            .find(|r| {
                r.point
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("parent"))
                    .and_then(|v| v.as_str())
                    == Some("A")
            })
            .expect("group A");
        assert!((group_a.score - 0.9).abs() < f32::EPSILON);
        let excerpt = group_a
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("excerpt"))
            .and_then(|v| v.as_str());
        assert_eq!(excerpt, Some("high"));
    }

    #[test]
    fn test_group_skip_missing_parent_field() {
        let results = vec![
            make_result(1, 0.9, serde_json::json!({"parent": "A"})),
            make_result(2, 0.8, serde_json::json!({"other": "no parent"})),
            make_result(3, 0.7, serde_json::json!({"parent": "A"})),
        ];
        let aggs = vec![max_score_agg(Some("relevance"))];
        let config = VectorGroupByConfig {
            group_by_columns: &["parent".to_string()],
            aggregations: &aggs,
            limit_hint: Some(10),
        };
        let grouped = group_search_results(&results, &config);
        assert_eq!(grouped.len(), 1);
    }

    #[test]
    fn test_first_null_when_column_missing() {
        let results = vec![make_result(1, 0.9, serde_json::json!({"parent": "A"}))];
        let aggs = vec![first_agg("nonexistent", Some("val"))];
        let config = VectorGroupByConfig {
            group_by_columns: &["parent".to_string()],
            aggregations: &aggs,
            limit_hint: Some(10),
        };
        let grouped = group_search_results(&results, &config);
        assert_eq!(grouped.len(), 1);
        let val = grouped[0].point.payload.as_ref().and_then(|p| p.get("val"));
        assert_eq!(val, Some(&serde_json::Value::Null));
    }

    #[test]
    fn test_is_vector_group_by_query_true() {
        let stmt = SelectStatement {
            group_by: Some(crate::velesql::GroupByClause {
                columns: vec!["parent".to_string()],
            }),
            where_clause: Some(crate::velesql::Condition::VectorSearch(
                crate::velesql::VectorSearch {
                    vector: crate::velesql::VectorExpr::Literal(vec![1.0, 0.0, 0.0, 0.0]),
                },
            )),
            ..SelectStatement::empty()
        };
        assert!(is_vector_group_by_query(&stmt));
    }

    #[test]
    fn test_is_vector_group_by_query_false_no_near() {
        let stmt = SelectStatement {
            group_by: Some(crate::velesql::GroupByClause {
                columns: vec!["parent".to_string()],
            }),
            where_clause: None,
            ..SelectStatement::empty()
        };
        assert!(!is_vector_group_by_query(&stmt));
    }

    #[test]
    fn test_is_vector_group_by_query_false_no_group_by() {
        let stmt = SelectStatement {
            group_by: None,
            where_clause: Some(crate::velesql::Condition::VectorSearch(
                crate::velesql::VectorSearch {
                    vector: crate::velesql::VectorExpr::Literal(vec![1.0, 0.0, 0.0, 0.0]),
                },
            )),
            ..SelectStatement::empty()
        };
        assert!(!is_vector_group_by_query(&stmt));
    }

    #[test]
    fn test_avg_score_strategy() {
        let results = vec![
            make_result(1, 0.5, serde_json::json!({"parent": "A"})),
            make_result(2, 0.9, serde_json::json!({"parent": "A"})),
        ];
        let aggs = vec![avg_score_agg(Some("relevance"))];
        let config = VectorGroupByConfig {
            group_by_columns: &["parent".to_string()],
            aggregations: &aggs,
            limit_hint: Some(10),
        };
        let grouped = group_search_results(&results, &config);
        assert_eq!(grouped.len(), 1);
        let expected_avg = (0.5 + 0.9) / 2.0;
        assert!((grouped[0].score - expected_avg).abs() < 0.001);
    }
}
