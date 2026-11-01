//! Collection statistics: analyze and cache collection stats.

use crate::{Error, Result};

use super::Database;

impl Database {
    /// Analyzes a collection, caches stats, and persists them to disk.
    ///
    /// ANALYZE walks the column store and returns what it computed — a
    /// collection read the gate decides in-core so every surface inherits it
    /// (the REST `POST /analyze`, the `ANALYZE` DDL statement, the Python and
    /// CLI facades): a denial refuses it and a scope, with no rows to narrow,
    /// refuses it too (`VELES-009`).
    ///
    /// # Errors
    ///
    /// Returns an error if the name is invalid, the collection does not exist,
    /// the read gate denies the read, analysis fails, or stats cannot be
    /// serialized and written to disk.
    pub fn analyze_collection(
        &self,
        name: &str,
    ) -> Result<crate::collection::stats::CollectionStats> {
        crate::validation::validate_collection_name(name)?;
        self.require_unscoped(
            name,
            crate::observer::QueryOperationKind::Select,
            "collection statistics",
        )?;

        let collection = self.resolve_collection(name)?;
        let stats = collection.analyze()?;

        // BFS reorder for cache locality: nodes close in the graph are placed
        // close in memory, reducing L2/L3 cache misses during search by 15–30%.
        // No-op for collections < 1 000 vectors (see REORDER_THRESHOLD).
        collection.reorder_for_locality()?;

        self.collection_stats
            .write()
            .insert(name.to_string(), stats.clone());

        // Bug #51: route the write through Collection so that stats_io_mutex
        // is held, preventing a race with incremental histogram updates.
        collection.write_stats_guarded(&stats)?;

        // Issue #608: bump analyze_generation after stats are persisted so
        // the compiled plan cache key changes and stale plans are rebuilt
        // with the fresh calibrated cost estimates.
        collection.bump_analyze_generation();

        Ok(stats)
    }

    /// Returns cached statistics when available, loading from disk if present.
    ///
    /// This method is deliberately **not** gated in-core: it is also the
    /// engine's own calibration source — `build_plan_with_stats` consults it
    /// on every uncached `execute_query`, so a consult per call would fire
    /// the observer on internal plan builds, not on caller reads. Every
    /// *surface* that exposes it to a caller must gate it instead: the REST
    /// `GET /stats` handler and the Python `get_collection_stats` binding do
    /// (`authorize_unscoped_read` / `deny_if_scoped`). New surfaces must do
    /// the same — this is the documented boundary, not an oversight.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is invalid, or the on-disk stats file
    /// exists but cannot be read or deserialized.
    pub fn get_collection_stats(
        &self,
        name: &str,
    ) -> Result<Option<crate::collection::stats::CollectionStats>> {
        crate::validation::validate_collection_name(name)?;

        // Bound before the `if let` so the guard does not span the disk read below.
        let cached = self.collection_stats.read().get(name).cloned();
        if let Some(stats) = cached {
            return Ok(Some(stats));
        }

        let stats_path = self.data_dir.join(name).join("collection.stats.json");
        if !stats_path.exists() {
            return Ok(None);
        }

        let bytes = std::fs::read(stats_path)?;
        let stats: crate::collection::stats::CollectionStats = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Serialization(format!("failed to parse stats: {e}")))?;
        self.collection_stats
            .write()
            .insert(name.to_string(), stats.clone());
        Ok(Some(stats))
    }
}
