//! Aggregation query execution for VelesQL (EPIC-017 US-002, US-003, US-006).
//!
//! Implements streaming aggregation with O(1) memory complexity.
//! Supports GROUP BY for grouped aggregations (US-003).
//! Supports HAVING for filtering groups (US-006).
//! Supports parallel aggregation with rayon (EPIC-018 US-001).

// SAFETY: Numeric casts in aggregation are intentional:
// - All casts are for computing aggregate statistics (sum, avg, count)
// - f64/u64 casts for maintaining precision in intermediate calculations
// - Values bounded by result set size and field cardinality
// - Precision loss acceptable for aggregation results
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

use crate::collection::types::Collection;
use crate::error::Result;
use crate::storage::{PayloadStorage, VectorStorage};
use crate::velesql::{
    AggregateArg, AggregateFunction, AggregateResult, AggregateType, Aggregator, CompareOp,
    HavingClause, Query, SelectColumns, Value,
};
use rayon::prelude::*;
use rustc_hash::FxHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Group key for GROUP BY operations with pre-computed hash.
/// Avoids JSON serialization overhead by using direct value hashing.
#[derive(Clone)]
struct GroupKey {
    /// Original values for result construction
    values: Vec<serde_json::Value>,
    /// Pre-computed hash for fast HashMap lookup
    hash: u64,
}

impl GroupKey {
    fn new(values: Vec<serde_json::Value>) -> Self {
        let hash = Self::compute_hash(&values);
        Self { values, hash }
    }

    fn compute_hash(values: &[serde_json::Value]) -> u64 {
        let mut hasher = FxHasher::default();
        for v in values {
            Self::hash_value(v, &mut hasher);
        }
        hasher.finish()
    }

    fn hash_value(value: &serde_json::Value, hasher: &mut FxHasher) {
        match value {
            serde_json::Value::Null => 0u8.hash(hasher),
            serde_json::Value::Bool(b) => {
                1u8.hash(hasher);
                b.hash(hasher);
            }
            serde_json::Value::Number(n) => {
                2u8.hash(hasher);
                // Use bits for consistent hashing of floats
                if let Some(f) = n.as_f64() {
                    f.to_bits().hash(hasher);
                }
            }
            serde_json::Value::String(s) => {
                3u8.hash(hasher);
                s.hash(hasher);
            }
            _ => {
                // Arrays and objects: fallback to string representation
                4u8.hash(hasher);
                value.to_string().hash(hasher);
            }
        }
    }
}

impl Hash for GroupKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl PartialEq for GroupKey {
    fn eq(&self, other: &Self) -> bool {
        // Fast path: different hash means definitely different
        self.hash == other.hash && self.values == other.values
    }
}

impl Eq for GroupKey {}

/// Default maximum number of groups allowed (memory protection).
/// Can be overridden via WITH(max_groups=N) or WITH(group_limit=N).
const DEFAULT_MAX_GROUPS: usize = 10000;

/// Threshold for switching to parallel aggregation.
/// Below this, sequential is faster due to overhead.
const PARALLEL_THRESHOLD: usize = 10_000;

/// Chunk size for parallel processing.
const CHUNK_SIZE: usize = 1000;

impl Collection {
    /// Execute an aggregation query and return results as JSON.
    ///
    /// Supports COUNT(*), COUNT(column), SUM, AVG, MIN, MAX.
    /// Uses streaming aggregation - O(1) memory, single pass over data.
    ///
    /// # Arguments
    ///
    /// * `query` - Parsed VelesQL query with aggregation functions
    /// * `params` - Query parameters for placeholders
    ///
    /// # Returns
    ///
    /// JSON object with aggregation results, e.g.:
    /// ```json
    /// {"count": 100, "sum_price": 5000.0, "avg_rating": 4.5}
    /// ```
    #[allow(clippy::too_many_lines)]
    pub fn execute_aggregate(
        &self,
        query: &Query,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let stmt = &query.select;

        // Extract aggregation functions from SELECT clause
        let aggregations: &[AggregateFunction] = match &stmt.columns {
            SelectColumns::Aggregations(aggs) => aggs,
            SelectColumns::Mixed { aggregations, .. } => aggregations,
            _ => {
                return Err(crate::error::Error::Config(
                    "execute_aggregate requires aggregation functions in SELECT".to_string(),
                ))
            }
        };

        // Check if GROUP BY is present
        if let Some(ref group_by) = stmt.group_by {
            return self.execute_grouped_aggregate(
                query,
                aggregations,
                &group_by.columns,
                stmt.having.as_ref(),
                params,
            );
        }

        // HAVING without GROUP BY is invalid - return error
        if stmt.having.is_some() {
            return Err(crate::error::Error::Config(
                "HAVING clause requires GROUP BY clause".to_string(),
            ));
        }

        // BUG-5 FIX: Resolve parameter placeholders in WHERE clause before creating filter
        let filter = stmt.where_clause.as_ref().map(|cond| {
            let resolved = Self::resolve_condition_params(cond, params);
            crate::filter::Filter::new(crate::filter::Condition::from(resolved))
        });

        // Create aggregator
        let mut aggregator = Aggregator::new();

        // Determine which columns we need to aggregate (deduplicated)
        let columns_to_aggregate: std::collections::HashSet<&str> = aggregations
            .iter()
            .filter_map(|agg| match &agg.argument {
                AggregateArg::Column(col) => Some(col.as_str()),
                AggregateArg::Wildcard => None, // COUNT(*) doesn't need column access
            })
            .collect();

        let has_count_star = aggregations
            .iter()
            .any(|agg| matches!(agg.argument, AggregateArg::Wildcard));

        // Collect all IDs for parallel processing decision
        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();
        let ids: Vec<u64> = vector_storage.ids();
        let total_count = ids.len();

        // Use parallel aggregation for large datasets
        let agg_result = if total_count >= PARALLEL_THRESHOLD {
            // PARALLEL: Pre-fetch all payloads (sequential) to avoid lock contention
            let payloads: Vec<Option<serde_json::Value>> = ids
                .iter()
                .map(|&id| payload_storage.retrieve(id).ok().flatten())
                .collect();

            // Drop the lock before parallel processing
            drop(payload_storage);
            drop(vector_storage);

            let columns_vec: Vec<String> = columns_to_aggregate
                .iter()
                .map(|s| (*s).to_string())
                .collect();

            // Parallel aggregation on pre-fetched data (no lock contention)
            let partial_aggregators: Vec<Aggregator> = payloads
                .par_chunks(CHUNK_SIZE)
                .map(|chunk| {
                    let mut chunk_agg = Aggregator::new();
                    for payload in chunk {
                        // Apply filter if present
                        if let Some(ref f) = filter {
                            let matches = match payload {
                                Some(ref p) => f.matches(p),
                                None => f.matches(&serde_json::Value::Null),
                            };
                            if !matches {
                                continue;
                            }
                        }

                        // Process COUNT(*)
                        if has_count_star {
                            chunk_agg.process_count();
                        }

                        // Process column aggregations
                        if let Some(ref p) = payload {
                            for col in &columns_vec {
                                if let Some(value) = Self::get_nested_value(p, col) {
                                    chunk_agg.process_value(col, value);
                                }
                            }
                        }
                    }
                    chunk_agg
                })
                .collect();

            // Merge all partial results
            let mut final_agg = Aggregator::new();
            for partial in partial_aggregators {
                final_agg.merge(partial);
            }
            final_agg.finalize()
        } else {
            // SEQUENTIAL: Original single-pass for small datasets
            for id in ids {
                let payload = payload_storage.retrieve(id).ok().flatten();

                // Apply filter if present
                if let Some(ref f) = filter {
                    let matches = match payload {
                        Some(ref p) => f.matches(p),
                        None => f.matches(&serde_json::Value::Null),
                    };
                    if !matches {
                        continue;
                    }
                }

                // Process COUNT(*)
                if has_count_star {
                    aggregator.process_count();
                }

                // Process column aggregations
                if let Some(ref p) = payload {
                    for col in &columns_to_aggregate {
                        if let Some(value) = Self::get_nested_value(p, col) {
                            aggregator.process_value(col, value);
                        }
                    }
                }
            }
            aggregator.finalize()
        };
        let mut result = serde_json::Map::new();

        // Build result based on requested aggregations
        for agg in aggregations {
            let key = if let Some(ref alias) = agg.alias {
                alias.clone()
            } else {
                match &agg.argument {
                    AggregateArg::Wildcard => "count".to_string(),
                    AggregateArg::Column(col) => {
                        let prefix = match agg.function_type {
                            AggregateType::Count => "count",
                            AggregateType::Sum => "sum",
                            AggregateType::Avg => "avg",
                            AggregateType::Min => "min",
                            AggregateType::Max => "max",
                        };
                        format!("{prefix}_{col}")
                    }
                }
            };

            let value = match (&agg.function_type, &agg.argument) {
                (AggregateType::Count, AggregateArg::Wildcard) => {
                    serde_json::json!(agg_result.count)
                }
                (AggregateType::Count, AggregateArg::Column(col)) => {
                    // COUNT(column) = number of non-null values for this column
                    let count = agg_result.counts.get(col.as_str()).copied().unwrap_or(0);
                    serde_json::json!(count)
                }
                (AggregateType::Sum, AggregateArg::Column(col)) => agg_result
                    .sums
                    .get(col.as_str())
                    .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                (AggregateType::Avg, AggregateArg::Column(col)) => agg_result
                    .avgs
                    .get(col.as_str())
                    .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                (AggregateType::Min, AggregateArg::Column(col)) => agg_result
                    .mins
                    .get(col.as_str())
                    .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                (AggregateType::Max, AggregateArg::Column(col)) => agg_result
                    .maxs
                    .get(col.as_str())
                    .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                _ => serde_json::Value::Null,
            };

            result.insert(key, value);
        }

        Ok(serde_json::Value::Object(result))
    }

    /// Get a nested value from JSON payload using dot notation.
    fn get_nested_value<'a>(
        payload: &'a serde_json::Value,
        path: &str,
    ) -> Option<&'a serde_json::Value> {
        let parts: Vec<&str> = path.split('.').collect();
        let mut current = payload;

        for part in parts {
            match current {
                serde_json::Value::Object(map) => {
                    current = map.get(part)?;
                }
                _ => return None,
            }
        }

        Some(current)
    }

    /// Execute a grouped aggregation query (GROUP BY) with optional HAVING filter.
    #[allow(clippy::too_many_lines)]
    fn execute_grouped_aggregate(
        &self,
        query: &Query,
        aggregations: &[AggregateFunction],
        group_by_columns: &[String],
        having: Option<&HavingClause>,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let stmt = &query.select;

        // EPIC-040 US-004: Extract max_groups from WITH clause if present
        let max_groups = Self::extract_max_groups_limit(stmt.with_clause.as_ref());

        // BUG-5 FIX: Build filter from WHERE clause with parameter resolution
        let filter = stmt.where_clause.as_ref().map(|cond| {
            let resolved = Self::resolve_condition_params(cond, params);
            crate::filter::Filter::new(crate::filter::Condition::from(resolved))
        });

        // HashMap: GroupKey -> Aggregator (optimized with pre-computed hash)
        let mut groups: HashMap<GroupKey, Aggregator> = HashMap::new();

        // Determine which columns we need to aggregate
        let columns_to_aggregate: std::collections::HashSet<&str> = aggregations
            .iter()
            .filter_map(|agg| match &agg.argument {
                AggregateArg::Column(col) => Some(col.as_str()),
                AggregateArg::Wildcard => None,
            })
            .collect();

        let has_count_star = aggregations
            .iter()
            .any(|agg| matches!(agg.argument, AggregateArg::Wildcard));

        // Stream through all points
        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();
        let ids = vector_storage.ids();

        for id in ids {
            let payload = payload_storage.retrieve(id).ok().flatten();

            // Apply filter if present
            if let Some(ref f) = filter {
                let matches = match payload {
                    Some(ref p) => f.matches(p),
                    None => f.matches(&serde_json::Value::Null),
                };
                if !matches {
                    continue;
                }
            }

            // Extract group key from payload (optimized: no JSON serialization)
            let group_key = Self::extract_group_key_fast(payload.as_ref(), group_by_columns);

            // Check group limit (configurable via WITH clause)
            if !groups.contains_key(&group_key) && groups.len() >= max_groups {
                return Err(crate::error::Error::Config(format!(
                    "Too many groups (limit: {max_groups})"
                )));
            }

            // Get or create aggregator for this group
            let aggregator = groups.entry(group_key).or_default();

            // Process COUNT(*)
            if has_count_star {
                aggregator.process_count();
            }

            // Process column aggregations
            if let Some(ref p) = payload {
                for col in &columns_to_aggregate {
                    if let Some(value) = Self::get_nested_value(p, col) {
                        aggregator.process_value(col, value);
                    }
                }
            }
        }

        // Build result array with HAVING filter
        let mut results = Vec::new();

        for (group_key, aggregator) in groups {
            let agg_result = aggregator.finalize();

            // Apply HAVING filter if present
            if let Some(having_clause) = having {
                if !Self::evaluate_having(having_clause, &agg_result) {
                    continue; // Skip groups that don't match HAVING
                }
            }

            let mut row = serde_json::Map::new();

            // Use group key values directly (no JSON parsing needed)
            for (i, col_name) in group_by_columns.iter().enumerate() {
                if let Some(val) = group_key.values.get(i) {
                    row.insert(col_name.clone(), val.clone());
                }
            }

            // Add aggregation results
            for agg in aggregations {
                let key = if let Some(ref alias) = agg.alias {
                    alias.clone()
                } else {
                    match &agg.argument {
                        AggregateArg::Wildcard => "count".to_string(),
                        AggregateArg::Column(col) => {
                            let prefix = match agg.function_type {
                                AggregateType::Count => "count",
                                AggregateType::Sum => "sum",
                                AggregateType::Avg => "avg",
                                AggregateType::Min => "min",
                                AggregateType::Max => "max",
                            };
                            format!("{prefix}_{col}")
                        }
                    }
                };

                let value = match (&agg.function_type, &agg.argument) {
                    (AggregateType::Count, AggregateArg::Wildcard) => {
                        serde_json::json!(agg_result.count)
                    }
                    (AggregateType::Count, AggregateArg::Column(col)) => {
                        // COUNT(column) = number of non-null values for this column
                        let count = agg_result.counts.get(col.as_str()).copied().unwrap_or(0);
                        serde_json::json!(count)
                    }
                    (AggregateType::Sum, AggregateArg::Column(col)) => agg_result
                        .sums
                        .get(col.as_str())
                        .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                    (AggregateType::Avg, AggregateArg::Column(col)) => agg_result
                        .avgs
                        .get(col.as_str())
                        .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                    (AggregateType::Min, AggregateArg::Column(col)) => agg_result
                        .mins
                        .get(col.as_str())
                        .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                    (AggregateType::Max, AggregateArg::Column(col)) => agg_result
                        .maxs
                        .get(col.as_str())
                        .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                    _ => serde_json::Value::Null,
                };

                row.insert(key, value);
            }

            results.push(serde_json::Value::Object(row));
        }

        // BUG-3 FIX: Apply ORDER BY to grouped aggregation results
        if let Some(ref order_by) = stmt.order_by {
            Self::sort_aggregation_results(&mut results, order_by);
        }

        Ok(serde_json::Value::Array(results))
    }

    /// BUG-3 FIX: Sort aggregation results by ORDER BY clause.
    fn sort_aggregation_results(
        results: &mut [serde_json::Value],
        order_by: &[crate::velesql::SelectOrderBy],
    ) {
        use crate::velesql::OrderByExpr;

        results.sort_by(|a, b| {
            for ob in order_by {
                let ordering = match &ob.expr {
                    OrderByExpr::Field(field) => {
                        let val_a = a.get(field);
                        let val_b = b.get(field);
                        Self::compare_json_values(val_a, val_b)
                    }
                    OrderByExpr::Aggregate(agg) => {
                        // Get the key name for this aggregate
                        let key = if let Some(ref alias) = agg.alias {
                            alias.clone()
                        } else {
                            match &agg.argument {
                                crate::velesql::AggregateArg::Wildcard => "count".to_string(),
                                crate::velesql::AggregateArg::Column(col) => {
                                    let prefix = match agg.function_type {
                                        crate::velesql::AggregateType::Count => "count",
                                        crate::velesql::AggregateType::Sum => "sum",
                                        crate::velesql::AggregateType::Avg => "avg",
                                        crate::velesql::AggregateType::Min => "min",
                                        crate::velesql::AggregateType::Max => "max",
                                    };
                                    format!("{prefix}_{col}")
                                }
                            }
                        };
                        let val_a = a.get(&key);
                        let val_b = b.get(&key);
                        Self::compare_json_values(val_a, val_b)
                    }
                    OrderByExpr::Similarity(_) => std::cmp::Ordering::Equal, // Not applicable for aggregations
                };

                let ordering = if ob.descending {
                    ordering.reverse()
                } else {
                    ordering
                };

                if ordering != std::cmp::Ordering::Equal {
                    return ordering;
                }
            }
            std::cmp::Ordering::Equal
        });
    }

    /// Compare two JSON values for sorting.
    fn compare_json_values(
        a: Option<&serde_json::Value>,
        b: Option<&serde_json::Value>,
    ) -> std::cmp::Ordering {
        match (a, b) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(va), Some(vb)) => {
                // Numeric comparison
                if let (Some(na), Some(nb)) = (va.as_f64(), vb.as_f64()) {
                    return na.total_cmp(&nb);
                }
                // String comparison
                if let (Some(sa), Some(sb)) = (va.as_str(), vb.as_str()) {
                    return sa.cmp(sb);
                }
                // Fallback: compare as strings
                va.to_string().cmp(&vb.to_string())
            }
        }
    }

    /// Extract group key from payload with pre-computed hash (optimized).
    /// Avoids JSON serialization overhead by using direct value hashing.
    fn extract_group_key_fast(
        payload: Option<&serde_json::Value>,
        group_by_columns: &[String],
    ) -> GroupKey {
        let values: Vec<serde_json::Value> = group_by_columns
            .iter()
            .map(|col| {
                payload
                    .and_then(|p| Self::get_nested_value(p, col).cloned())
                    .unwrap_or(serde_json::Value::Null)
            })
            .collect();
        GroupKey::new(values)
    }

    /// Evaluate HAVING clause against aggregation result.
    /// Supports both AND and OR logical operators between conditions.
    fn evaluate_having(having: &HavingClause, agg_result: &AggregateResult) -> bool {
        if having.conditions.is_empty() {
            return true;
        }

        // Evaluate first condition
        let mut result = {
            let cond = &having.conditions[0];
            let agg_value = Self::get_aggregate_value(&cond.aggregate, agg_result);
            Self::compare_values(agg_value, cond.operator, &cond.value)
        };

        // Apply remaining conditions with their operators
        for (i, cond) in having.conditions.iter().enumerate().skip(1) {
            let cond_result = {
                let agg_value = Self::get_aggregate_value(&cond.aggregate, agg_result);
                Self::compare_values(agg_value, cond.operator, &cond.value)
            };

            // Get operator (default to AND if not specified - backward compatible)
            let op = having
                .operators
                .get(i - 1)
                .copied()
                .unwrap_or(crate::velesql::LogicalOp::And);

            match op {
                crate::velesql::LogicalOp::And => result = result && cond_result,
                crate::velesql::LogicalOp::Or => result = result || cond_result,
            }
        }

        result
    }

    /// Get aggregate value from result based on function type.
    fn get_aggregate_value(agg: &AggregateFunction, result: &AggregateResult) -> Option<f64> {
        match (&agg.function_type, &agg.argument) {
            (AggregateType::Count, AggregateArg::Wildcard) => Some(result.count as f64),
            (AggregateType::Count, AggregateArg::Column(col)) => {
                // COUNT(column) = number of non-null values for this column
                result.counts.get(col.as_str()).map(|&c| c as f64)
            }
            (AggregateType::Sum, AggregateArg::Column(col)) => {
                result.sums.get(col.as_str()).copied()
            }
            (AggregateType::Avg, AggregateArg::Column(col)) => {
                result.avgs.get(col.as_str()).copied()
            }
            (AggregateType::Min, AggregateArg::Column(col)) => {
                result.mins.get(col.as_str()).copied()
            }
            (AggregateType::Max, AggregateArg::Column(col)) => {
                result.maxs.get(col.as_str()).copied()
            }
            _ => None,
        }
    }

    /// Compare aggregate value against threshold using operator.
    fn compare_values(agg_value: Option<f64>, op: CompareOp, threshold: &Value) -> bool {
        let agg = match agg_value {
            Some(v) => v,
            None => return false,
        };

        let thresh = match threshold {
            Value::Integer(i) => *i as f64,
            Value::Float(f) => *f,
            _ => return false,
        };

        // Use relative epsilon for large values (precision loss in sums)
        // Scale epsilon by max magnitude, with floor of 1.0 for small values
        let relative_epsilon = f64::EPSILON * agg.abs().max(thresh.abs()).max(1.0);

        match op {
            CompareOp::Eq => (agg - thresh).abs() < relative_epsilon,
            CompareOp::NotEq => (agg - thresh).abs() >= relative_epsilon,
            CompareOp::Gt => agg > thresh,
            CompareOp::Gte => agg >= thresh,
            CompareOp::Lt => agg < thresh,
            CompareOp::Lte => agg <= thresh,
        }
    }

    /// Extract max_groups limit from WITH clause (EPIC-040 US-004).
    /// Supports both `max_groups` and `group_limit` option names.
    /// Returns DEFAULT_MAX_GROUPS if not specified.
    fn extract_max_groups_limit(with_clause: Option<&crate::velesql::WithClause>) -> usize {
        let Some(with) = with_clause else {
            return DEFAULT_MAX_GROUPS;
        };

        for opt in &with.options {
            if opt.key == "max_groups" || opt.key == "group_limit" {
                // Try to parse value as integer
                if let crate::velesql::WithValue::Integer(n) = &opt.value {
                    // Ensure positive and reasonable limit
                    let limit = (*n).max(1) as usize;
                    return limit.min(1_000_000); // Hard cap at 1M groups
                }
            }
        }

        DEFAULT_MAX_GROUPS
    }

    /// BUG-5 FIX: Resolve parameter placeholders in a condition.
    /// Replaces Value::Parameter("name") with the actual value from params HashMap.
    fn resolve_condition_params(
        cond: &crate::velesql::Condition,
        params: &HashMap<String, serde_json::Value>,
    ) -> crate::velesql::Condition {
        use crate::velesql::Condition;

        match cond {
            Condition::Comparison(cmp) => {
                let resolved_value = Self::resolve_value(&cmp.value, params);
                Condition::Comparison(crate::velesql::Comparison {
                    column: cmp.column.clone(),
                    operator: cmp.operator,
                    value: resolved_value,
                })
            }
            Condition::In(in_cond) => {
                let resolved_values: Vec<Value> = in_cond
                    .values
                    .iter()
                    .map(|v| Self::resolve_value(v, params))
                    .collect();
                Condition::In(crate::velesql::InCondition {
                    column: in_cond.column.clone(),
                    values: resolved_values,
                })
            }
            Condition::Between(btw) => {
                let resolved_low = Self::resolve_value(&btw.low, params);
                let resolved_high = Self::resolve_value(&btw.high, params);
                Condition::Between(crate::velesql::BetweenCondition {
                    column: btw.column.clone(),
                    low: resolved_low,
                    high: resolved_high,
                })
            }
            Condition::And(left, right) => Condition::And(
                Box::new(Self::resolve_condition_params(left, params)),
                Box::new(Self::resolve_condition_params(right, params)),
            ),
            Condition::Or(left, right) => Condition::Or(
                Box::new(Self::resolve_condition_params(left, params)),
                Box::new(Self::resolve_condition_params(right, params)),
            ),
            Condition::Not(inner) => {
                Condition::Not(Box::new(Self::resolve_condition_params(inner, params)))
            }
            Condition::Group(inner) => {
                Condition::Group(Box::new(Self::resolve_condition_params(inner, params)))
            }
            // These conditions don't have Value parameters to resolve
            other => other.clone(),
        }
    }

    /// Resolve a single Value, substituting Parameter with actual value from params.
    fn resolve_value(value: &Value, params: &HashMap<String, serde_json::Value>) -> Value {
        match value {
            Value::Parameter(name) => {
                if let Some(param_value) = params.get(name) {
                    // Convert serde_json::Value to VelesQL Value
                    match param_value {
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                Value::Integer(i)
                            } else if let Some(f) = n.as_f64() {
                                Value::Float(f)
                            } else {
                                Value::Null
                            }
                        }
                        serde_json::Value::String(s) => Value::String(s.clone()),
                        serde_json::Value::Bool(b) => Value::Boolean(*b),
                        // Null, arrays, and objects not supported as params
                        _ => Value::Null,
                    }
                } else {
                    // Parameter not found, keep as null
                    Value::Null
                }
            }
            other => other.clone(),
        }
    }
}
