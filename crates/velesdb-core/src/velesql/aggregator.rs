//! Streaming aggregation for `VelesQL` (EPIC-017 US-002).
//!
//! Implements O(1) memory aggregation using single-pass streaming algorithm.
//! Based on state-of-art practices from `DuckDB` and `DataFusion` (arXiv 2024).

// SAFETY: Numeric casts in aggregation are intentional:
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

/// Streaming aggregator - O(1) memory, single-pass.
///
/// Based on online algorithms for computing aggregates without
/// storing all values in memory.
#[derive(Debug, Default)]
pub struct Aggregator {
    /// Running count for COUNT(*).
    count: u64,
    /// Running sums by column.
    sums: HashMap<String, f64>,
    /// Running counts by column (for AVG calculation).
    counts: HashMap<String, u64>,
    /// Running minimums by column.
    mins: HashMap<String, f64>,
    /// Running maximums by column.
    maxs: HashMap<String, f64>,
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
    /// Updates SUM, MIN, MAX, and count for AVG calculation.
    /// Optimized to avoid String allocation in hot path when column already exists.
    ///
    /// # Panics
    ///
    /// This function will not panic under normal operation. The internal `expect()`
    /// calls are guarded by invariant that all `HashMaps` are kept in sync.
    pub fn process_value(&mut self, column: &str, value: &serde_json::Value) {
        if let Some(num) = Self::extract_number(value) {
            // Fast path: column already tracked - no allocation
            if let Some(sum) = self.sums.get_mut(column) {
                *sum += num;
                // Reason: All 4 HashMaps (sums, counts, mins, maxs) are always inserted
                // together in the slow path below — missing key here is a logic bug.
                *self
                    .counts
                    .get_mut(column)
                    .expect("counts synced with sums") += 1;
                let min = self.mins.get_mut(column).expect("mins synced with sums");
                if num < *min {
                    *min = num;
                }
                let max = self.maxs.get_mut(column).expect("maxs synced with sums");
                if num > *max {
                    *max = num;
                }
                return;
            }

            // Slow path: first time seeing this column - allocate once
            let col_owned = column.to_string();
            self.sums.insert(col_owned.clone(), num);
            self.counts.insert(col_owned.clone(), 1);
            self.mins.insert(col_owned.clone(), num);
            self.maxs.insert(col_owned, num);
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
    ///
    /// # Panics
    ///
    /// This function will not panic under normal operation. The internal `expect()`
    /// calls are guarded by invariant that all `HashMaps` are kept in sync.
    pub fn process_batch(&mut self, column: &str, values: &[f64]) {
        if values.is_empty() {
            return;
        }

        // SIMD-friendly: compiler auto-vectorizes these loops
        let batch_sum: f64 = values.iter().sum();
        let batch_count = values.len() as u64;
        let batch_min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let batch_max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);

        // Fast path: column already tracked
        if let Some(sum) = self.sums.get_mut(column) {
            *sum += batch_sum;
            // Reason: All 4 HashMaps (sums, counts, mins, maxs) are always inserted
            // together in the slow path below — missing key here is a logic bug.
            *self
                .counts
                .get_mut(column)
                .expect("counts synced with sums") += batch_count;
            let min = self.mins.get_mut(column).expect("mins synced with sums");
            if batch_min < *min {
                *min = batch_min;
            }
            let max = self.maxs.get_mut(column).expect("maxs synced with sums");
            if batch_max > *max {
                *max = batch_max;
            }
            return;
        }

        // Slow path: first time seeing this column
        let col_owned = column.to_string();
        self.sums.insert(col_owned.clone(), batch_sum);
        self.counts.insert(col_owned.clone(), batch_count);
        self.mins.insert(col_owned.clone(), batch_min);
        self.maxs.insert(col_owned, batch_max);
    }

    /// Merge another aggregator into this one (for parallel aggregation).
    ///
    /// Combines counts, sums, mins, maxs from the other aggregator.
    /// Used in map-reduce pattern for parallel processing.
    pub fn merge(&mut self, other: Self) {
        // Merge COUNT(*)
        self.count += other.count;

        // Merge sums
        for (col, sum) in other.sums {
            *self.sums.entry(col).or_insert(0.0) += sum;
        }

        // Merge counts (for AVG calculation)
        for (col, count) in other.counts {
            *self.counts.entry(col).or_insert(0) += count;
        }

        // Merge mins (take minimum of both)
        for (col, min) in other.mins {
            let current = self.mins.entry(col).or_insert(min);
            if min < *current {
                *current = min;
            }
        }

        // Merge maxs (take maximum of both)
        for (col, max) in other.maxs {
            let current = self.maxs.entry(col).or_insert(max);
            if max > *current {
                *current = max;
            }
        }
    }

    /// Finalize aggregation and return results.
    #[must_use]
    pub fn finalize(self) -> AggregateResult {
        // Calculate averages from sums and counts
        let avgs: HashMap<String, f64> = self
            .sums
            .iter()
            .filter_map(|(col, sum)| {
                self.counts
                    .get(col)
                    .filter(|&&c| c > 0)
                    .map(|&c| (col.clone(), sum / c as f64))
            })
            .collect();

        AggregateResult {
            count: self.count,
            counts: self.counts,
            sums: self.sums,
            avgs,
            mins: self.mins,
            maxs: self.maxs,
        }
    }
}

// Tests moved to aggregator_tests.rs per project rules
