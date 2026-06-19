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
}

impl AggregateResult {
    /// Convert to JSON Value for query result.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();

        if self.count > 0 || self.sums.is_empty() {
            map.insert("count".to_string(), serde_json::json!(self.count));
        }

        for (col, sum) in &self.sums {
            map.insert(format!("sum_{col}"), serde_json::json!(sum));
        }

        for (col, avg) in &self.avgs {
            map.insert(format!("avg_{col}"), serde_json::json!(avg));
        }

        for (col, min) in &self.mins {
            map.insert(format!("min_{col}"), serde_json::json!(min));
        }

        for (col, max) in &self.maxs {
            map.insert(format!("max_{col}"), serde_json::json!(max));
        }

        serde_json::Value::Object(map)
    }
}

/// Running aggregate state for a single column.
///
/// Collocates sum, count, min, and max together so that the `Aggregator`
/// can maintain them with a single `HashMap` lookup per value instead of four
/// separate lookups across four maps. This also eliminates the cross-map
/// synchronisation invariant that previously required `debug_assert` guards.
#[derive(Debug, Clone)]
struct ColumnAgg {
    sum: f64,
    count: u64,
    min: f64,
    max: f64,
}

impl ColumnAgg {
    fn new(value: f64) -> Self {
        Self {
            sum: value,
            count: 1,
            min: value,
            max: value,
        }
    }

    fn new_batch(sum: f64, count: u64, min: f64, max: f64) -> Self {
        Self { sum, count, min, max }
    }

    #[inline]
    fn update(&mut self, value: f64) {
        self.sum += value;
        self.count += 1;
        if value < self.min {
            self.min = value;
        }
        if value > self.max {
            self.max = value;
        }
    }

    #[inline]
    fn update_batch(&mut self, batch_sum: f64, batch_count: u64, batch_min: f64, batch_max: f64) {
        self.sum += batch_sum;
        self.count += batch_count;
        if batch_min < self.min {
            self.min = batch_min;
        }
        if batch_max > self.max {
            self.max = batch_max;
        }
    }

    #[inline]
    fn merge_from(&mut self, other: &Self) {
        self.sum += other.sum;
        self.count += other.count;
        if other.min < self.min {
            self.min = other.min;
        }
        if other.max > self.max {
            self.max = other.max;
        }
    }
}

/// Streaming aggregator - O(1) memory, single-pass.
///
/// Based on online algorithms for computing aggregates without
/// storing all values in memory.
#[derive(Debug, Default)]
pub struct Aggregator {
    /// Running count for COUNT(*).
    count: u64,
    /// Per-column running aggregates (sum, count, min, max in one entry).
    columns: HashMap<String, ColumnAgg>,
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
        if let Some(num) = Self::extract_number(value) {
            match self.columns.get_mut(column) {
                Some(agg) => agg.update(num),
                None => {
                    self.columns.insert(column.to_string(), ColumnAgg::new(num));
                }
            }
        }
    }

    /// Extract a numeric value from JSON.
    fn extract_number(value: &serde_json::Value) -> Option<f64> {
        match value {
            serde_json::Value::Number(n) => n.as_f64(),
            _ => None,
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
        let mut counts = HashMap::with_capacity(cap);
        let mut avgs = HashMap::with_capacity(cap);
        let mut mins = HashMap::with_capacity(cap);
        let mut maxs = HashMap::with_capacity(cap);

        for (col, agg) in self.columns {
            if agg.count > 0 {
                avgs.insert(col.clone(), agg.sum / agg.count as f64);
            }
            sums.insert(col.clone(), agg.sum);
            counts.insert(col.clone(), agg.count);
            mins.insert(col.clone(), agg.min);
            maxs.insert(col, agg.max);
        }

        AggregateResult {
            count: self.count,
            counts,
            sums,
            avgs,
            mins,
            maxs,
        }
    }
}

// Tests moved to aggregator_tests.rs per project rules
