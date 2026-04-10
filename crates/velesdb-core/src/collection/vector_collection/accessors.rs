//! Read-only accessors for `VectorCollection` metadata and state.

use crate::collection::types::CollectionConfig;
use crate::distance::DistanceMetric;
use crate::quantization::StorageMode;

use super::VectorCollection;

impl VectorCollection {
    /// Returns a reference to the collection's guard rails.
    #[must_use]
    pub fn guard_rails(&self) -> &std::sync::Arc<crate::guardrails::GuardRails> {
        self.inner.guard_rails()
    }

    /// Returns the collection name.
    #[must_use]
    pub fn name(&self) -> String {
        self.inner.config().name
    }

    /// Returns the vector dimension.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.inner.config().dimension
    }

    /// Returns the distance metric.
    #[must_use]
    pub fn metric(&self) -> DistanceMetric {
        self.inner.config().metric
    }

    /// Returns the storage mode.
    #[must_use]
    pub fn storage_mode(&self) -> StorageMode {
        self.inner.config().storage_mode
    }

    /// Returns the number of points in the collection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns all point IDs.
    #[must_use]
    pub fn all_ids(&self) -> Vec<u64> {
        self.inner.all_ids()
    }

    /// Returns the next batch of points for scroll iteration.
    ///
    /// Delegates to [`Collection::scroll_batch`].
    ///
    /// # Errors
    ///
    /// Returns an error if `batch_size` is 0.
    pub fn scroll_batch(
        &self,
        cursor: Option<u64>,
        batch_size: usize,
        filter: Option<&crate::filter::Filter>,
    ) -> crate::error::Result<crate::collection::ScrollBatch> {
        self.inner.scroll_batch(cursor, batch_size, filter)
    }

    /// Returns the current collection config.
    #[must_use]
    pub fn config(&self) -> CollectionConfig {
        self.inner.config()
    }

    /// Rebuilds the HNSW index of this collection from the vector
    /// storage, reclaiming memory occupied by tombstoned entries.
    /// Returns the number of entries compacted.
    ///
    /// Used by the server admin endpoint
    /// `POST /collections/{name}/index/rebuild` (finding F-21).
    ///
    /// # Errors
    ///
    /// Returns an error if the vacuum fails (for instance, when
    /// vector storage is disabled on the HNSW index).
    pub fn rebuild_index(&self) -> crate::error::Result<usize> {
        self.inner.vacuum_hnsw_index()
    }

    /// Applies post-creation overrides to the advanced configuration
    /// fields (`pq_rescore_oversampling`, `deferred_indexing`,
    /// `async_index_builder`) and persists the updated `config.json`.
    ///
    /// Each parameter is wrapped in an outer `Option` that expresses
    /// "leave unchanged" (`None`) versus "set to this value" (`Some(_)`).
    /// Passing `Some(None)` explicitly clears the inner field. A local
    /// clippy allow is applied because the three-state semantics are
    /// the intended contract here.
    ///
    /// # Errors
    ///
    /// Returns an error if the updated config cannot be written to disk.
    #[allow(clippy::option_option)]
    pub fn apply_advanced_config(
        &self,
        pq_rescore_oversampling: Option<Option<u32>>,
        #[cfg(feature = "persistence")] deferred_indexing: Option<
            Option<crate::collection::streaming::DeferredIndexerConfig>,
        >,
        async_index_builder: Option<
            Option<crate::collection::streaming::AsyncIndexBuilderConfig>,
        >,
    ) -> crate::error::Result<()> {
        self.inner.apply_advanced_config(
            pq_rescore_oversampling,
            #[cfg(feature = "persistence")]
            deferred_indexing,
            async_index_builder,
        )
    }

    /// Returns CBO statistics.
    #[must_use]
    pub fn get_stats(&self) -> crate::collection::stats::CollectionStats {
        self.inner.get_stats()
    }

    /// Returns `true` if the collection is a metadata-only collection.
    #[must_use]
    pub fn is_metadata_only(&self) -> bool {
        self.inner.is_metadata_only()
    }

    /// Analyzes the collection and returns fresh statistics.
    ///
    /// # Errors
    ///
    /// - Returns an error if statistics computation fails.
    pub fn analyze(&self) -> crate::error::Result<crate::collection::stats::CollectionStats> {
        self.inner.analyze()
    }

    /// Returns `true` if a secondary index exists on `field`.
    #[must_use]
    pub fn has_secondary_index(&self, field: &str) -> bool {
        self.inner.has_secondary_index(field)
    }

    /// Drops a secondary index on `field_name`. Returns `true` if the index existed.
    #[must_use]
    pub fn drop_secondary_index(&self, field_name: &str) -> bool {
        self.inner.drop_secondary_index(field_name)
    }

    /// Returns `true` if a property index exists.
    #[must_use]
    pub fn has_property_index(&self, label: &str, property: &str) -> bool {
        self.inner.has_property_index(label, property)
    }

    /// Returns `true` if a range index exists.
    #[must_use]
    pub fn has_range_index(&self, label: &str, property: &str) -> bool {
        self.inner.has_range_index(label, property)
    }

    /// Lists all index definitions on this collection.
    #[must_use]
    pub fn list_indexes(&self) -> Vec<crate::collection::IndexInfo> {
        self.inner.list_indexes()
    }

    /// Returns total memory usage of all indexes in bytes.
    #[must_use]
    pub fn indexes_memory_usage(&self) -> usize {
        self.inner.indexes_memory_usage()
    }
}
