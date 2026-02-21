//! # `VelesDB` Core
//!
//! High-performance vector database engine written in Rust.
//!
//! `VelesDB` is a local-first vector database designed for semantic search,
//! recommendation systems, and RAG (Retrieval-Augmented Generation) applications.
//!
//! ## Features
//!
//! - **Blazing Fast**: HNSW index with explicit SIMD (4x faster)
//! - **5 Distance Metrics**: Cosine, Euclidean, Dot Product, Hamming, Jaccard
//! - **Hybrid Search**: Vector + BM25 full-text with RRF fusion
//! - **Quantization**: SQ8 (4x) and Binary (32x) memory compression
//! - **Persistent Storage**: Memory-mapped files for efficient disk access
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use velesdb_core::{Database, DistanceMetric, Point, StorageMode};
//! use serde_json::json;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a new database
//!     let db = Database::open("./data")?;
//!
//!     // Create a collection (all 5 metrics available)
//!     db.create_collection("documents", 768, DistanceMetric::Cosine)?;
//!     // Or with quantization: DistanceMetric::Hamming + StorageMode::Binary
//!
//!     let collection = db.get_collection("documents").ok_or("Collection not found")?;
//!
//!     // Insert vectors (upsert takes ownership)
//!     collection.upsert(vec![
//!         Point::new(1, vec![0.1; 768], Some(json!({"title": "Hello World"}))),
//!     ])?;
//!
//!     // Search for similar vectors
//!     let query_vector = vec![0.1; 768];
//!     let results = collection.search(&query_vector, 10)?;
//!
//!     // Hybrid search (vector + text)
//!     let hybrid = collection.hybrid_search(&query_vector, "hello", 5, Some(0.7))?;
//!     # Ok(())
//! }
//! ```

#![warn(missing_docs)]
// Clippy lints configured in workspace Cargo.toml [workspace.lints.clippy]
#![cfg_attr(
    test,
    allow(
        clippy::large_stack_arrays,
        clippy::doc_markdown,
        clippy::uninlined_format_args,
        clippy::single_match_else,
        clippy::cast_lossless,
        clippy::manual_assert
    )
)]

#[cfg(feature = "persistence")]
pub mod agent;
pub mod alloc_guard;
#[cfg(test)]
mod alloc_guard_tests;
pub mod cache;
#[cfg(feature = "persistence")]
pub mod collection;
#[cfg(feature = "persistence")]
pub mod column_store;
#[cfg(all(test, feature = "persistence"))]
mod column_store_tests;
pub mod compression;
pub mod config;
#[cfg(test)]
mod config_tests;
pub mod distance;
#[cfg(test)]
mod distance_tests;
pub mod error;
#[cfg(test)]
mod error_tests;
pub mod filter;
#[cfg(test)]
mod filter_like_tests;
#[cfg(test)]
mod filter_tests;
pub mod fusion;
pub mod gpu;
#[cfg(test)]
mod gpu_tests;
#[cfg(feature = "persistence")]
pub mod guardrails;
#[cfg(all(test, feature = "persistence"))]
mod guardrails_tests;
pub mod half_precision;
#[cfg(test)]
mod half_precision_tests;
#[cfg(feature = "persistence")]
pub mod index;
pub mod metrics;
#[cfg(test)]
mod metrics_tests;
pub mod perf_optimizations;
#[cfg(test)]
mod perf_optimizations_tests;
pub mod point;
#[cfg(test)]
mod point_tests;
pub mod quantization;
#[cfg(test)]
mod quantization_tests;
pub mod simd_dispatch;
#[cfg(test)]
mod simd_dispatch_tests;
#[cfg(test)]
mod simd_epic073_tests;
// simd_explicit removed - consolidated into simd_native (EPIC-075)
pub mod simd_native;
#[cfg(test)]
mod simd_native_tests;
#[cfg(target_arch = "aarch64")]
pub mod simd_neon;
#[cfg(target_arch = "aarch64")]
pub mod simd_neon_prefetch;
// simd_ops removed - direct dispatch via simd_native (EPIC-CLEANUP)
#[cfg(test)]
mod simd_prefetch_x86_tests;
#[cfg(test)]
mod simd_tests;
#[cfg(feature = "persistence")]
pub mod storage;
pub mod sync;
#[cfg(not(target_arch = "wasm32"))]
pub mod update_check;
pub mod vector_ref;
#[cfg(test)]
mod vector_ref_tests;
pub mod velesql;

#[cfg(all(not(target_arch = "wasm32"), feature = "update-check"))]
pub use update_check::{check_for_updates, spawn_update_check};
#[cfg(not(target_arch = "wasm32"))]
pub use update_check::{compute_instance_hash, UpdateCheckConfig};

#[cfg(feature = "persistence")]
pub use index::{HnswIndex, HnswParams, SearchQuality, VectorIndex};

#[cfg(feature = "persistence")]
pub use collection::{
    Collection, CollectionType, ConcurrentEdgeStore, EdgeStore, EdgeType, Element, GraphEdge,
    GraphNode, GraphSchema, IndexInfo, NodeType, TraversalResult, ValueType,
};
pub use distance::DistanceMetric;
pub use error::{Error, Result};
pub use filter::{Condition, Filter};
pub use point::{Point, SearchResult};
pub use quantization::{
    cosine_similarity_quantized, cosine_similarity_quantized_simd, dot_product_quantized,
    dot_product_quantized_simd, euclidean_squared_quantized, euclidean_squared_quantized_simd,
    BinaryQuantizedVector, QuantizedVector, StorageMode,
};

#[cfg(feature = "persistence")]
pub use column_store::{
    BatchUpdate, BatchUpdateResult, BatchUpsertResult, ColumnStore, ColumnStoreError, ColumnType,
    ColumnValue, ExpireResult, StringId, StringTable, TypedColumn, UpsertResult,
};
pub use config::{
    ConfigError, HnswConfig, LimitsConfig, LoggingConfig, QuantizationConfig, SearchConfig,
    SearchMode, ServerConfig, StorageConfig, VelesConfig,
};
pub use fusion::{FusionError, FusionStrategy};
pub use metrics::{
    average_metrics, compute_latency_percentiles, hit_rate, mean_average_precision, mrr, ndcg_at_k,
    precision_at_k, recall_at_k, LatencyStats,
};

/// Database instance managing collections and storage.
#[cfg(feature = "persistence")]
pub struct Database {
    /// Path to the data directory
    data_dir: std::path::PathBuf,
    /// Collections managed by this database
    collections: parking_lot::RwLock<std::collections::HashMap<String, Collection>>,
}

#[cfg(feature = "persistence")]
impl Database {
    /// Opens or creates a database at the specified path.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the data directory
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or accessed.
    pub fn open<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        let data_dir = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&data_dir)?;

        // Log SIMD features detected at startup
        let features = simd_dispatch::simd_features_info();
        tracing::info!(
            avx512 = features.avx512f,
            avx2 = features.avx2,
            "SIMD features detected - direct dispatch enabled"
        );

        Ok(Self {
            data_dir,
            collections: parking_lot::RwLock::new(std::collections::HashMap::new()),
        })
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
        let base_collection = self
            .get_collection(&base_name)
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
    /// Call this after opening a database to load previously created collections.
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

        collection.upsert(vec![point.clone()])?;
        Ok(vec![SearchResult::new(point, 0.0)])
    }

    fn execute_update(
        &self,
        stmt: &crate::velesql::UpdateStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let collection = self
            .get_collection(&stmt.table)
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

        collection.upsert(updated_points.clone())?;
        Ok(updated_points
            .into_iter()
            .map(|p| SearchResult::new(p, 0.0))
            .collect())
    }

    fn resolve_dml_value(
        value: &crate::velesql::Value,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        match value {
            crate::velesql::Value::Integer(v) => Ok(serde_json::json!(v)),
            crate::velesql::Value::Float(v) => Ok(serde_json::json!(v)),
            crate::velesql::Value::String(v) => Ok(serde_json::json!(v)),
            crate::velesql::Value::Boolean(v) => Ok(serde_json::json!(v)),
            crate::velesql::Value::Null => Ok(serde_json::Value::Null),
            crate::velesql::Value::Parameter(name) => params
                .get(name)
                .cloned()
                .ok_or_else(|| Error::Config(format!("Missing query parameter: ${name}"))),
            crate::velesql::Value::Temporal(expr) => Ok(serde_json::json!(expr.to_epoch_seconds())),
            crate::velesql::Value::Subquery(_) => Err(Error::Query(
                "Subquery values are not supported in INSERT/UPDATE".to_string(),
            )),
        }
    }

    fn json_to_u64_id(value: &serde_json::Value) -> Result<u64> {
        value
            .as_i64()
            .ok_or_else(|| Error::Query("id must be an integer".to_string()))
            .and_then(|v| u64::try_from(v).map_err(|_| Error::Query("id must be >= 0".to_string())))
    }

    fn json_to_vector(value: &serde_json::Value) -> Result<Vec<f32>> {
        let arr = value
            .as_array()
            .ok_or_else(|| Error::Query("'vector' must be an array of numbers".to_string()))?;
        let mut out = Vec::with_capacity(arr.len());
        for v in arr {
            let f = v
                .as_f64()
                .ok_or_else(|| Error::Query("vector values must be numeric".to_string()))?;
            if !f.is_finite() || f < f64::from(f32::MIN) || f > f64::from(f32::MAX) {
                return Err(Error::Query(
                    "vector values must be finite f32-compatible numbers".to_string(),
                ));
            }
            #[allow(clippy::cast_possible_truncation)]
            let as_f32 = f as f32;
            out.push(as_f32);
        }
        Ok(out)
    }

    fn build_update_filter(
        where_clause: Option<&crate::velesql::Condition>,
    ) -> Result<Option<crate::Filter>> {
        let Some(condition) = where_clause else {
            return Ok(None);
        };

        if Self::contains_non_metadata_condition(condition) {
            return Err(Error::Query(
                "UPDATE WHERE supports metadata predicates only (no similarity/NEAR/MATCH)"
                    .to_string(),
            ));
        }

        let filter_condition =
            crate::Collection::extract_metadata_filter(condition).ok_or_else(|| {
                Error::Query("UPDATE WHERE produced empty metadata filter".to_string())
            })?;
        Ok(Some(crate::Filter::new(crate::Condition::from(
            filter_condition,
        ))))
    }

    fn contains_non_metadata_condition(condition: &crate::velesql::Condition) -> bool {
        match condition {
            crate::velesql::Condition::Similarity(_)
            | crate::velesql::Condition::VectorSearch(_)
            | crate::velesql::Condition::VectorFusedSearch(_)
            | crate::velesql::Condition::GraphMatch(_) => true,
            crate::velesql::Condition::And(left, right)
            | crate::velesql::Condition::Or(left, right) => {
                Self::contains_non_metadata_condition(left)
                    || Self::contains_non_metadata_condition(right)
            }
            crate::velesql::Condition::Group(inner) | crate::velesql::Condition::Not(inner) => {
                Self::contains_non_metadata_condition(inner)
            }
            _ => false,
        }
    }

    fn matches_update_filter(point: &crate::Point, filter: Option<&crate::Filter>) -> bool {
        let Some(filter) = filter else {
            return true;
        };

        let mut obj = point
            .payload
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        obj.insert("id".to_string(), serde_json::json!(point.id));
        filter.matches(&serde_json::Value::Object(obj))
    }

    fn build_join_column_store(collection: &Collection) -> Result<ColumnStore> {
        use crate::column_store::{ColumnType, ColumnValue};

        let ids = collection.all_ids();
        let points: Vec<_> = collection.get(&ids).into_iter().flatten().collect();

        let mut inferred: std::collections::BTreeMap<String, ColumnType> =
            std::collections::BTreeMap::new();
        inferred.insert("id".to_string(), ColumnType::Int);

        for point in &points {
            let Some(payload) = point.payload.as_ref() else {
                continue;
            };
            let Some(obj) = payload.as_object() else {
                continue;
            };
            for (key, value) in obj {
                if key == "id" {
                    continue;
                }
                let Some(col_type) = Self::json_to_column_type(value) else {
                    continue;
                };
                if let Some(existing) = inferred.get(key) {
                    if *existing != col_type {
                        inferred.remove(key);
                    }
                } else {
                    inferred.insert(key.clone(), col_type);
                }
            }
        }

        let schema: Vec<(String, ColumnType)> = inferred.into_iter().collect();
        let schema_refs: Vec<(&str, ColumnType)> = schema
            .iter()
            .map(|(name, ty)| (name.as_str(), *ty))
            .collect();

        let mut store = ColumnStore::with_primary_key(&schema_refs, "id")
            .map_err(|e| Error::ColumnStoreError(e.to_string()))?;
        for point in &points {
            let Ok(pk) = i64::try_from(point.id) else {
                continue;
            };

            let mut values: Vec<(String, ColumnValue)> = Vec::with_capacity(schema.len());
            values.push(("id".to_string(), ColumnValue::Int(pk)));

            if let Some(obj) = point
                .payload
                .as_ref()
                .and_then(serde_json::Value::as_object)
            {
                for (key, value) in obj {
                    if key == "id" {
                        continue;
                    }
                    if !schema_refs.iter().any(|(name, _)| *name == key.as_str()) {
                        continue;
                    }
                    if let Some(column_value) = Self::json_to_column_value(value, &mut store) {
                        values.push((key.clone(), column_value));
                    }
                }
            }

            let row: Vec<(&str, ColumnValue)> = values
                .iter()
                .map(|(name, value)| (name.as_str(), value.clone()))
                .collect();
            store
                .insert_row(&row)
                .map_err(|e| Error::ColumnStoreError(e.to_string()))?;
        }

        Ok(store)
    }

    fn json_to_column_type(value: &serde_json::Value) -> Option<crate::column_store::ColumnType> {
        use crate::column_store::ColumnType;
        match value {
            serde_json::Value::Number(n) if n.is_i64() => Some(ColumnType::Int),
            serde_json::Value::Number(_) => Some(ColumnType::Float),
            serde_json::Value::String(_) => Some(ColumnType::String),
            serde_json::Value::Bool(_) => Some(ColumnType::Bool),
            _ => None,
        }
    }

    fn json_to_column_value(
        value: &serde_json::Value,
        store: &mut ColumnStore,
    ) -> Option<crate::column_store::ColumnValue> {
        use crate::column_store::ColumnValue;
        match value {
            serde_json::Value::Number(n) => {
                if let Some(v) = n.as_i64() {
                    Some(ColumnValue::Int(v))
                } else {
                    n.as_f64().map(ColumnValue::Float)
                }
            }
            serde_json::Value::String(s) => {
                let sid = store.string_table_mut().intern(s);
                Some(ColumnValue::String(sid))
            }
            serde_json::Value::Bool(b) => Some(ColumnValue::Bool(*b)),
            serde_json::Value::Null => Some(ColumnValue::Null),
            _ => None,
        }
    }
}

#[cfg(all(test, feature = "persistence"))]
mod tests {
    use super::*;
    use crate::collection::graph::GraphEdge;
    use crate::velesql::Parser;
    use tempfile::tempdir;

    #[test]
    fn test_database_open() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        assert!(db.list_collections().is_empty());
    }

    #[test]
    fn test_create_collection() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        db.create_collection("test", 768, DistanceMetric::Cosine)
            .unwrap();

        assert_eq!(db.list_collections(), vec!["test"]);
    }

    #[test]
    fn test_duplicate_collection_error() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        db.create_collection("test", 768, DistanceMetric::Cosine)
            .unwrap();

        let result = db.create_collection("test", 768, DistanceMetric::Cosine);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_collection() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        // Non-existent collection returns None
        assert!(db.get_collection("nonexistent").is_none());

        // Create and retrieve collection
        db.create_collection("test", 768, DistanceMetric::Cosine)
            .unwrap();

        let collection = db.get_collection("test");
        assert!(collection.is_some());

        let config = collection.unwrap().config();
        assert_eq!(config.dimension, 768);
        assert_eq!(config.metric, DistanceMetric::Cosine);
    }

    #[test]
    fn test_delete_collection() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        db.create_collection("to_delete", 768, DistanceMetric::Cosine)
            .unwrap();
        assert_eq!(db.list_collections().len(), 1);

        // Delete the collection
        db.delete_collection("to_delete").unwrap();
        assert!(db.list_collections().is_empty());
        assert!(db.get_collection("to_delete").is_none());
    }

    #[test]
    fn test_delete_nonexistent_collection() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        let result = db.delete_collection("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_multiple_collections() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        db.create_collection("coll1", 128, DistanceMetric::Cosine)
            .unwrap();
        db.create_collection("coll2", 256, DistanceMetric::Euclidean)
            .unwrap();
        db.create_collection("coll3", 768, DistanceMetric::DotProduct)
            .unwrap();

        let collections = db.list_collections();
        assert_eq!(collections.len(), 3);
        assert!(collections.contains(&"coll1".to_string()));
        assert!(collections.contains(&"coll2".to_string()));
        assert!(collections.contains(&"coll3".to_string()));
    }

    #[test]
    fn test_database_execute_query_join_on_end_to_end() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        db.create_collection("orders", 2, DistanceMetric::Cosine)
            .unwrap();
        db.create_collection("customers", 2, DistanceMetric::Cosine)
            .unwrap();

        let orders = db.get_collection("orders").unwrap();
        let customers = db.get_collection("customers").unwrap();

        orders
            .upsert(vec![
                Point::new(
                    1,
                    vec![1.0, 0.0],
                    Some(serde_json::json!({"id": 1, "customer_id": 10, "total": 100})),
                ),
                Point::new(
                    2,
                    vec![0.0, 1.0],
                    Some(serde_json::json!({"id": 2, "customer_id": 999, "total": 50})),
                ),
            ])
            .unwrap();
        customers
            .upsert(vec![Point::new(
                10,
                vec![1.0, 0.0],
                Some(serde_json::json!({"id": 10, "name": "Alice", "tier": "gold"})),
            )])
            .unwrap();

        let query = Parser::parse(
            "SELECT * FROM orders JOIN customers ON orders.customer_id = customers.id",
        )
        .unwrap();
        let results = db
            .execute_query(&query, &std::collections::HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 1);
        let payload = results[0].point.payload.as_ref().unwrap();
        assert_eq!(payload.get("name").unwrap().as_str(), Some("Alice"));
    }

    #[test]
    fn test_database_execute_query_join_using_with_graph_match_filter() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        db.create_collection("orders", 2, DistanceMetric::Cosine)
            .unwrap();
        db.create_collection("profiles", 2, DistanceMetric::Cosine)
            .unwrap();

        let orders = db.get_collection("orders").unwrap();
        let profiles = db.get_collection("profiles").unwrap();

        orders
            .upsert(vec![
                Point::new(
                    1,
                    vec![1.0, 0.0],
                    Some(serde_json::json!({"id": 1, "_labels": ["Doc"], "kind": "source"})),
                ),
                Point::new(
                    2,
                    vec![0.0, 1.0],
                    Some(serde_json::json!({"id": 2, "_labels": ["Doc"], "kind": "target"})),
                ),
            ])
            .unwrap();
        orders
            .add_edge(GraphEdge::new(100, 1, 2, "REL").unwrap())
            .unwrap();

        profiles
            .upsert(vec![
                Point::new(
                    1,
                    vec![1.0, 0.0],
                    Some(serde_json::json!({"id": 1, "nickname": "alpha"})),
                ),
                Point::new(
                    2,
                    vec![0.0, 1.0],
                    Some(serde_json::json!({"id": 2, "nickname": "beta"})),
                ),
            ])
            .unwrap();

        let query = Parser::parse(
            "SELECT * FROM orders AS o JOIN profiles USING (id) WHERE MATCH (o:Doc)-[:REL]->(x:Doc)",
        )
        .unwrap();
        let results = db
            .execute_query(&query, &std::collections::HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].point.id, 1);
        let payload = results[0].point.payload.as_ref().unwrap();
        assert_eq!(payload.get("nickname").unwrap().as_str(), Some("alpha"));
    }

    #[test]
    fn test_database_execute_query_supports_left_join_runtime() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        db.create_collection("orders", 2, DistanceMetric::Cosine)
            .unwrap();
        db.create_collection("customers", 2, DistanceMetric::Cosine)
            .unwrap();

        let query = Parser::parse(
            "SELECT * FROM orders LEFT JOIN customers ON orders.customer_id = customers.id",
        )
        .unwrap();
        let results = db
            .execute_query(&query, &std::collections::HashMap::new())
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_database_execute_query_rejects_join_using_multi_column() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        db.create_collection("orders", 2, DistanceMetric::Cosine)
            .unwrap();
        db.create_collection("customers", 2, DistanceMetric::Cosine)
            .unwrap();

        let query =
            Parser::parse("SELECT * FROM orders JOIN customers USING (id, customer_id)").unwrap();
        let err = db
            .execute_query(&query, &std::collections::HashMap::new())
            .unwrap_err();
        assert!(err.to_string().contains("USING(single_column)"));
    }

    #[test]
    fn test_collection_execute_query_match_order_by_property() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        db.create_collection("docs", 2, DistanceMetric::Cosine)
            .unwrap();
        let docs = db.get_collection("docs").unwrap();

        docs.upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0],
                Some(serde_json::json!({"_labels": ["Doc"], "name": "Charlie"})),
            ),
            Point::new(
                2,
                vec![1.0, 0.0],
                Some(serde_json::json!({"_labels": ["Doc"], "name": "Alice"})),
            ),
            Point::new(
                3,
                vec![1.0, 0.0],
                Some(serde_json::json!({"_labels": ["Doc"], "name": "Bob"})),
            ),
        ])
        .unwrap();

        let query =
            Parser::parse("MATCH (d:Doc) RETURN d.name ORDER BY d.name ASC LIMIT 3").unwrap();
        let results = docs
            .execute_query(&query, &std::collections::HashMap::new())
            .unwrap();

        let names: Vec<String> = results
            .iter()
            .map(|r| {
                r.point
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("name"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(names, vec!["Alice", "Bob", "Charlie"]);
    }

    #[test]
    fn test_database_execute_query_rejects_top_level_match_queries() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        db.create_collection("docs", 2, DistanceMetric::Cosine)
            .unwrap();

        let query = Parser::parse("MATCH (d:Doc) RETURN d LIMIT 10").unwrap();
        let err = db
            .execute_query(&query, &std::collections::HashMap::new())
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("Database::execute_query does not support top-level MATCH queries"));
    }

    #[test]
    fn test_database_execute_query_insert_metadata_only() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        db.create_collection_typed("products", &CollectionType::MetadataOnly)
            .unwrap();

        let query = Parser::parse(
            "INSERT INTO products (id, name, price, active) VALUES (1, 'Notebook', 12.5, true)",
        )
        .unwrap();
        let results = db
            .execute_query(&query, &std::collections::HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].point.id, 1);
        let payload = results[0].point.payload.as_ref().unwrap();
        assert_eq!(payload["name"], serde_json::json!("Notebook"));
        assert_eq!(payload["price"], serde_json::json!(12.5));
        assert_eq!(payload["active"], serde_json::json!(true));
    }

    #[test]
    fn test_database_execute_query_update_metadata_only_where_id() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        db.create_collection_typed("products", &CollectionType::MetadataOnly)
            .unwrap();
        let products = db.get_collection("products").unwrap();
        products
            .upsert_metadata(vec![Point::metadata_only(
                1,
                serde_json::json!({"name": "Notebook", "price": 10.0}),
            )])
            .unwrap();

        let query = Parser::parse("UPDATE products SET price = 19.99 WHERE id = 1").unwrap();
        let results = db
            .execute_query(&query, &std::collections::HashMap::new())
            .unwrap();
        assert_eq!(results.len(), 1);

        let updated = products.get(&[1]).into_iter().flatten().next().unwrap();
        let payload = updated.payload.unwrap();
        assert_eq!(payload["price"], serde_json::json!(19.99));
    }

    #[test]
    fn test_database_execute_query_insert_with_params() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        db.create_collection_typed("profiles", &CollectionType::MetadataOnly)
            .unwrap();

        let query = Parser::parse("INSERT INTO profiles (id, name, age) VALUES ($id, $name, $age)")
            .unwrap();
        let mut params = std::collections::HashMap::new();
        params.insert("id".to_string(), serde_json::json!(7));
        params.insert("name".to_string(), serde_json::json!("Alice"));
        params.insert("age".to_string(), serde_json::json!(30));

        db.execute_query(&query, &params).unwrap();

        let profiles = db.get_collection("profiles").unwrap();
        let point = profiles.get(&[7]).into_iter().flatten().next().unwrap();
        let payload = point.payload.unwrap();
        assert_eq!(payload["name"], serde_json::json!("Alice"));
        assert_eq!(payload["age"], serde_json::json!(30));
    }
}
