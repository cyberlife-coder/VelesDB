//! Database facade and orchestration layer for collection lifecycle and query routing.

use crate::collection::{GraphCollection, MetadataCollection, VectorCollection};
use crate::observer::DatabaseObserver;
use crate::simd_dispatch;
use crate::{
    Collection, CollectionType, ColumnStore, DistanceMetric, Error, Result, SearchResult,
    StorageMode,
};

/// Database instance managing collections and storage.
///
/// # Lifecycle
///
/// `Database::open()` automatically loads all previously created collections from disk.
/// There is no need to call `load_collections()` separately.
///
/// # Extension (Premium)
///
/// Use [`Database::open_with_observer`] to inject a [`DatabaseObserver`] implementation
/// from `velesdb-premium` without modifying this crate.
#[cfg(feature = "persistence")]
pub struct Database {
    /// Path to the data directory
    data_dir: std::path::PathBuf,
    /// Legacy registry (Collection god-object) — kept for backward compatibility during migration.
    collections: parking_lot::RwLock<std::collections::HashMap<String, Collection>>,
    /// New registry: vector collections.
    vector_colls: parking_lot::RwLock<std::collections::HashMap<String, VectorCollection>>,
    /// New registry: graph collections.
    graph_colls: parking_lot::RwLock<std::collections::HashMap<String, GraphCollection>>,
    /// New registry: metadata-only collections.
    metadata_colls: parking_lot::RwLock<std::collections::HashMap<String, MetadataCollection>>,
    /// Cached collection statistics for CBO planning.
    collection_stats: parking_lot::RwLock<
        std::collections::HashMap<String, crate::collection::stats::CollectionStats>,
    >,
    /// Optional lifecycle observer (used by velesdb-premium for RBAC, audit, multi-tenant).
    observer: Option<std::sync::Arc<dyn DatabaseObserver>>,
}

#[cfg(feature = "persistence")]
impl Database {
    /// Opens or creates a database, **automatically loading all existing collections**.
    ///
    /// This replaces the previous `open()` + `load_collections()` two-step pattern.
    /// The new `open()` is a strict auto-load: all `config.json` directories under
    /// `path` are loaded on startup.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or accessed.
    pub fn open<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        Self::open_impl(path, None)
    }

    /// Opens a database with a [`DatabaseObserver`] (used by velesdb-premium).
    ///
    /// The observer receives lifecycle hooks for every collection operation,
    /// enabling RBAC, audit logging, multi-tenant routing, etc.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or accessed.
    pub fn open_with_observer<P: AsRef<std::path::Path>>(
        path: P,
        observer: std::sync::Arc<dyn DatabaseObserver>,
    ) -> Result<Self> {
        Self::open_impl(path, Some(observer))
    }

    fn open_impl<P: AsRef<std::path::Path>>(
        path: P,
        observer: Option<std::sync::Arc<dyn DatabaseObserver>>,
    ) -> Result<Self> {
        let data_dir = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&data_dir)?;

        // Log SIMD features detected at startup
        let features = simd_dispatch::simd_features_info();
        tracing::info!(
            avx512 = features.avx512f,
            avx2 = features.avx2,
            "SIMD features detected - direct dispatch enabled"
        );

        let db = Self {
            data_dir,
            collections: parking_lot::RwLock::new(std::collections::HashMap::new()),
            vector_colls: parking_lot::RwLock::new(std::collections::HashMap::new()),
            graph_colls: parking_lot::RwLock::new(std::collections::HashMap::new()),
            metadata_colls: parking_lot::RwLock::new(std::collections::HashMap::new()),
            collection_stats: parking_lot::RwLock::new(std::collections::HashMap::new()),
            observer,
        };

        // Auto-load all existing collections from disk (replaces manual load_collections()).
        db.load_collections()?;

        Ok(db)
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
    /// Returns an error if a collection with the same name already exists.
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
        let mut collections = self.collections.write();

        if collections.contains_key(name) {
            return Err(Error::CollectionExists(name.to_string()));
        }

        let collection_path = self.data_dir.join(name);
        let collection =
            Collection::create_with_options(collection_path, dimension, metric, storage_mode)?;
        collections.insert(name.to_string(), collection);

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
    pub fn get_collection(&self, name: &str) -> Option<Collection> {
        self.collections.read().get(name).cloned()
    }

    /// Analyzes a collection, caches stats, and persists them to disk.
    pub fn analyze_collection(
        &self,
        name: &str,
    ) -> Result<crate::collection::stats::CollectionStats> {
        let collection = self
            .get_collection(name)
            .ok_or_else(|| Error::CollectionNotFound(name.to_string()))?;
        let stats = collection.analyze()?;

        self.collection_stats
            .write()
            .insert(name.to_string(), stats.clone());

        let stats_path = self.data_dir.join(name).join("collection.stats.json");
        let serialized = serde_json::to_vec_pretty(&stats)
            .map_err(|e| Error::Serialization(format!("failed to serialize stats: {e}")))?;
        std::fs::write(&stats_path, serialized)?;

        Ok(stats)
    }

    /// Returns cached statistics when available, loading from disk if present.
    pub fn get_collection_stats(
        &self,
        name: &str,
    ) -> Result<Option<crate::collection::stats::CollectionStats>> {
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

    /// Executes a `VelesQL` query with database-level JOIN resolution.
    ///
    /// This method resolves JOIN target collections from the database registry
    /// and executes JOIN runtime in sequence.
    ///
    /// # Errors
    ///
    /// Returns an error if the base collection or any JOIN collection is missing.
    pub fn execute_query(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        crate::velesql::QueryValidator::validate(query).map_err(|e| Error::Query(e.to_string()))?;

        if let Some(dml) = query.dml.as_ref() {
            return self.execute_dml(dml, params);
        }

        if query.is_match_query() {
            return Err(Error::Query(
                "Database::execute_query does not support top-level MATCH queries. Use Collection::execute_query or pass the collection name."
                    .to_string(),
            ));
        }

        let base_name = query.select.from.clone();
        // Priority: collections registry first (contains live instances for both legacy
        // create_collection and new create_vector_collection via shared inner Arc<>).
        // Fallback to opening from disk via get_vector_collection for any missed case.
        let base_collection = self
            .get_collection(&base_name)
            .or_else(|| self.get_vector_collection(&base_name).map(|vc| vc.inner))
            .ok_or_else(|| Error::CollectionNotFound(base_name.clone()))?;

        if query.select.joins.is_empty() {
            return base_collection.execute_query(query, params);
        }

        let mut base_query = query.clone();
        base_query.select.joins.clear();

        let mut results = base_collection.execute_query(&base_query, params)?;
        for join in &query.select.joins {
            let join_collection = self
                .get_collection(&join.table)
                .or_else(|| self.get_vector_collection(&join.table).map(|vc| vc.inner))
                .ok_or_else(|| Error::CollectionNotFound(join.table.clone()))?;
            let column_store = Self::build_join_column_store(&join_collection)?;
            let joined = crate::collection::search::query::join::execute_join(
                &results,
                join,
                &column_store,
            )?;
            results = crate::collection::search::query::join::joined_to_search_results(joined);
        }

        Ok(results)
    }

    /// Lists all collection names in the database.
    pub fn list_collections(&self) -> Vec<String> {
        self.collections.read().keys().cloned().collect()
    }

    /// Deletes a collection by name.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the collection to delete
    ///
    /// # Errors
    ///
    /// Returns an error if the collection does not exist.
    pub fn delete_collection(&self, name: &str) -> Result<()> {
        let mut collections = self.collections.write();

        if collections.remove(name).is_none() {
            return Err(Error::CollectionNotFound(name.to_string()));
        }

        let collection_path = self.data_dir.join(name);
        if collection_path.exists() {
            std::fs::remove_dir_all(collection_path)?;
        }

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
        if self.collections.read().contains_key(name) || self.vector_colls.read().contains_key(name)
        {
            return Err(Error::CollectionExists(name.to_string()));
        }
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
        if self.collections.read().contains_key(name) || self.graph_colls.read().contains_key(name)
        {
            return Err(Error::CollectionExists(name.to_string()));
        }
        let path = self.data_dir.join(name);
        let coll =
            GraphCollection::create(path, name, None, DistanceMetric::Cosine, schema.clone())?;
        self.graph_colls.write().insert(name.to_string(), coll);

        if let Some(ref obs) = self.observer {
            let kind = CollectionType::Graph {
                dimension: None,
                metric: DistanceMetric::Cosine,
                schema,
            };
            obs.on_collection_created(name, &kind);
        }
        Ok(())
    }

    /// Creates a new metadata-only collection.
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_metadata_collection(&self, name: &str) -> Result<()> {
        if self.collections.read().contains_key(name)
            || self.metadata_colls.read().contains_key(name)
        {
            return Err(Error::CollectionExists(name.to_string()));
        }
        let path = self.data_dir.join(name);
        let coll = MetadataCollection::create(path, name)?;
        self.metadata_colls.write().insert(name.to_string(), coll);

        // Also register in legacy registry
        let legacy = Collection::create_metadata_only(self.data_dir.join(name), name);
        if let Ok(c) = legacy {
            self.collections.write().insert(name.to_string(), c);
        }

        if let Some(ref obs) = self.observer {
            obs.on_collection_created(name, &CollectionType::MetadataOnly);
        }
        Ok(())
    }

    /// Returns a `VectorCollection` by name.
    ///
    /// Checks the new registry first; falls back to casting a legacy `Collection` if found.
    #[must_use]
    pub fn get_vector_collection(&self, name: &str) -> Option<VectorCollection> {
        if let Some(c) = self.vector_colls.read().get(name).cloned() {
            return Some(c);
        }
        // Fallback: open from disk if registered in legacy registry
        let path = self.data_dir.join(name);
        if path.join("config.json").exists() {
            VectorCollection::open(path).ok()
        } else {
            None
        }
    }

    /// Returns a `GraphCollection` by name.
    #[must_use]
    pub fn get_graph_collection(&self, name: &str) -> Option<GraphCollection> {
        if let Some(c) = self.graph_colls.read().get(name).cloned() {
            return Some(c);
        }
        let path = self.data_dir.join(name);
        if path.join("config.json").exists() {
            GraphCollection::open(path).ok()
        } else {
            None
        }
    }

    /// Returns a `MetadataCollection` by name.
    #[must_use]
    pub fn get_metadata_collection(&self, name: &str) -> Option<MetadataCollection> {
        if let Some(c) = self.metadata_colls.read().get(name).cloned() {
            return Some(c);
        }
        let path = self.data_dir.join(name);
        if path.join("config.json").exists() {
            MetadataCollection::open(path).ok()
        } else {
            None
        }
    }

    // =========================================================================
    // Legacy typed creation (kept for backward compatibility)
    // =========================================================================

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
        let mut collections = self.collections.write();

        if collections.contains_key(name) {
            return Err(Error::CollectionExists(name.to_string()));
        }

        let collection_path = self.data_dir.join(name);
        let collection = Collection::create_typed(collection_path, name, collection_type)?;
        collections.insert(name.to_string(), collection);

        Ok(())
    }

    /// Loads existing collections from disk.
    ///
    /// # Deprecation note
    ///
    /// **This method is called automatically by [`Database::open`].**
    /// There is no need to call it manually. It is kept public only for
    /// backward compatibility with code that relied on the old two-step pattern.
    ///
    /// # Errors
    ///
    /// Returns an error if collection directories cannot be read.
    pub fn load_collections(&self) -> Result<()> {
        let mut collections = self.collections.write();

        for entry in std::fs::read_dir(&self.data_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                let config_path = path.join("config.json");
                if config_path.exists() {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        collections.entry(name)
                    {
                        match Collection::open(path) {
                            Ok(collection) => {
                                entry.insert(collection);
                            }
                            Err(err) => {
                                tracing::warn!(error = %err, "Failed to load collection");
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn execute_dml(
        &self,
        dml: &crate::velesql::DmlStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        match dml {
            crate::velesql::DmlStatement::Insert(stmt) => self.execute_insert(stmt, params),
            crate::velesql::DmlStatement::Update(stmt) => self.execute_update(stmt, params),
        }
    }

    fn execute_insert(
        &self,
        stmt: &crate::velesql::InsertStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let collection = self
            .get_collection(&stmt.table)
            .or_else(|| self.get_vector_collection(&stmt.table).map(|vc| vc.inner))
            .ok_or_else(|| Error::CollectionNotFound(stmt.table.clone()))?;

        let mut id: Option<u64> = None;
        let mut payload = serde_json::Map::new();
        let mut vector: Option<Vec<f32>> = None;

        for (column, value_expr) in stmt.columns.iter().zip(&stmt.values) {
            let resolved = Self::resolve_dml_value(value_expr, params)?;
            if column == "id" {
                id = Some(Self::json_to_u64_id(&resolved)?);
                continue;
            }
            if column == "vector" {
                vector = Some(Self::json_to_vector(&resolved)?);
                continue;
            }
            payload.insert(column.clone(), resolved);
        }

        let point_id =
            id.ok_or_else(|| Error::Query("INSERT requires integer 'id' column".to_string()))?;
        let point = if collection.is_metadata_only() {
            if vector.is_some() {
                return Err(Error::Query(
                    "INSERT on metadata-only collection cannot set 'vector'".to_string(),
                ));
            }
            crate::Point::metadata_only(point_id, serde_json::Value::Object(payload))
        } else {
            let vec_value = vector.ok_or_else(|| {
                Error::Query("INSERT on vector collection requires 'vector' column".to_string())
            })?;
            crate::Point::new(
                point_id,
                vec_value,
                Some(serde_json::Value::Object(payload)),
            )
        };

        let result = SearchResult::new(point.clone(), 0.0);
        collection.upsert(vec![point])?;
        Ok(vec![result])
    }

    fn execute_update(
        &self,
        stmt: &crate::velesql::UpdateStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let collection = self
            .get_collection(&stmt.table)
            .or_else(|| self.get_vector_collection(&stmt.table).map(|vc| vc.inner))
            .ok_or_else(|| Error::CollectionNotFound(stmt.table.clone()))?;

        let assignments = stmt
            .assignments
            .iter()
            .map(|a| Ok((a.column.clone(), Self::resolve_dml_value(&a.value, params)?)))
            .collect::<Result<Vec<_>>>()?;

        if assignments.iter().any(|(name, _)| name == "id") {
            return Err(Error::Query(
                "UPDATE cannot modify primary key column 'id'".to_string(),
            ));
        }

        let all_ids = collection.all_ids();
        let rows = collection.get(&all_ids);
        let filter = Self::build_update_filter(stmt.where_clause.as_ref())?;

        let mut updated_points = Vec::new();
        for point in rows.into_iter().flatten() {
            if !Self::matches_update_filter(&point, filter.as_ref()) {
                continue;
            }

            let mut payload_map = point
                .payload
                .as_ref()
                .and_then(serde_json::Value::as_object)
                .cloned()
                .unwrap_or_default();

            let mut updated_vector = point.vector.clone();

            for (field, value) in &assignments {
                if field == "vector" {
                    if collection.is_metadata_only() {
                        return Err(Error::Query(
                            "UPDATE on metadata-only collection cannot set 'vector'".to_string(),
                        ));
                    }
                    updated_vector = Self::json_to_vector(value)?;
                } else {
                    payload_map.insert(field.clone(), value.clone());
                }
            }

            let updated = if collection.is_metadata_only() {
                crate::Point::metadata_only(point.id, serde_json::Value::Object(payload_map))
            } else {
                crate::Point::new(
                    point.id,
                    updated_vector,
                    Some(serde_json::Value::Object(payload_map)),
                )
            };
            updated_points.push(updated);
        }

        if updated_points.is_empty() {
            return Ok(Vec::new());
        }

        let results = updated_points
            .iter()
            .map(|p| SearchResult::new(p.clone(), 0.0))
            .collect();
        collection.upsert(updated_points)?;
        Ok(results)
    }
}

#[cfg(feature = "persistence")]
mod database_helpers;

#[cfg(all(test, feature = "persistence"))]
mod database_tests;
