//! Collection CRUD operations: create, delete, list, and get for all collection types.

use crate::collection::{GraphCollection, MetadataCollection, VectorCollection};
#[allow(deprecated)]
use crate::{Collection, CollectionType, DistanceMetric, Error, Result, StorageMode};

use super::Database;

// Almost every method below reads/writes `self.collections` (HashMap<String, Collection>),
// so `#[allow(deprecated)]` is applied at impl-block level rather than per-method.
#[allow(deprecated)]
impl Database {
    /// Ensures a collection name is free in memory and on disk.
    ///
    /// This prevents re-creating over a skipped/corrupted on-disk collection
    /// that was not loaded into registries.
    pub(super) fn ensure_collection_name_available(&self, name: &str) -> Result<()> {
        let exists_in_registry = self.collections.read().contains_key(name)
            || self.vector_colls.read().contains_key(name)
            || self.graph_colls.read().contains_key(name)
            || self.metadata_colls.read().contains_key(name);
        if exists_in_registry {
            return Err(Error::CollectionExists(name.to_string()));
        }

        let collection_path = self.data_dir.join(name);
        if collection_path.exists() {
            return Err(Error::CollectionExists(name.to_string()));
        }

        Ok(())
    }

    /// Creates a new collection with the specified parameters.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for the collection
    /// * `dimension` - Vector dimension (e.g., 768 for many embedding models)
    /// * `metric` - Distance metric to use for similarity calculations
    ///
    /// # Errors
    ///
    /// - Returns `Error::CollectionExists` if a collection with the same name already exists.
    /// - Returns an error if the directory cannot be created or storage initialization fails.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use velesdb_core::{Database, DistanceMetric};
    /// let db = Database::open("./data")?;
    /// db.create_collection("documents", 768, DistanceMetric::Cosine)?;
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    pub fn create_collection(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
    ) -> Result<()> {
        self.create_collection_with_options(name, dimension, metric, StorageMode::default())
    }

    /// Creates a new collection with custom storage options.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for the collection
    /// * `dimension` - Vector dimension
    /// * `metric` - Distance metric
    /// * `storage_mode` - Vector storage mode (Full, SQ8, Binary)
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_collection_with_options(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) -> Result<()> {
        self.ensure_collection_name_available(name)?;

        let collection_path = self.data_dir.join(name);
        let coll =
            VectorCollection::create(collection_path, name, dimension, metric, storage_mode)?;
        // Keep legacy and typed registries in sync (same Arc<> — zero copy).
        self.collections
            .write()
            .insert(name.to_string(), coll.inner.clone());
        self.vector_colls.write().insert(name.to_string(), coll);

        // Bump schema version (CACHE-01 DDL invalidation).
        self.schema_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }

    /// Gets a reference to a collection by name.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the collection
    ///
    /// # Returns
    ///
    /// Returns `None` if the collection does not exist.
    #[deprecated(
        since = "2.0.0",
        note = "Use get_vector_collection(), get_graph_collection(), or get_metadata_collection()"
    )]
    pub fn get_collection(&self, name: &str) -> Option<Collection> {
        self.collections.read().get(name).cloned()
    }

    /// Returns the write generation for a named collection, if it exists.
    ///
    /// Checks the legacy registry first (covers all collection types).
    #[must_use]
    pub fn collection_write_generation(&self, name: &str) -> Option<u64> {
        self.collections
            .read()
            .get(name)
            .map(crate::Collection::write_generation)
    }

    /// Lists all collection names in the database.
    ///
    /// Includes collections created via any typed API (vector, graph, metadata).
    pub fn list_collections(&self) -> Vec<String> {
        // BUG-7: acquire all locks together for a consistent point-in-time snapshot.
        let collections = self.collections.read();
        let vector_colls = self.vector_colls.read();
        let graph_colls = self.graph_colls.read();
        let metadata_colls = self.metadata_colls.read();

        let mut names: std::collections::HashSet<String> = collections.keys().cloned().collect();
        for k in vector_colls.keys() {
            names.insert(k.clone());
        }
        for k in graph_colls.keys() {
            names.insert(k.clone());
        }
        for k in metadata_colls.keys() {
            names.insert(k.clone());
        }
        let mut result: Vec<String> = names.into_iter().collect();
        result.sort();
        result
    }

    /// Deletes a collection by name.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the collection to delete
    ///
    /// # Errors
    ///
    /// Returns an error if the collection does not exist in any registry.
    ///
    /// # Concurrency
    ///
    /// The existence check and the registry removals are not a single atomic
    /// operation. Under concurrent deletion of the same collection, at most one
    /// caller receives `CollectionNotFound`; subsequent callers perform no-op
    /// `remove()` calls which are safe and idempotent. `remove_dir_all` is
    /// guarded by an `exists()` check and is therefore also safe.
    pub fn delete_collection(&self, name: &str) -> Result<()> {
        // Check existence across all registries before taking any write lock.
        let exists = self.collections.read().contains_key(name)
            || self.vector_colls.read().contains_key(name)
            || self.graph_colls.read().contains_key(name)
            || self.metadata_colls.read().contains_key(name);

        if !exists {
            return Err(Error::CollectionNotFound(name.to_string()));
        }

        // Remove from all registries and delete directory.
        // Remove directory BEFORE purging registries so that, on failure, the
        // in-memory state is still consistent (collection remains accessible).
        let collection_path = self.data_dir.join(name);
        if collection_path.exists() {
            std::fs::remove_dir_all(&collection_path)?;
        }

        // Directory is gone (or never existed) — now purge registries (BUG-6).
        self.collections.write().remove(name);
        self.vector_colls.write().remove(name);
        self.graph_colls.write().remove(name);
        self.metadata_colls.write().remove(name);
        self.collection_stats.write().remove(name);

        if let Some(ref obs) = self.observer {
            obs.on_collection_deleted(name);
        }

        // Bump schema version (CACHE-01 DDL invalidation).
        self.schema_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }

    // =========================================================================
    // New typed API (WP-4) — preferred over the legacy `create_collection` methods
    // =========================================================================

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
        // Register the inner Collection in the legacy registry so that both
        // get_collection() and get_vector_collection() use the same live instance.
        // Collection is Clone and all heavy fields are Arc<> so this is zero-copy.
        self.collections
            .write()
            .insert(name.to_string(), coll.inner.clone());
        self.vector_colls.write().insert(name.to_string(), coll);

        if let Some(ref obs) = self.observer {
            let kind = CollectionType::Vector {
                dimension,
                metric,
                storage_mode,
            };
            obs.on_collection_created(name, &kind);
        }

        // Bump schema version (CACHE-01 DDL invalidation).
        self.schema_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

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
        self.collections
            .write()
            .insert(name.to_string(), coll.inner.clone());
        self.vector_colls.write().insert(name.to_string(), coll);

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

        Ok(())
    }

    /// Creates a new graph collection.
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_graph_collection(
        &self,
        name: &str,
        schema: crate::collection::GraphSchema,
    ) -> Result<()> {
        self.ensure_collection_name_available(name)?;
        let path = self.data_dir.join(name);
        let coll =
            GraphCollection::create(path, name, None, DistanceMetric::Cosine, schema.clone())?;
        // Register in legacy registry so get_collection() and execute_query() work (BUG-2).
        self.collections
            .write()
            .insert(name.to_string(), coll.inner.clone());
        self.graph_colls.write().insert(name.to_string(), coll);

        if let Some(ref obs) = self.observer {
            let kind = CollectionType::Graph {
                dimension: None,
                metric: DistanceMetric::Cosine,
                schema,
            };
            obs.on_collection_created(name, &kind);
        }

        // Bump schema version (CACHE-01 DDL invalidation).
        self.schema_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }

    /// Creates a new metadata-only collection.
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_metadata_collection(&self, name: &str) -> Result<()> {
        self.ensure_collection_name_available(name)?;
        let path = self.data_dir.join(name);
        let coll = MetadataCollection::create(path, name)?;
        // Share the same inner Collection instance in the legacy registry so that
        // get_collection() and execute_query() see the same live data.
        self.collections
            .write()
            .insert(name.to_string(), coll.inner.clone());
        self.metadata_colls.write().insert(name.to_string(), coll);

        if let Some(ref obs) = self.observer {
            obs.on_collection_created(name, &CollectionType::MetadataOnly);
        }

        // Bump schema version (CACHE-01 DDL invalidation).
        self.schema_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }

    /// Creates a new collection with a specific type (Vector or `MetadataOnly`).
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for the collection
    /// * `collection_type` - Type of collection to create
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use velesdb_core::{Database, CollectionType, DistanceMetric, StorageMode};
    ///
    /// let db = Database::open("./data")?;
    ///
    /// // Create a metadata-only collection
    /// db.create_collection_typed("products", CollectionType::MetadataOnly)?;
    ///
    /// // Create a vector collection
    /// db.create_collection_typed("embeddings", CollectionType::Vector {
    ///     dimension: 768,
    ///     metric: DistanceMetric::Cosine,
    ///     storage_mode: StorageMode::Full,
    /// })?;
    /// ```
    pub fn create_collection_typed(
        &self,
        name: &str,
        collection_type: &CollectionType,
    ) -> Result<()> {
        // Delegate to the typed APIs so all registries stay in sync.
        match collection_type {
            CollectionType::Vector {
                dimension,
                metric,
                storage_mode,
            } => {
                self.create_vector_collection_with_options(name, *dimension, *metric, *storage_mode)
            }
            CollectionType::MetadataOnly => self.create_metadata_collection(name),
            CollectionType::Graph {
                dimension,
                metric,
                schema,
            } => self.create_graph_collection_from_type(name, *dimension, *metric, schema),
        }
    }

    /// Internal helper for `create_collection_typed` with `Graph` variant.
    fn create_graph_collection_from_type(
        &self,
        name: &str,
        dimension: Option<usize>,
        metric: DistanceMetric,
        schema: &crate::collection::GraphSchema,
    ) -> Result<()> {
        self.ensure_collection_name_available(name)?;
        let path = self.data_dir.join(name);
        let coll = GraphCollection::create(path, name, dimension, metric, schema.clone())?;
        self.collections
            .write()
            .insert(name.to_string(), coll.inner.clone());
        self.graph_colls.write().insert(name.to_string(), coll);
        if let Some(ref obs) = self.observer {
            let kind = CollectionType::Graph {
                dimension,
                metric,
                schema: schema.clone(),
            };
            obs.on_collection_created(name, &kind);
        }
        // Bump schema version (CACHE-01 DDL invalidation).
        self.schema_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    // =========================================================================
    // Typed collection getters with disk fallback
    // =========================================================================

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
        let path = self.data_dir.join(name);
        let config_path = path.join("config.json");
        if !config_path.exists() {
            return None;
        }
        // Read config to confirm this is a vector collection.
        let data = std::fs::read_to_string(&config_path).ok()?;
        let cfg = serde_json::from_str::<crate::collection::CollectionConfig>(&data).ok()?;
        if cfg.graph_schema.is_some() || cfg.metadata_only {
            return None;
        }
        let coll = VectorCollection::open(path).ok()?;
        self.vector_colls
            .write()
            .insert(name.to_string(), coll.clone());
        Some(coll)
    }

    /// Returns a `GraphCollection` by name.
    ///
    /// Checks the typed registry first.  Falls back to opening from disk if the
    /// collection was not registered in-memory (e.g. after a restart or when
    /// the collection was auto-created by a graph handler).  The instance is
    /// cached into the registry so subsequent calls are free.
    ///
    /// Returns `None` if the collection does not exist on disk.
    #[must_use]
    pub fn get_graph_collection(&self, name: &str) -> Option<GraphCollection> {
        if let Some(c) = self.graph_colls.read().get(name).cloned() {
            return Some(c);
        }
        self.open_graph_collection_from_disk(name)
    }

    /// Disk fallback for `get_graph_collection`.
    fn open_graph_collection_from_disk(&self, name: &str) -> Option<GraphCollection> {
        let path = self.data_dir.join(name);
        let config_path = path.join("config.json");
        if !config_path.exists() {
            return None;
        }
        let data = std::fs::read_to_string(&config_path).ok()?;
        let cfg = serde_json::from_str::<crate::collection::CollectionConfig>(&data).ok()?;
        cfg.graph_schema.as_ref()?;
        let coll = GraphCollection::open(path).ok()?;
        self.graph_colls
            .write()
            .insert(name.to_string(), coll.clone());
        Some(coll)
    }

    /// Returns a `MetadataCollection` by name.
    ///
    /// Checks the typed registry first.  Falls back to opening from disk for
    /// collections created before the typed API existed or after a restart.
    /// The instance is cached to avoid repeated disk reads.
    ///
    /// Returns `None` if the collection does not exist on disk.
    #[must_use]
    pub fn get_metadata_collection(&self, name: &str) -> Option<MetadataCollection> {
        if let Some(c) = self.metadata_colls.read().get(name).cloned() {
            return Some(c);
        }
        self.open_metadata_collection_from_disk(name)
    }

    /// Propagates updated query limits to all active collections.
    ///
    /// Called by the REST `PUT /guardrails` handler to ensure that runtime
    /// limit changes take effect for subsequent queries across every
    /// collection, not just new ones created after the change.
    pub fn update_guardrails(&self, limits: &crate::guardrails::QueryLimits) {
        let collections = self.collections.read();
        for collection in collections.values() {
            collection.guard_rails.update_limits(limits);
        }
    }

    /// Disk fallback for `get_metadata_collection`.
    fn open_metadata_collection_from_disk(&self, name: &str) -> Option<MetadataCollection> {
        let path = self.data_dir.join(name);
        let config_path = path.join("config.json");
        if !config_path.exists() {
            return None;
        }
        let data = std::fs::read_to_string(&config_path).ok()?;
        let cfg = serde_json::from_str::<crate::collection::CollectionConfig>(&data).ok()?;
        if !cfg.metadata_only {
            return None;
        }
        let coll = MetadataCollection::open(path).ok()?;
        self.metadata_colls
            .write()
            .insert(name.to_string(), coll.clone());
        Some(coll)
    }

    // =========================================================================
    // Diagnostics (US-006)
    // =========================================================================

    /// Returns diagnostics for a named collection.
    ///
    /// Checks vector, graph, and metadata registries in order.
    ///
    /// # Errors
    ///
    /// Returns `Error::CollectionNotFound` if the collection does not exist.
    pub fn collection_diagnostics(
        &self,
        name: &str,
    ) -> Result<crate::collection::CollectionDiagnostics> {
        if let Some(c) = self.get_vector_collection(name) {
            return Ok(c.diagnostics());
        }
        if let Some(c) = self.get_graph_collection(name) {
            return Ok(c.diagnostics());
        }
        if let Some(c) = self.get_metadata_collection(name) {
            return Ok(c.diagnostics());
        }
        Err(Error::CollectionNotFound(name.to_string()))
    }
}
