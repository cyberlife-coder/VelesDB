//! Incremental histogram maintenance for upsert and delete paths.
//!
//! Updates persisted histogram bucket counts on each mutation so that
//! selectivity estimates remain approximately accurate between ANALYZE runs.
//! Histogram data lives in `collection.stats.json`, not in the volatile
//! `cached_stats` cache, so it survives cache invalidation.

use crate::collection::stats::CollectionStats;
use crate::collection::types::Collection;

impl Collection {
    /// Incrementally updates persisted histograms for upserted payloads.
    ///
    /// For each column that has a histogram in the persisted stats, converts
    /// the payload value to `f64` and calls `increment_bucket`. Reads and
    /// writes `collection.stats.json` only when the file exists.
    ///
    /// Called BEFORE `invalidate_caches_and_bump_generation()` in the upsert path.
    pub(super) fn update_histograms_on_upsert(&self, payloads: &[Option<serde_json::Value>]) {
        self.update_histograms_for_payloads(payloads, true);
    }

    /// Incrementally updates persisted histograms for deleted payloads.
    ///
    /// For each column that has a histogram in the persisted stats, converts
    /// the payload value to `f64` and calls `decrement_bucket` (floored at zero).
    /// Reads and writes `collection.stats.json` only when the file exists.
    ///
    /// Called BEFORE `invalidate_caches_and_bump_generation()` in the delete path.
    pub(super) fn update_histograms_on_delete(&self, payloads: &[Option<serde_json::Value>]) {
        self.update_histograms_for_payloads(payloads, false);
    }

    /// Decrements old payload histograms and increments new ones in a single
    /// read → modify → write cycle. Used by bulk upsert paths where points
    /// replace existing data (old values must be decremented, new values
    /// incremented). Avoids the 2× I/O of calling delete + upsert separately.
    pub(super) fn update_histograms_replace(
        &self,
        old_payloads: &[Option<serde_json::Value>],
        new_payloads: &[Option<serde_json::Value>],
    ) {
        let stats_path = self.path.join("collection.stats.json");
        let _guard = self.stats_io_mutex.lock();

        let Some(mut stats) = Self::read_persisted_stats(&stats_path) else {
            return;
        };
        if !Self::has_any_histogram(&stats) {
            return;
        }

        let mut modified = Self::apply_histogram_updates(&mut stats, old_payloads, false);
        modified |= Self::apply_histogram_updates(&mut stats, new_payloads, true);

        if modified {
            Self::write_persisted_stats(&stats_path, &stats);
        }
    }

    /// Core histogram update logic shared by upsert and delete paths.
    ///
    /// Acquires `stats_io_mutex` to serialise the read → modify → write cycle,
    /// preventing two concurrent upserts from overwriting each other's changes.
    /// No-op when the stats file does not exist or contains no histograms.
    fn update_histograms_for_payloads(
        &self,
        payloads: &[Option<serde_json::Value>],
        increment: bool,
    ) {
        let stats_path = self.path.join("collection.stats.json");

        // LOCK ORDER: stats_io_mutex (12) — no other lock held.
        let _guard = self.stats_io_mutex.lock();

        let Some(mut stats) = Self::read_persisted_stats(&stats_path) else {
            return;
        };

        if !Self::has_any_histogram(&stats) {
            return;
        }

        let modified = Self::apply_histogram_updates(&mut stats, payloads, increment);

        if modified {
            Self::write_persisted_stats(&stats_path, &stats);
        }
    }

    /// Reads persisted `CollectionStats` from disk. Returns `None` on any error.
    fn read_persisted_stats(stats_path: &std::path::Path) -> Option<CollectionStats> {
        let bytes = std::fs::read(stats_path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Returns `true` if any column in the stats has a non-empty histogram.
    fn has_any_histogram(stats: &CollectionStats) -> bool {
        let check = |cs: &crate::collection::stats::ColumnStats| {
            cs.histogram.as_ref().is_some_and(|h| !h.buckets.is_empty())
        };
        stats.field_stats.values().any(check) || stats.column_stats.values().any(check)
    }

    /// Applies histogram updates for all payloads. Returns `true` if any histogram was modified.
    fn apply_histogram_updates(
        stats: &mut CollectionStats,
        payloads: &[Option<serde_json::Value>],
        increment: bool,
    ) -> bool {
        let mut modified = false;
        for payload in payloads.iter().filter_map(|p| p.as_ref()) {
            if let Some(obj) = payload.as_object() {
                for (col, value) in obj {
                    if let Some(v) = payload_value_to_f64(value) {
                        modified |= Self::update_column_histogram(stats, col, v, increment);
                    }
                }
            }
        }
        modified
    }

    /// Updates the histogram for a single column in both `field_stats` and `column_stats`.
    ///
    /// Returns `true` if any histogram was updated.
    fn update_column_histogram(
        stats: &mut CollectionStats,
        column: &str,
        value: f64,
        increment: bool,
    ) -> bool {
        let mut updated = false;
        updated |=
            Self::update_single_histogram_map(&mut stats.field_stats, column, value, increment);
        updated |=
            Self::update_single_histogram_map(&mut stats.column_stats, column, value, increment);
        updated
    }

    /// Updates the histogram in a single stats map entry, logging staleness.
    fn update_single_histogram_map(
        map: &mut std::collections::HashMap<String, crate::collection::stats::ColumnStats>,
        column: &str,
        value: f64,
        increment: bool,
    ) -> bool {
        let Some(col_stats) = map.get_mut(column) else {
            return false;
        };
        let histogram = match col_stats.histogram.as_mut() {
            Some(h) if !h.buckets.is_empty() => h,
            _ => return false,
        };

        let was_stale = histogram.stale;
        if increment {
            histogram.increment_bucket(value);
        } else {
            histogram.decrement_bucket(value);
        }

        if histogram.stale && !was_stale {
            tracing::debug!(
                "Histogram for column '{}' is stale; consider running ANALYZE",
                column
            );
        }
        true
    }

    /// Writes full `CollectionStats` to disk under the `stats_io_mutex`.
    ///
    /// Called by `Database::analyze_collection` to ensure the write does not
    /// race with incremental histogram updates (Bug #51). Returns `Ok(())`
    /// on success, or an I/O / serialisation error.
    pub fn write_stats_guarded(&self, stats: &CollectionStats) -> crate::error::Result<()> {
        let stats_path = self.path.join("collection.stats.json");
        let _guard = self.stats_io_mutex.lock();

        let serialized = serde_json::to_vec_pretty(stats).map_err(|e| {
            crate::error::Error::Serialization(format!("failed to serialize stats: {e}"))
        })?;

        let tmp_path = stats_path.with_file_name(".stats.json.tmp");
        std::fs::write(&tmp_path, serialized)?;

        if let Err(e) = std::fs::rename(&tmp_path, &stats_path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e.into());
        }

        Ok(())
    }

    /// Writes `CollectionStats` back to disk atomically.
    ///
    /// Serialises to a temporary file in the same directory, then renames over
    /// the target path. On most file-systems `rename` is atomic, so a crash
    /// mid-write cannot leave a corrupted JSON file. If the rename fails the
    /// temporary file is cleaned up on a best-effort basis.
    fn write_persisted_stats(stats_path: &std::path::Path, stats: &CollectionStats) {
        let Ok(serialized) = serde_json::to_vec_pretty(stats) else {
            tracing::warn!("Failed to serialize stats for histogram update");
            return;
        };

        let tmp_path = stats_path.with_file_name(".stats.json.tmp");

        if let Err(e) = std::fs::write(&tmp_path, serialized) {
            tracing::warn!("Failed to write temp stats file: {e}");
            return;
        }

        if let Err(e) = std::fs::rename(&tmp_path, stats_path) {
            tracing::warn!("Failed to rename temp stats file: {e}");
            // Best-effort cleanup of the orphaned temp file.
            let _ = std::fs::remove_file(&tmp_path);
        }
    }
}

/// Converts a JSON payload value to `f64` for histogram bucket lookup.
///
/// Maps integers and floats directly. Skips strings (ordinal rank not available
/// without the full string mapping), nulls, booleans, arrays, and objects.
fn payload_value_to_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(n) => n.as_f64(),
        _ => None,
    }
}
