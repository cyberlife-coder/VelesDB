//! Vector collection creation and retrieval operations.

use crate::collection::VectorCollection;
use crate::index::hnsw::HnswParams;
use crate::{CollectionType, DistanceMetric, Result, StorageMode};

use super::Database;

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
    /// Returns an error if a collection with the same name already exists
    /// or if the dimension exceeds the configured `max_dimensions` limit.
    pub fn create_vector_collection_with_options(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) -> Result<()> {
        self.ensure_collection_name_available(name)?;
        self.enforce_vector_dimension_limit(dimension)?;
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
    /// Shortcut for [`Database::create_vector_collection_with_params`] that
    /// only overrides `max_connections` and `ef_construction`.
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
        self.enforce_vector_dimension_limit(dimension)?;
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

    /// Creates a new vector collection with a fully specified
    /// [`HnswParams`] and an explicit `pq_rescore_oversampling` override.
    ///
    /// This is the most expressive vector constructor exposed by
    /// `Database`: callers pass every HNSW parameter — `max_connections`,
    /// `ef_construction`, `max_elements`, `alpha`, storage mode — via a
    /// single value, and override the PQ rescore factor explicitly rather
    /// than implicitly falling back to the engine default of `Some(4)`.
    /// Passing `pq_rescore_oversampling = None` keeps the persisted config
    /// in "no explicit override" mode so later migrations can recompute
    /// the factor from dataset shape.
    ///
    /// The storage mode argument wins over `hnsw_params.storage_mode` if
    /// they disagree — the field on `HnswParams` is a legacy denormalised
    /// copy that the engine keeps in sync with the collection-level value.
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists
    /// or if the underlying directory cannot be created.
    pub fn create_vector_collection_with_params(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
        hnsw_params: HnswParams,
        pq_rescore_oversampling: Option<u32>,
    ) -> Result<()> {
        self.ensure_collection_name_available(name)?;
        self.enforce_vector_dimension_limit(dimension)?;
        let path = self.data_dir.join(name);
        let coll = VectorCollection::create_with_params(
            path,
            dimension,
            metric,
            storage_mode,
            hnsw_params,
            pq_rescore_oversampling,
        )?;
        self.register_vector_collection(name, &coll, dimension, metric, storage_mode);
        Ok(())
    }

    /// Registers a vector collection in the typed registry,
    /// notifies the observer, and bumps the schema version.
    fn register_vector_collection(
        &self,
        name: &str,
        coll: &VectorCollection,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) {
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
