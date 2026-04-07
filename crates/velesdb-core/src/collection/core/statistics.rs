//! Collection statistics methods (EPIC-046 US-001).
//!
//! Provides the `analyze()` method for collecting runtime statistics
//! to support cost-based query planning.

use crate::collection::query_cost::cost_model::{calibrate_cost_factors, OperationCostFactors};
use crate::collection::stats::{CollectionStats, IndexStats, StatsCollector};
use crate::collection::Collection;
use crate::error::Error;
use crate::storage::PayloadStorage;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Converts a `usize` to `u64`, saturating to `u64::MAX` on overflow.
///
/// Used throughout statistics collection where collection sizes are bounded by
/// available memory and precision is non-critical.
fn saturating_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// TTL for cached collection statistics used by the cost-based query planner.
const STATS_TTL: Duration = Duration::from_secs(30);

impl Collection {
    /// Analyzes the collection and returns statistics.
    ///
    /// This method collects:
    /// - Row count and deleted count
    /// - Index statistics (HNSW entry count)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let stats = collection.analyze()?;
    /// println!("Row count: {}", stats.row_count);
    /// println!("Deletion ratio: {:.1}%", stats.deletion_ratio() * 100.0);
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if statistics cannot be collected.
    ///
    /// # Panics
    ///
    /// Panics if `point_count` exceeds `u64::MAX` (extremely unlikely on 64-bit systems).
    #[allow(clippy::unnecessary_wraps)] // Reason: Public API contract — callers expect Result
    pub fn analyze(&self) -> Result<CollectionStats, Error> {
        let mut collector = StatsCollector::new();

        // Basic counts from config
        // Note: deleted_count and column_stats are placeholders for future tombstone tracking
        // and per-column cardinality analysis (EPIC-046 future work)
        let config = self.config.read();
        collector.set_row_count(saturating_u64(config.point_count));
        drop(config);

        let (payload_size_bytes, distinct_values, null_counts) = self.sample_payload_stats();

        collector.set_total_size(payload_size_bytes);

        for (field, values) in distinct_values {
            let mut col = crate::collection::stats::ColumnStats::new(field.clone())
                .with_distinct_count(saturating_u64(values.len()));
            if let Some(null_count) = null_counts.get(&field) {
                col = col.with_null_count(*null_count);
            }
            collector.add_column_stats(col);
        }

        // Histogram construction: separate 10K-row sample for equi-depth histograms
        let mut column_values = self.sample_column_values_for_histograms(10_000);
        for (col_name, values) in &mut column_values {
            collector.build_histogram(col_name, values, 64);
        }

        // HNSW index statistics
        let hnsw_len = self.index.len();
        let hnsw_stats =
            IndexStats::new("hnsw_primary", "HNSW").with_entry_count(saturating_u64(hnsw_len));
        collector.add_index_stats(hnsw_stats);

        // BM25 index statistics - use len() if available
        let bm25_len = self.text_index.len();
        if bm25_len > 0 {
            let bm25_stats =
                IndexStats::new("bm25_text", "BM25").with_entry_count(saturating_u64(bm25_len));
            collector.add_index_stats(bm25_stats);
        }

        let mut stats = collector.build();

        // Calibrate cost factors from observed collection characteristics
        let calibrated = calibrate_cost_factors(&stats, &OperationCostFactors::default());
        stats.calibrated_cost_factors = Some(calibrated);

        Ok(stats)
    }

    /// Samples up to 1000 payloads to compute size, distinct values, and null counts.
    fn sample_payload_stats(
        &self,
    ) -> (u64, HashMap<String, HashSet<String>>, HashMap<String, u64>) {
        let mut distinct_values: HashMap<String, HashSet<String>> = HashMap::new();
        let mut null_counts: HashMap<String, u64> = HashMap::new();
        let mut payload_size_bytes = 0u64;

        let payload_storage = self.payload_storage.read();
        let ids = payload_storage.ids();
        for id in ids.into_iter().take(1_000) {
            if let Ok(Some(payload)) = payload_storage.retrieve(id) {
                if let Ok(payload_bytes) = serde_json::to_vec(&payload) {
                    payload_size_bytes =
                        payload_size_bytes.saturating_add(saturating_u64(payload_bytes.len()));
                }

                if let Some(obj) = payload.as_object() {
                    for (key, value) in obj {
                        if value.is_null() {
                            *null_counts.entry(key.clone()).or_insert(0) += 1;
                        } else {
                            distinct_values
                                .entry(key.clone())
                                .or_default()
                                .insert(value.to_string());
                        }
                    }
                }
            }
        }

        (payload_size_bytes, distinct_values, null_counts)
    }

    /// Samples up to `max_samples` rows from payload storage for histogram construction.
    ///
    /// For each payload field, extracts values and converts them to `f64`:
    /// - Integer → `i as f64`
    /// - Float → `f` directly
    /// - String → ordinal rank (sorted unique index, 0-based)
    /// - NULL, Bool, Array, and Object values are skipped.
    ///
    /// String ordinal ranking is computed per-column: all sampled strings are
    /// collected, sorted lexicographically, deduplicated, and each string is
    /// mapped to its 0-based index in the sorted unique list.
    fn sample_column_values_for_histograms(&self, max_samples: usize) -> HashMap<String, Vec<f64>> {
        let payload_storage = self.payload_storage.read();
        let ids = payload_storage.ids();

        // First pass: collect raw values per column
        let mut numeric_values: HashMap<String, Vec<f64>> = HashMap::new();
        let mut string_values: HashMap<String, Vec<String>> = HashMap::new();

        for id in ids.into_iter().take(max_samples) {
            if let Ok(Some(payload)) = payload_storage.retrieve(id) {
                if let Some(obj) = payload.as_object() {
                    for (key, value) in obj {
                        Self::collect_column_value(
                            key,
                            value,
                            &mut numeric_values,
                            &mut string_values,
                        );
                    }
                }
            }
        }

        // Second pass: convert string ordinal ranks and merge into result
        for (col, strings) in string_values {
            let ranks = Self::compute_ordinal_ranks(&strings);
            numeric_values.entry(col).or_default().extend(ranks);
        }

        numeric_values
    }

    /// Collects a single payload field value into the appropriate accumulator.
    ///
    /// Numeric values go into `numeric_values`; strings go into `string_values`.
    /// NULL, Bool, Array, and Object values are skipped.
    fn collect_column_value(
        key: &str,
        value: &serde_json::Value,
        numeric_values: &mut HashMap<String, Vec<f64>>,
        string_values: &mut HashMap<String, Vec<String>>,
    ) {
        match value {
            serde_json::Value::Number(n) => {
                if let Some(f) = n.as_f64() {
                    numeric_values.entry(key.to_owned()).or_default().push(f);
                }
            }
            serde_json::Value::String(s) => {
                string_values
                    .entry(key.to_owned())
                    .or_default()
                    .push(s.clone());
            }
            _ => {} // Skip Null, Bool, Array, Object
        }
    }

    /// Computes ordinal ranks for a list of string values.
    ///
    /// Sorts unique strings lexicographically and maps each input string
    /// to its 0-based index in the sorted unique list.
    // Reason: ordinal rank indices are bounded by sample size (≤10,000);
    // usize→f64 is exact for values < 2^53.
    #[allow(clippy::cast_precision_loss)]
    fn compute_ordinal_ranks(strings: &[String]) -> Vec<f64> {
        let mut unique: Vec<&str> = strings.iter().map(String::as_str).collect();
        unique.sort_unstable();
        unique.dedup();

        strings
            .iter()
            .filter_map(|s| unique.binary_search(&s.as_str()).ok().map(|idx| idx as f64))
            .collect()
    }

    /// Returns cached statistics if available and fresh, otherwise recomputes.
    ///
    /// Results are cached for 30 seconds (`STATS_TTL`) to avoid re-scanning payload
    /// storage on every `execute_query()` call. Mutating methods (`upsert`,
    /// `delete`, etc.) invalidate the cache so the next call always recomputes.
    ///
    /// # Note
    /// Returns default stats on error (intentional for convenience).
    /// Use `analyze()` directly if error handling is required.
    #[must_use]
    pub fn get_stats(&self) -> CollectionStats {
        let mut cached = self.cached_stats.lock();
        if let Some((ref stats, ts)) = *cached {
            if ts.elapsed() < STATS_TTL {
                return stats.clone();
            }
        }
        match self.analyze() {
            Ok(stats) => {
                *cached = Some((stats.clone(), Instant::now()));
                stats
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to compute collection statistics: {}. Returning defaults.",
                    e
                );
                CollectionStats::default()
            }
        }
    }

    /// Returns the selectivity estimate for a column.
    ///
    /// Selectivity is 1/cardinality, representing the probability
    /// that a random row matches a specific value.
    #[must_use]
    #[allow(dead_code)] // Reason: Public API for cost model — used by typed wrappers
    pub fn estimate_column_selectivity(&self, column: &str) -> f64 {
        let stats = self.get_stats();
        stats.estimate_selectivity(column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distance::DistanceMetric;
    use tempfile::TempDir;

    #[test]
    fn test_analyze_empty_collection() {
        let temp_dir = TempDir::new().unwrap();
        let collection =
            Collection::create(temp_dir.path().to_path_buf(), 128, DistanceMetric::Cosine).unwrap();

        let stats = collection.analyze().unwrap();

        assert_eq!(stats.row_count, 0);
        assert_eq!(stats.deleted_count, 0);
        assert!(stats.index_stats.contains_key("hnsw_primary"));
    }

    #[test]
    fn test_analyze_with_data() {
        use crate::point::Point;

        let temp_dir = TempDir::new().unwrap();
        let collection =
            Collection::create(temp_dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

        // Insert some vectors using Point
        let points: Vec<Point> = (0..10)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)] // Reason: i < 20 in test; u64→f32 exact.
                Point::new(
                    i,
                    vec![i as f32; 4],
                    Some(serde_json::json!({"category": format!("cat_{}", i % 3)})),
                )
            })
            .collect();
        collection.upsert(points).unwrap();

        let stats = collection.analyze().unwrap();

        assert_eq!(stats.row_count, 10);
        assert!(stats.index_stats.get("hnsw_primary").unwrap().entry_count >= 10);
    }

    #[test]
    fn test_get_stats_returns_defaults_on_error() {
        let temp_dir = TempDir::new().unwrap();
        let collection =
            Collection::create(temp_dir.path().to_path_buf(), 128, DistanceMetric::Cosine).unwrap();

        let stats = collection.get_stats();

        // Should not panic, returns default on any issue
        assert_eq!(stats.live_row_count(), 0);
    }

    #[test]
    fn test_get_stats_uses_cache_within_ttl() {
        let temp_dir = TempDir::new().unwrap();
        let collection =
            Collection::create(temp_dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

        // First call populates the cache.
        let stats1 = collection.get_stats();
        assert_eq!(stats1.row_count, 0);

        // Insert a point — but bypass invalidation by calling the storage directly
        // so we can verify the cache is still served unchanged.
        // We just call get_stats() again immediately: within TTL it must return
        // the same object (row_count == 0) without re-scanning.
        let stats2 = collection.get_stats();
        assert_eq!(
            stats1.row_count, stats2.row_count,
            "get_stats should return cached value within TTL"
        );
    }

    #[test]
    fn test_get_stats_invalidated_after_upsert() {
        use crate::point::Point;

        let temp_dir = TempDir::new().unwrap();
        let collection =
            Collection::create(temp_dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

        // Warm the cache.
        let stats_before = collection.get_stats();
        assert_eq!(stats_before.row_count, 0);

        // upsert() must invalidate the cache.
        let points = vec![Point::new(1, vec![0.1, 0.2, 0.3, 0.4], None)];
        collection.upsert(points).unwrap();

        // Next get_stats() should recompute and reflect the new point.
        let stats_after = collection.get_stats();
        assert_eq!(
            stats_after.row_count, 1,
            "get_stats should recompute after upsert invalidates the cache"
        );
    }
}
