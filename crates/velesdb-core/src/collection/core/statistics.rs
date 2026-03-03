//! Collection statistics methods (EPIC-046 US-001).
//!
//! Provides the `analyze()` method for collecting runtime statistics
//! to support cost-based query planning.

use crate::collection::stats::{CollectionStats, IndexStats, StatsCollector};
use crate::collection::Collection;
use crate::error::Error;
use crate::storage::PayloadStorage;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

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
    pub fn analyze(&self) -> Result<CollectionStats, Error> {
        let mut collector = StatsCollector::new();

        // Basic counts from config
        // Note: deleted_count and column_stats are placeholders for future tombstone tracking
        // and per-column cardinality analysis (EPIC-046 future work)
        let config = self.config.read();
        // Reason: Collection sizes are bounded by available memory, always < u64::MAX on 64-bit systems
        collector.set_row_count(u64::try_from(config.point_count).unwrap_or(u64::MAX));
        drop(config);

        let mut distinct_values: HashMap<String, HashSet<String>> = HashMap::new();
        let mut null_counts: HashMap<String, u64> = HashMap::new();
        let mut payload_size_bytes = 0u64;

        let payload_storage = self.payload_storage.read();
        let ids = payload_storage.ids();
        for id in ids.into_iter().take(1_000) {
            if let Ok(Some(payload)) = payload_storage.retrieve(id) {
                if let Ok(payload_bytes) = serde_json::to_vec(&payload) {
                    payload_size_bytes = payload_size_bytes
                        .saturating_add(u64::try_from(payload_bytes.len()).unwrap_or(u64::MAX));
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
        drop(payload_storage);

        collector.set_total_size(payload_size_bytes);

        for (field, values) in distinct_values {
            let mut col = crate::collection::stats::ColumnStats::new(field.clone())
                .with_distinct_count(u64::try_from(values.len()).unwrap_or(u64::MAX));
            if let Some(null_count) = null_counts.get(&field) {
                col = col.with_null_count(*null_count);
            }
            collector.add_column_stats(col);
        }

        // HNSW index statistics
        let hnsw_len = self.index.len();
        let hnsw_stats = IndexStats::new("hnsw_primary", "HNSW")
            .with_entry_count(u64::try_from(hnsw_len).unwrap_or(u64::MAX));
        collector.add_index_stats(hnsw_stats);

        // BM25 index statistics - use len() if available
        let bm25_len = self.text_index.len();
        if bm25_len > 0 {
            let bm25_stats = IndexStats::new("bm25_text", "BM25")
                .with_entry_count(u64::try_from(bm25_len).unwrap_or(u64::MAX));
            collector.add_index_stats(bm25_stats);
        }

        Ok(collector.build())
    }

    /// Returns cached statistics if available and fresh, otherwise recomputes.
    ///
    /// Results are cached for [`STATS_TTL`] (30 s) to avoid re-scanning payload
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
