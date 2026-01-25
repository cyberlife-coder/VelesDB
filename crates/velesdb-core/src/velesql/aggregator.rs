//! Streaming aggregation for VelesQL (EPIC-017 US-002).
//!
//! Implements O(1) memory aggregation using single-pass streaming algorithm.
//! Based on state-of-art practices from DuckDB and DataFusion (arXiv 2024).

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
    /// calls are guarded by invariant that all HashMaps are kept in sync.
    pub fn process_value(&mut self, column: &str, value: &serde_json::Value) {
        if let Some(num) = Self::extract_number(value) {
            // Fast path: column already tracked - no allocation
            if let Some(sum) = self.sums.get_mut(column) {
                *sum += num;
                // SAFETY: if sums has the key, counts/mins/maxs also have it
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregator_count() {
        let mut agg = Aggregator::new();
        agg.process_count();
        agg.process_count();
        agg.process_count();

        let result = agg.finalize();
        assert_eq!(result.count, 3);
    }

    #[test]
    fn test_aggregator_sum() {
        let mut agg = Aggregator::new();
        agg.process_value("price", &serde_json::json!(10));
        agg.process_value("price", &serde_json::json!(20));
        agg.process_value("price", &serde_json::json!(30));

        let result = agg.finalize();
        assert_eq!(result.sums.get("price"), Some(&60.0));
    }

    #[test]
    fn test_aggregator_avg() {
        let mut agg = Aggregator::new();
        agg.process_value("rating", &serde_json::json!(3));
        agg.process_value("rating", &serde_json::json!(4));
        agg.process_value("rating", &serde_json::json!(5));

        let result = agg.finalize();
        assert_eq!(result.avgs.get("rating"), Some(&4.0));
    }

    #[test]
    fn test_aggregator_min_max() {
        let mut agg = Aggregator::new();
        agg.process_value("val", &serde_json::json!(5));
        agg.process_value("val", &serde_json::json!(1));
        agg.process_value("val", &serde_json::json!(9));

        let result = agg.finalize();
        assert_eq!(result.mins.get("val"), Some(&1.0));
        assert_eq!(result.maxs.get("val"), Some(&9.0));
    }

    #[test]
    fn test_aggregator_multiple_columns() {
        let mut agg = Aggregator::new();
        agg.process_count();
        agg.process_value("a", &serde_json::json!(10));
        agg.process_value("b", &serde_json::json!(100));
        agg.process_count();
        agg.process_value("a", &serde_json::json!(20));
        agg.process_value("b", &serde_json::json!(200));

        let result = agg.finalize();
        assert_eq!(result.count, 2);
        assert_eq!(result.sums.get("a"), Some(&30.0));
        assert_eq!(result.sums.get("b"), Some(&300.0));
    }

    #[test]
    fn test_result_to_json() {
        let mut agg = Aggregator::new();
        agg.process_count();
        agg.process_value("price", &serde_json::json!(50));

        let result = agg.finalize();
        let json = result.to_json();

        assert_eq!(
            json.get("count").and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            json.get("sum_price").and_then(serde_json::Value::as_f64),
            Some(50.0)
        );
    }
}
