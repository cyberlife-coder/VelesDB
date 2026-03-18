//! Collection CRUD dispatcher: create, delete, list, get, and diagnostics.
//!
//! Type-specific operations are in sibling modules:
//! - [`vector_ops`] — vector collection create/get
//! - [`graph_ops`] — graph collection create/get
//! - [`metadata_ops`] — metadata-only collection create/get

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
        self.create_vector_collection_with_options(name, dimension, metric, storage_mode)
    }

    /// Gets a reference to a collection by name.
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
    /// # Errors
    ///
    /// Returns an error if the collection does not exist in any registry.
    pub fn delete_collection(&self, name: &str) -> Result<()> {
        let exists = self.collections.read().contains_key(name)
            || self.vector_colls.read().contains_key(name)
            || self.graph_colls.read().contains_key(name)
            || self.metadata_colls.read().contains_key(name);

        if !exists {
            return Err(Error::CollectionNotFound(name.to_string()));
        }

        let collection_path = self.data_dir.join(name);
        if collection_path.exists() {
            std::fs::remove_dir_all(&collection_path)?;
        }

        self.collections.write().remove(name);
        self.vector_colls.write().remove(name);
        self.graph_colls.write().remove(name);
        self.metadata_colls.write().remove(name);
        self.collection_stats.write().remove(name);

        if let Some(ref obs) = self.observer {
            obs.on_collection_deleted(name);
        }

        self.schema_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }

    /// Creates a new collection with a specific type (Vector, Graph, or `MetadataOnly`).
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_collection_typed(
        &self,
        name: &str,
        collection_type: &CollectionType,
    ) -> Result<()> {
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

    /// Reads and parses `config.json` from a collection directory.
    ///
    /// Returns `None` if the config file does not exist or cannot be parsed.
    pub(super) fn read_collection_config(
        &self,
        name: &str,
    ) -> Option<crate::collection::CollectionConfig> {
        let path = self.data_dir.join(name);
        let config_path = path.join("config.json");
        if !config_path.exists() {
            return None;
        }
        let data = std::fs::read_to_string(&config_path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Propagates updated query limits to all active collections.
    pub fn update_guardrails(&self, limits: &crate::guardrails::QueryLimits) {
        let collections = self.collections.read();
        for collection in collections.values() {
            collection.guard_rails.update_limits(limits);
        }
    }

    /// Returns diagnostics for a named collection.
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
