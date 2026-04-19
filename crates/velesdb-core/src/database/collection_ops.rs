//! Collection CRUD dispatcher: create, delete, list, get, and diagnostics.
//!
//! Type-specific operations are in sibling modules:
//! - [`vector_ops`] — vector collection create/get
//! - [`graph_ops`] — graph collection create/get
//! - [`metadata_ops`] — metadata-only collection create/get

use crate::collection::AnyCollection;
use crate::{CollectionType, DistanceMetric, Error, Result, StorageMode};

use super::Database;

impl Database {
    /// Ensures a collection name is valid, free in memory, and free on disk.
    ///
    /// Validates the name against path traversal and forbidden characters
    /// **before** any filesystem operation, then checks that no collection
    /// with the same name already exists in any registry or on disk, and
    /// finally enforces the `LimitsConfig::max_collections` cap so that
    /// callers are refused cleanly instead of filling the registry past
    /// the configured ceiling.
    pub(super) fn ensure_collection_name_available(&self, name: &str) -> Result<()> {
        crate::validation::validate_collection_name(name)?;

        if self.collection_exists_in_registry(name) {
            return Err(Error::CollectionExists(name.to_string()));
        }

        let collection_path = self.data_dir.join(name);
        if collection_path.exists() {
            return Err(Error::CollectionExists(name.to_string()));
        }

        // Wave 3 Commit 7 — enforce `LimitsConfig::max_collections`.
        //
        // Counted across every typed registry (vector + graph + metadata)
        // because the limit is tenant-wide, not per-type. Evaluated after
        // the name validation and duplicate checks so the typed error
        // precedence stays unchanged: invalid name and duplicate still
        // win over the cap — callers that want to detect "too many
        // collections" specifically rely on the `GuardRail` variant.
        let total_collections = self.vector_colls.read().len()
            + self.graph_colls.read().len()
            + self.metadata_colls.read().len();
        let cap = self.config.limits.max_collections;
        if total_collections >= cap {
            return Err(Error::GuardRail(format!(
                "max_collections limit reached ({total_collections} / {cap}); \
                 raise `limits.max_collections` in VelesConfig to create more"
            )));
        }

        Ok(())
    }

    /// Checks whether a collection name exists in any of the typed registries.
    fn collection_exists_in_registry(&self, name: &str) -> bool {
        self.vector_colls.read().contains_key(name)
            || self.graph_colls.read().contains_key(name)
            || self.metadata_colls.read().contains_key(name)
    }

    /// Enforces `LimitsConfig::max_dimensions` on a prospective vector
    /// collection creation.
    ///
    /// Complements [`crate::validation::validate_dimension`] (the static
    /// `65_536` hard ceiling): the config-driven limit is typically tighter
    /// — 4096 by default — and is consulted here so the guard-rail can
    /// be relaxed per tenant via [`Database::open_with_config`] without
    /// touching the static constant.
    ///
    /// Dimension `0` is accepted because it is the sentinel used by
    /// metadata-only and graph-without-embeddings collections. Callers
    /// that need to reject zero should do so upstream via
    /// [`crate::validation::validate_dimension`].
    pub(super) fn enforce_vector_dimension_limit(&self, dimension: usize) -> Result<()> {
        if dimension == 0 {
            return Ok(());
        }
        let cap = self.config.limits.max_dimensions;
        if dimension > cap {
            return Err(Error::GuardRail(format!(
                "vector dimension {dimension} exceeds configured max_dimensions cap of {cap}; \
                 raise `limits.max_dimensions` in VelesConfig to allow larger vectors"
            )));
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

    /// Returns a type-erased collection handle by name.
    ///
    /// Checks vector → graph → metadata registries in order.
    /// Returns `None` if no collection with the given name exists.
    #[must_use]
    pub fn get_any_collection(&self, name: &str) -> Option<AnyCollection> {
        if let Some(c) = self.get_vector_collection(name) {
            return Some(AnyCollection::Vector(c));
        }
        if let Some(c) = self.get_graph_collection(name) {
            return Some(AnyCollection::Graph(c));
        }
        if let Some(c) = self.get_metadata_collection(name) {
            return Some(AnyCollection::Metadata(c));
        }
        None
    }

    /// Returns the write generation for a named collection, if it exists.
    #[must_use]
    pub fn collection_write_generation(&self, name: &str) -> Option<u64> {
        if let Some(vc) = self.vector_colls.read().get(name) {
            return Some(vc.inner.write_generation());
        }
        if let Some(gc) = self.graph_colls.read().get(name) {
            return Some(gc.inner.write_generation());
        }
        if let Some(mc) = self.metadata_colls.read().get(name) {
            return Some(mc.inner.write_generation());
        }
        None
    }

    /// Returns the analyze generation for a named collection, if it exists
    /// (issue #608).
    ///
    /// Parallel to [`collection_write_generation`], but tracks `ANALYZE`
    /// invocations instead of data mutations. Threaded into the compiled plan
    /// cache key so that an `ANALYZE` run alone invalidates cached plans whose
    /// cost estimates pre-date the fresh calibrated statistics.
    #[must_use]
    pub fn collection_analyze_generation(&self, name: &str) -> Option<u64> {
        if let Some(vc) = self.vector_colls.read().get(name) {
            return Some(vc.inner.analyze_generation());
        }
        if let Some(gc) = self.graph_colls.read().get(name) {
            return Some(gc.inner.analyze_generation());
        }
        if let Some(mc) = self.metadata_colls.read().get(name) {
            return Some(mc.inner.analyze_generation());
        }
        None
    }

    /// Lists all collection names in the database.
    ///
    /// Includes collections created via any typed API (vector, graph, metadata).
    pub fn list_collections(&self) -> Vec<String> {
        let vector_colls = self.vector_colls.read();
        let graph_colls = self.graph_colls.read();
        let metadata_colls = self.metadata_colls.read();

        let mut names: std::collections::HashSet<String> = vector_colls.keys().cloned().collect();
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
    /// Returns an error if the name is invalid or the collection does not
    /// exist in any registry.
    pub fn delete_collection(&self, name: &str) -> Result<()> {
        crate::validation::validate_collection_name(name)?;

        if !self.collection_exists_in_registry(name) {
            return Err(Error::CollectionNotFound(name.to_string()));
        }

        let collection_path = self.data_dir.join(name);
        if collection_path.exists() {
            std::fs::remove_dir_all(&collection_path)?;
        }

        self.remove_from_all_registries(name);

        if let Some(ref obs) = self.observer {
            obs.on_collection_deleted(name);
        }

        self.schema_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }

    /// Removes a collection from all registries and stats cache.
    fn remove_from_all_registries(&self, name: &str) {
        self.vector_colls.write().remove(name);
        self.graph_colls.write().remove(name);
        self.metadata_colls.write().remove(name);
        self.collection_stats.write().remove(name);
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
    /// Returns `None` if the name is invalid, the config file does not exist,
    /// or the config cannot be parsed.
    pub(super) fn read_collection_config(
        &self,
        name: &str,
    ) -> Option<crate::collection::CollectionConfig> {
        if crate::validation::validate_collection_name(name).is_err() {
            return None;
        }
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
        for vc in self.vector_colls.read().values() {
            vc.guard_rails().update_limits(limits);
        }
        for gc in self.graph_colls.read().values() {
            gc.inner.guard_rails().update_limits(limits);
        }
        for mc in self.metadata_colls.read().values() {
            mc.inner.guard_rails().update_limits(limits);
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
