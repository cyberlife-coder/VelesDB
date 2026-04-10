//! DDL executor for `VelesQL`.
//!
//! Handles CREATE/DROP COLLECTION, CREATE/DROP INDEX, ANALYZE, TRUNCATE,
//! and ALTER COLLECTION by delegating to existing [`Database`] APIs.
//!
//! DML mutations (INSERT EDGE, DELETE, DELETE EDGE, SELECT EDGES,
//! INSERT NODE) live in the sibling [`dml_executor`](super::dml_executor)
//! module.

use crate::collection::graph::{EdgeType, GraphSchema, NodeType, ValueType};
use crate::velesql::{
    AlterCollectionStatement, AnalyzeStatement, CreateCollectionKind, CreateIndexStatement,
    DdlStatement, DropIndexStatement, GraphSchemaMode, SchemaDefinition, TruncateStatement,
};
use crate::{Error, Result, SearchResult};

use super::Database;

impl Database {
    /// Dispatches a DDL statement to the appropriate executor.
    ///
    /// # Errors
    ///
    /// Returns an error if the observer rejects the operation (RBAC)
    /// or if the collection operation itself fails.
    pub(super) fn execute_ddl(&self, ddl: &DdlStatement) -> Result<Vec<SearchResult>> {
        // RBAC hook — allows premium extensions to reject DDL.
        if let Some(ref observer) = self.observer {
            let (operation, name) = ddl_operation_info(ddl);
            observer.on_ddl_request(operation, &name)?;
        }

        match ddl {
            DdlStatement::CreateCollection(stmt) => self.execute_create_collection(stmt),
            DdlStatement::DropCollection(stmt) => self.execute_drop_collection(stmt),
            DdlStatement::CreateIndex(stmt) => self.execute_create_index(stmt),
            DdlStatement::DropIndex(stmt) => self.execute_drop_index(stmt),
            DdlStatement::Analyze(stmt) => self.execute_analyze(stmt),
            DdlStatement::Truncate(stmt) => self.execute_truncate(stmt),
            DdlStatement::AlterCollection(stmt) => self.execute_alter_collection(stmt),
        }
    }

    /// Executes a CREATE COLLECTION statement.
    ///
    /// Delegates to the appropriate typed creation API based on the
    /// collection kind (Vector, Graph, or Metadata).
    ///
    /// # Errors
    ///
    /// Returns an error if the collection already exists or parameters are invalid.
    fn execute_create_collection(
        &self,
        stmt: &crate::velesql::CreateCollectionStatement,
    ) -> Result<Vec<SearchResult>> {
        match &stmt.kind {
            CreateCollectionKind::Vector(params) => self.create_vector_from_ddl(&stmt.name, params),
            CreateCollectionKind::Graph(params) => self.create_graph_from_ddl(&stmt.name, params),
            CreateCollectionKind::Metadata => {
                self.create_metadata_collection(&stmt.name)?;
                Ok(Vec::new())
            }
        }
    }

    /// Creates a vector collection from DDL parameters.
    fn create_vector_from_ddl(
        &self,
        name: &str,
        params: &crate::velesql::VectorCollectionParams,
    ) -> Result<Vec<SearchResult>> {
        let metric = resolve_metric(&params.metric)?;
        let storage = resolve_storage_mode(params.storage.as_deref())?;

        if params.m.is_some() || params.ef_construction.is_some() {
            self.create_vector_collection_with_hnsw(
                name,
                params.dimension,
                metric,
                storage,
                params.m,
                params.ef_construction,
            )?;
        } else {
            self.create_vector_collection_with_options(name, params.dimension, metric, storage)?;
        }
        Ok(Vec::new())
    }

    /// Creates a graph collection from DDL parameters.
    fn create_graph_from_ddl(
        &self,
        name: &str,
        params: &crate::velesql::GraphCollectionParams,
    ) -> Result<Vec<SearchResult>> {
        let schema = build_graph_schema(&params.schema_mode);

        if let Some(dim) = params.dimension {
            let metric_str = params.metric.as_deref().unwrap_or("cosine");
            let metric = resolve_metric(metric_str)?;
            self.create_graph_collection_with_embeddings(name, schema, dim, metric)?;
        } else {
            self.create_graph_collection(name, schema)?;
        }
        Ok(Vec::new())
    }

    /// Executes a DROP COLLECTION statement.
    ///
    /// When `IF EXISTS` is specified, silently succeeds if the collection
    /// does not exist instead of returning an error.
    ///
    /// # Errors
    ///
    /// Returns an error if the collection does not exist (without IF EXISTS)
    /// or if the deletion itself fails.
    fn execute_drop_collection(
        &self,
        stmt: &crate::velesql::DropCollectionStatement,
    ) -> Result<Vec<SearchResult>> {
        match self.delete_collection(&stmt.name) {
            Ok(()) => Ok(Vec::new()),
            Err(Error::CollectionNotFound(_)) if stmt.if_exists => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Executes a CREATE INDEX statement.
    ///
    /// Resolves the collection (vector or legacy) and creates a secondary
    /// `BTree` index on the specified payload field.  Index creation is
    /// idempotent -- creating the same index twice is a no-op.
    ///
    /// # Errors
    ///
    /// Returns an error if the collection does not exist.
    fn execute_create_index(&self, stmt: &CreateIndexStatement) -> Result<Vec<SearchResult>> {
        let collection = self.resolve_writable_collection(&stmt.collection)?;
        collection.create_index(&stmt.field)?;
        Ok(Vec::new())
    }

    /// Executes a DROP INDEX statement.
    ///
    /// Resolves the collection and removes the secondary metadata index for
    /// the specified field.  Silently succeeds if no such index existed.
    ///
    /// # Errors
    ///
    /// Returns an error if the collection does not exist.
    fn execute_drop_index(&self, stmt: &DropIndexStatement) -> Result<Vec<SearchResult>> {
        let collection = self.resolve_writable_collection(&stmt.collection)?;
        let _ = collection.drop_secondary_index(&stmt.field);
        Ok(Vec::new())
    }

    /// Executes an ANALYZE statement.
    ///
    /// Delegates to [`Database::analyze_collection`] and returns the
    /// computed statistics as a JSON payload in a single `SearchResult`.
    ///
    /// # Errors
    ///
    /// Returns an error if the collection does not exist or analysis fails.
    fn execute_analyze(&self, stmt: &AnalyzeStatement) -> Result<Vec<SearchResult>> {
        let stats = self.analyze_collection(&stmt.collection)?;
        let stats_json = serde_json::to_value(&stats)
            .unwrap_or_else(|_| serde_json::json!({"error": "failed to serialize stats"}));
        let result = SearchResult::new(crate::Point::metadata_only(0, stats_json), 0.0);
        Ok(vec![result])
    }

    /// Executes a TRUNCATE statement.
    ///
    /// Retrieves all point IDs and deletes them, returning a payload
    /// with the count of deleted points. Returns success with
    /// `deleted_count: 0` if the collection is already empty.
    ///
    /// Checks vector/legacy collections first, then falls back to
    /// metadata collections (which `resolve_writable_collection` skips).
    ///
    /// # Errors
    ///
    /// Returns an error if the collection does not exist or deletion fails.
    fn execute_truncate(&self, stmt: &TruncateStatement) -> Result<Vec<SearchResult>> {
        // Graph collections have both nodes and edges — handle separately.
        if let Some(gc) = self.get_graph_collection(&stmt.collection) {
            return Self::truncate_graph(&gc);
        }
        // Vector/legacy + metadata fallback.
        let collection = self
            .resolve_writable_collection(&stmt.collection)
            .or_else(|_| self.resolve_collection(&stmt.collection))?;
        let ids = collection.all_point_ids();
        let count = ids.len();
        if !ids.is_empty() {
            collection.delete(&ids)?;
        }
        let payload = serde_json::json!({"deleted_count": count});
        let result = SearchResult::new(crate::Point::metadata_only(0, payload), 0.0);
        Ok(vec![result])
    }

    /// Truncates a graph collection: removes all edges then all nodes.
    fn truncate_graph(gc: &crate::collection::GraphCollection) -> Result<Vec<SearchResult>> {
        // Remove all edges first (edges reference nodes).
        let edges = gc.get_edges(None);
        let edge_count = edges.len();
        for edge in &edges {
            let _ = gc.remove_edge(edge.id());
        }
        // Remove all node payloads.
        let node_ids = gc.all_node_ids();
        let node_count = node_ids.len();
        if !node_ids.is_empty() {
            gc.delete(&node_ids)?;
        }
        let payload = serde_json::json!({
            "deleted_nodes": node_count,
            "deleted_edges": edge_count,
            "deleted_count": node_count + edge_count,
        });
        let result = SearchResult::new(crate::Point::metadata_only(0, payload), 0.0);
        Ok(vec![result])
    }

    /// Executes an ALTER COLLECTION SET statement.
    ///
    /// Currently supports only the `auto_reindex` option (boolean).
    /// Executes an `ALTER COLLECTION ... SET <key> = <value>` statement.
    ///
    /// Persistence of `ALTER COLLECTION SET` options is tracked under
    /// US-300 and is not currently implemented. The grammar accepts the
    /// syntax so existing documentation examples and tests parse without
    /// error, but the execution path returns a diagnostic error so that
    /// callers cannot mistakenly assume the option took effect.
    ///
    /// Validation order is chosen to return the most actionable error
    /// first:
    ///   1. Collection existence — `Error::CollectionNotFound`.
    ///   2. Per-option syntax (unknown key, unparseable value) —
    ///      `Error::Query` via [`apply_alter_option`].
    ///   3. Feature-gap rejection referencing US-300.
    ///
    /// # Errors
    ///
    /// Returns `Error::CollectionNotFound` or `Error::Query` as described
    /// above. A successful result is not currently possible.
    fn execute_alter_collection(
        &self,
        stmt: &AlterCollectionStatement,
    ) -> Result<Vec<SearchResult>> {
        // Step 1: existence check.
        let _collection = self.resolve_writable_collection(&stmt.collection)?;

        // Step 2: per-option syntax validation.
        for (key, value) in &stmt.options {
            apply_alter_option(key, value)?;
        }

        // Step 3: feature-gap rejection (US-300).
        let option_list = stmt
            .options
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ");
        Err(Error::Query(format!(
            "ALTER COLLECTION SET is parsed but not yet implemented \
             (tracked under US-300). Requested options on collection '{}': \
             [{}]. Workaround: drop and recreate the collection with \
             CREATE COLLECTION.",
            stmt.collection, option_list
        )))
    }
}

// ---------------------------------------------------------------------------
// Private helper functions
// ---------------------------------------------------------------------------

/// Extracts the operation name and collection name from a DDL statement.
///
/// Used by the RBAC hook to identify the operation being requested.
fn ddl_operation_info(ddl: &DdlStatement) -> (&str, String) {
    match ddl {
        DdlStatement::CreateCollection(stmt) => ("CREATE", stmt.name.clone()),
        DdlStatement::DropCollection(stmt) => ("DROP", stmt.name.clone()),
        DdlStatement::CreateIndex(stmt) => ("CREATE_INDEX", stmt.collection.clone()),
        DdlStatement::DropIndex(stmt) => ("DROP_INDEX", stmt.collection.clone()),
        DdlStatement::Analyze(stmt) => ("ANALYZE", stmt.collection.clone()),
        DdlStatement::Truncate(stmt) => ("TRUNCATE", stmt.collection.clone()),
        DdlStatement::AlterCollection(stmt) => ("ALTER", stmt.collection.clone()),
    }
}

/// Resolves a metric name string to a `DistanceMetric` enum.
///
/// # Errors
///
/// Returns a query error if the metric name is unrecognized.
fn resolve_metric(s: &str) -> Result<crate::DistanceMetric> {
    crate::DistanceMetric::parse_alias(s).ok_or_else(|| {
        Error::Query(format!(
            "Unknown metric '{s}'. Use: cosine, euclidean, dot, hamming, jaccard"
        ))
    })
}

/// Resolves an optional storage mode string to a `StorageMode` enum.
///
/// Defaults to `StorageMode::Full` when `None` is provided.
///
/// # Errors
///
/// Returns a query error if the storage mode name is unrecognized.
fn resolve_storage_mode(s: Option<&str>) -> Result<crate::StorageMode> {
    let Some(name) = s else {
        return Ok(crate::StorageMode::default());
    };
    crate::StorageMode::parse_alias(name).ok_or_else(|| {
        Error::Query(format!(
            "Unknown storage mode '{name}'. Use: full, sq8, binary, pq, rabitq"
        ))
    })
}

/// Maps a `VelesQL` type name string to a `ValueType`.
fn resolve_value_type(s: &str) -> ValueType {
    match s.to_uppercase().as_str() {
        "INTEGER" | "INT" => ValueType::Integer,
        "FLOAT" | "DOUBLE" => ValueType::Float,
        "BOOLEAN" | "BOOL" => ValueType::Boolean,
        "VECTOR" | "EMBEDDING" => ValueType::Vector,
        // "STRING", "TEXT", and any unrecognized type default to String.
        _ => ValueType::String,
    }
}

/// Builds a `GraphSchema` from the AST `GraphSchemaMode`.
fn build_graph_schema(mode: &GraphSchemaMode) -> GraphSchema {
    match mode {
        GraphSchemaMode::Schemaless => GraphSchema::schemaless(),
        GraphSchemaMode::Typed(definitions) => build_typed_schema(definitions),
    }
}

/// Builds a typed graph schema from a list of schema definitions.
fn build_typed_schema(definitions: &[SchemaDefinition]) -> GraphSchema {
    let mut schema = GraphSchema::new();

    for def in definitions {
        match def {
            SchemaDefinition::Node { name, properties } => {
                let props: std::collections::HashMap<String, ValueType> = properties
                    .iter()
                    .map(|(k, v)| (k.clone(), resolve_value_type(v)))
                    .collect();
                schema = schema.with_node_type(NodeType::new(name).with_properties(props));
            }
            SchemaDefinition::Edge {
                name,
                from_type,
                to_type,
            } => {
                schema = schema.with_edge_type(EdgeType::new(name, from_type, to_type));
            }
        }
    }

    schema
}

/// Validates the syntax of a single `ALTER COLLECTION` option.
///
/// Returns `Ok(())` when the option key is recognized and the value
/// parses into the expected type. The parsed value is discarded because
/// `execute_alter_collection` rejects the entire statement with a
/// feature-gap error (US-300). This function is kept separate so that
/// malformed options surface a specific diagnostic before the generic
/// feature-gap error.
///
/// Supported options: `auto_reindex` (boolean).
///
/// # Errors
///
/// Returns `Error::Query` for unknown option keys or unparseable values.
fn apply_alter_option(key: &str, value: &str) -> Result<()> {
    match key {
        "auto_reindex" => value.parse::<bool>().map(|_| ()).map_err(|_| {
            Error::Query(format!(
                "auto_reindex must be 'true' or 'false', got '{value}'"
            ))
        }),
        _ => Err(Error::Query(format!(
            "Unsupported ALTER option: '{key}'. Supported: auto_reindex"
        ))),
    }
}
