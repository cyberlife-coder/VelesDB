//! Collection statistics: analyze and cache collection stats.

use crate::{Error, Result};

use super::Database;

impl Database {
    /// Analyzes a collection, caches stats, and persists them to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is invalid, the collection does not exist,
    /// analysis fails, or stats cannot be serialized and written to disk.
    pub fn analyze_collection(
        &self,
        name: &str,
    ) -> Result<crate::collection::stats::CollectionStats> {
        crate::validation::validate_collection_name(name)?;

        let collection = self.resolve_collection(name)?;
        let stats = collection.analyze()?;

        self.collection_stats
            .write()
            .insert(name.to_string(), stats.clone());

        // Bug #51: route the write through Collection so that stats_io_mutex
        // is held, preventing a race with incremental histogram updates.
        collection.write_stats_guarded(&stats)?;

        Ok(stats)
    }

    /// Returns cached statistics when available, loading from disk if present.
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

        if let Some(stats) = self.collection_stats.read().get(name).cloned() {
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
