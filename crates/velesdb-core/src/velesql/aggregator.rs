//! Streaming aggregation for VelesQL (EPIC-017 US-002).
//!
//! Implements O(1) memory aggregation using single-pass streaming algorithm.
//! Based on state-of-art practices from DuckDB and DataFusion (arXiv 2024).

// Reason: Numeric casts in aggregation are intentional:
// - u64->f64 for count-to-double conversion: precision loss acceptable for averages
// - Count values are bounded by result set size
#![allow(clippy::cast_precision_loss)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use serde_json::Number;

use super::ast::{AggregateFunction, AggregateType};
use super::json_order::compare_numbers;

/// Result of aggregation operations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AggregateResult {
    /// COUNT(*) result.
    pub count: u64,
    /// COUNT(column) results by column name (non-null value counts).
    pub counts: HashMap<String, u64>,
    /// SUM results by column name.
    pub sums: HashMap<String, f64>,
    /// AVG results by column name (computed from sum/count).
    pub avgs: HashMap<String, f64>,
    /// MIN results by column name.
    pub mins: HashMap<String, f64>,
    /// MAX results by column name.
    pub maxs: HashMap<String, f64>,
    /// MIN results by column name, exactly: an integer stays an integer,
    /// beyond 2^53 included, where `mins` holds its `f64`.
    #[serde(default)]
    pub min_values: HashMap<String, Number>,
    /// MAX results by column name, exactly (see `min_values`).
    #[serde(default)]
    pub max_values: HashMap<String, Number>,
}

impl AggregateResult {
    /// The value `agg` computed, as an output row holds it: a count as an
    /// integer, a sum or mean as a float, a minimum or maximum as the value
    /// it is (an integer exactly), `null` where the group had no numeric
    /// value to aggregate. `FIRST` is not a running
    /// aggregate and reads `null` here; its value is the group's best row's
    /// (`velesql::grouping`). `score` names the column of that name, which a
    /// vector GROUP BY fills with each row's search score.
    #[must_use]
    pub(crate) fn value_of(&self, agg: &AggregateFunction) -> serde_json::Value {
        let column = agg.argument.column();
        let float = |values: &HashMap<String, f64>, column: &str| {
            values
                .get(column)
                .map_or(serde_json::Value::Null, |v| serde_json::json!(v))
        };
        match (agg.function_type, column) {
            (AggregateType::Count, None) => serde_json::json!(self.count),
            (AggregateType::Count, Some(column)) => {
                serde_json::json!(self.counts.get(column).copied().unwrap_or(0))
            }
            (AggregateType::Sum, Some(column)) => float(&self.sums, column),
            (AggregateType::Avg, Some(column)) => float(&self.avgs, column),
            (AggregateType::Min, Some(column)) => extreme(&self.min_values, &self.mins, column),
            (AggregateType::Max, Some(column)) => extreme(&self.max_values, &self.maxs, column),
            _ => serde_json::Value::Null,
        }
    }
}

/// A minimum or maximum as the value it is, from `exact`, else as the `f64`
/// `floats` holds (a result deserialized from before `exact` existed).
fn extreme(
    exact: &HashMap<String, Number>,
    floats: &HashMap<String, f64>,
    column: &str,
) -> serde_json::Value {
    match exact.get(column) {
        Some(n) => serde_json::Value::Number(n.clone()),
        None => floats
            .get(column)
            .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
    }
}

/// Running aggregate state for a single column.
///
/// Collocates sum, count, min, and max together so that the `Aggregator`
/// can maintain them with a single `HashMap` lookup per value instead of four
/// separate lookups across four maps. This also eliminates the cross-map
/// synchronisation invariant that previously required `debug_assert` guards.
///
/// The extremes are kept as the numbers they are, compared exactly
/// ([`compare_numbers`]): an integer past 2^53 stays itself, where an `f64`
/// would round it onto its neighbour.
#[derive(Debug, Clone)]
struct ColumnAgg {
    sum: f64,
    count: u64,
    min: Number,
    max: Number,
}

impl ColumnAgg {
    fn new(value: &Number) -> Self {
        Self {
            sum: value.as_f64().unwrap_or(0.0),
            count: 1,
            min: value.clone(),
            max: value.clone(),
        }
    }

    fn new_batch(sum: f64, count: u64, min: f64, max: f64) -> Self {
        Self {
            sum,
            count,
            min: float_number(min),
            max: float_number(max),
        }
    }

    #[inline]
    fn update(&mut self, value: &Number) {
        self.sum += value.as_f64().unwrap_or(0.0);
        self.count += 1;
        self.widen(value, value);
    }

    #[inline]
    fn update_batch(&mut self, batch_sum: f64, batch_count: u64, batch_min: f64, batch_max: f64) {
        self.sum += batch_sum;
        self.count += batch_count;
        self.widen(&float_number(batch_min), &float_number(batch_max));
    }

    #[inline]
    fn merge_from(&mut self, other: &Self) {
        self.sum += other.sum;
        self.count += other.count;
        self.widen(&other.min, &other.max);
    }

    /// Takes `min` and `max` as the extremes where they extend them.
    #[inline]
    fn widen(&mut self, min: &Number, max: &Number) {
        if compare_numbers(min, &self.min).is_lt() {
            self.min = min.clone();
        }
        if compare_numbers(max, &self.max).is_gt() {
            self.max = max.clone();
        }
    }
}

/// An `f64` extreme of a batch as a JSON number (`0` for a non-finite one,
/// which no JSON value holds).
fn float_number(value: f64) -> Number {
    Number::from_f64(value).unwrap_or_else(|| Number::from(0))
}

/// Streaming aggregator - O(1) memory, single-pass.
///
/// Based on online algorithms for computing aggregates without
/// storing all values in memory.
#[derive(Debug, Default)]
pub struct Aggregator {
    /// Running count for COUNT(*).
    count: u64,
    /// Per-column running aggregates over numeric values (sum, count, min,
    /// max in one entry).
    columns: HashMap<String, ColumnAgg>,
    /// Per-column count of non-null values of any type, for COUNT(column).
    present: HashMap<String, u64>,
}

impl Aggregator {
    /// Create a new aggregator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the row count (for COUNT(*)).
    pub fn process_count(&mut self) {
        self.count += 1;
    }

    /// Process a value for a specific column's aggregation.
    ///
    /// Updates SUM, MIN, MAX, and count for AVG calculation in a single
    /// HashMap lookup (fast path) or one allocation (slow path on first
    /// occurrence of the column).
    pub fn process_value(&mut self, column: &str, value: &serde_json::Value) {
        if !value.is_null() {
            Self::count_present(&mut self.present, column, 1);
        }
        if let serde_json::Value::Number(num) = value {
            match self.columns.get_mut(column) {
                Some(agg) => agg.update(num),
                None => {
                    self.columns.insert(column.to_string(), ColumnAgg::new(num));
                }
            }
        }
    }

    /// Adds `n` non-null values to `column`'s COUNT(column).
    fn count_present(present: &mut HashMap<String, u64>, column: &str, n: u64) {
        match present.get_mut(column) {
            Some(count) => *count += n,
            None => {
                present.insert(column.to_string(), n);
            }
        }
    }

    /// Process a batch of numeric values for SIMD-friendly aggregation.
    ///
    /// This method processes values in batches, allowing the compiler to
    /// auto-vectorize the loops using SIMD instructions for better performance.
    ///
    /// # Arguments
    /// * `column` - Column name for the aggregation
    /// * `values` - Slice of f64 values to aggregate
    pub fn process_batch(&mut self, column: &str, values: &[f64]) {
        if values.is_empty() {
            return;
        }

        // SIMD-friendly: compiler auto-vectorizes these loops
        let batch_sum: f64 = values.iter().sum();
        let batch_count = values.len() as u64;
        Self::count_present(&mut self.present, column, batch_count);
        let batch_min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let batch_max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);

        match self.columns.get_mut(column) {
            Some(agg) => agg.update_batch(batch_sum, batch_count, batch_min, batch_max),
            None => {
                self.columns.insert(
                    column.to_string(),
                    ColumnAgg::new_batch(batch_sum, batch_count, batch_min, batch_max),
                );
            }
        }
    }

    /// Merge another aggregator into this one (for parallel aggregation).
    ///
    /// Combines counts, sums, mins, maxs from the other aggregator.
    /// Used in map-reduce pattern for parallel processing.
    pub fn merge(&mut self, other: Self) {
        self.count += other.count;
        for (col, n) in other.present {
            Self::count_present(&mut self.present, &col, n);
        }
        for (col, other_agg) in other.columns {
            match self.columns.get_mut(&col) {
                Some(agg) => agg.merge_from(&other_agg),
                None => {
                    self.columns.insert(col, other_agg);
                }
            }
        }
    }

    /// Finalize aggregation and return results.
    #[must_use]
    pub fn finalize(self) -> AggregateResult {
        let cap = self.columns.len();
        let mut sums = HashMap::with_capacity(cap);
        let mut avgs = HashMap::with_capacity(cap);
        let mut mins = HashMap::with_capacity(cap);
        let mut maxs = HashMap::with_capacity(cap);
        let mut min_values = HashMap::with_capacity(cap);
        let mut max_values = HashMap::with_capacity(cap);

        for (col, agg) in self.columns {
            if agg.count > 0 {
                avgs.insert(col.clone(), agg.sum / agg.count as f64);
            }
            sums.insert(col.clone(), agg.sum);
            mins.insert(col.clone(), agg.min.as_f64().unwrap_or(0.0));
            maxs.insert(col.clone(), agg.max.as_f64().unwrap_or(0.0));
            min_values.insert(col.clone(), agg.min);
            max_values.insert(col, agg.max);
        }

        AggregateResult {
            count: self.count,
            counts: self.present,
            sums,
            avgs,
            mins,
            maxs,
            min_values,
            max_values,
        }
    }
}

// Tests moved to aggregator_tests.rs per project rules
