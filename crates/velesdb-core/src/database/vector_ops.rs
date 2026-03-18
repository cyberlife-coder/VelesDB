//! Vector collection creation and retrieval operations.

use crate::collection::VectorCollection;
use crate::{CollectionType, DistanceMetric, Result, StorageMode};

use super::Database;

#[allow(deprecated)]
impl Database {
    /// Creates a new vector collection.
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_vector_collection(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
    ) -> Result<()> {
        self.create_vector_collection_with_options(name, dimension, metric, StorageMode::default())
    }

    /// Creates a new vector collection with custom storage options.
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_vector_collection_with_options(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) -> Result<()> {
        self.ensure_collection_name_available(name)?;
        let path = self.data_dir.join(name);
        let coll = VectorCollection::create(path, name, dimension, metric, storage_mode)?;
        self.register_vector_collection(name, &coll, dimension, metric, storage_mode);
        Ok(())
    }

    /// Creates a new vector collection with custom HNSW parameters.
    ///
    /// When `m` or `ef_construction` are `Some`, those values override the
    /// dimension-based auto-tuned defaults from [`HnswParams::auto`].
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_vector_collection_with_hnsw(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
        m: Option<usize>,
        ef_construction: Option<usize>,
    ) -> Result<()> {
        self.ensure_collection_name_available(name)?;
        let path = self.data_dir.join(name);
        let coll = VectorCollection::create_with_hnsw(
            path,
            name,
            dimension,
            metric,
            storage_mode,
            m,
            ef_construction,
        )?;
        self.register_vector_collection(name, &coll, dimension, metric, storage_mode);
        Ok(())
    }

    /// Registers a vector collection in both legacy and typed registries,
    /// notifies the observer, and bumps the schema version.
    fn register_vector_collection(
        &self,
        name: &str,
        coll: &VectorCollection,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) {
        self.collections
            .write()
            .insert(name.to_string(), coll.inner.clone());
        self.vector_colls
            .write()
            .insert(name.to_string(), coll.clone());

        if let Some(ref obs) = self.observer {
            let kind = CollectionType::Vector {
                dimension,
                metric,
                storage_mode,
            };
            obs.on_collection_created(name, &kind);
        }

        self.schema_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Returns a `VectorCollection` by name.
    ///
    /// Checks the typed registry first.  If not found there, falls back to
    /// opening the collection directory from disk (e.g. for collections created
    /// via the legacy `create_collection` API that were not registered in the
    /// typed registry).  The opened instance is cached back into the registry
    /// so subsequent calls avoid the disk round-trip.
    ///
    /// Returns `None` if the collection does not exist on disk.
    #[must_use]
    pub fn get_vector_collection(&self, name: &str) -> Option<VectorCollection> {
        if let Some(c) = self.vector_colls.read().get(name).cloned() {
            return Some(c);
        }
        self.open_vector_collection_from_disk(name)
    }

    /// Disk fallback for `get_vector_collection`.
    fn open_vector_collection_from_disk(&self, name: &str) -> Option<VectorCollection> {
        let cfg = self.read_collection_config(name)?;
        if cfg.graph_schema.is_some() || cfg.metadata_only {
            return None;
        }
        let coll = VectorCollection::open(self.data_dir.join(name)).ok()?;
        self.vector_colls
            .write()
            .insert(name.to_string(), coll.clone());
        Some(coll)
    }
}
