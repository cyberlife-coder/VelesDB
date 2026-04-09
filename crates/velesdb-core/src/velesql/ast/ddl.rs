//! DDL statement types for VelesQL.
//!
//! This module defines CREATE/DROP statement AST nodes.

use serde::{Deserialize, Serialize};

/// DDL statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DdlStatement {
    /// CREATE COLLECTION statement.
    CreateCollection(CreateCollectionStatement),
    /// DROP COLLECTION statement.
    DropCollection(DropCollectionStatement),
    /// CREATE INDEX ON collection (field) -- secondary metadata index.
    CreateIndex(CreateIndexStatement),
    /// DROP INDEX ON collection (field) -- remove secondary metadata index.
    DropIndex(DropIndexStatement),
    /// ANALYZE [COLLECTION] name -- compute CBO statistics.
    Analyze(AnalyzeStatement),
    /// TRUNCATE [COLLECTION] name -- delete all rows.
    Truncate(TruncateStatement),
    /// ALTER COLLECTION name SET (options) -- modify collection settings.
    AlterCollection(AlterCollectionStatement),
}

/// CREATE COLLECTION statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateCollectionStatement {
    /// Collection name.
    pub name: String,
    /// What kind of collection to create.
    pub kind: CreateCollectionKind,
}

/// Kind of collection to create.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CreateCollectionKind {
    /// Vector collection (default).
    Vector(VectorCollectionParams),
    /// Graph collection with optional embeddings.
    Graph(GraphCollectionParams),
    /// Metadata-only collection (no vectors).
    Metadata,
}

/// Parameters for creating a vector collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorCollectionParams {
    /// Vector dimension (required).
    pub dimension: usize,
    /// Distance metric name (resolved at execution time).
    pub metric: String,
    /// Storage mode: "full", "sq8", "binary", "pq".
    pub storage: Option<String>,
    /// HNSW `m` parameter.
    pub m: Option<usize>,
    /// HNSW `ef_construction` parameter.
    pub ef_construction: Option<usize>,
}

/// Parameters for creating a graph collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphCollectionParams {
    /// Optional embedding dimension (None = no embeddings).
    pub dimension: Option<usize>,
    /// Distance metric name (required if dimension is set).
    pub metric: Option<String>,
    /// Schema mode.
    pub schema_mode: GraphSchemaMode,
}

/// Graph schema mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GraphSchemaMode {
    /// No schema enforcement.
    Schemaless,
    /// Typed schema with node/edge definitions.
    Typed(Vec<SchemaDefinition>),
}

/// A single schema definition (node type or edge type).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SchemaDefinition {
    /// Node type with properties.
    Node {
        /// Node type name.
        name: String,
        /// Property definitions: (name, type_name).
        properties: Vec<(String, String)>,
    },
    /// Edge type connecting two node types.
    Edge {
        /// Edge type name.
        name: String,
        /// Source node type.
        from_type: String,
        /// Target node type.
        to_type: String,
    },
}

/// DROP COLLECTION statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropCollectionStatement {
    /// Collection name.
    pub name: String,
    /// Whether IF EXISTS was specified.
    pub if_exists: bool,
}

/// CREATE INDEX statement -- secondary metadata index on a payload field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateIndexStatement {
    /// Target collection name.
    pub collection: String,
    /// Payload field to index.
    pub field: String,
}

/// DROP INDEX statement -- remove secondary metadata index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropIndexStatement {
    /// Target collection name.
    pub collection: String,
    /// Payload field whose index to drop.
    pub field: String,
}

/// ANALYZE statement -- compute CBO statistics for query optimizer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyzeStatement {
    /// Collection name to analyze.
    pub collection: String,
}

/// TRUNCATE statement -- delete all rows from a collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TruncateStatement {
    /// Collection name to truncate.
    pub collection: String,
}

/// ALTER COLLECTION SET statement -- modify collection settings at runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlterCollectionStatement {
    /// Collection name to alter.
    pub collection: String,
    /// Key-value pairs of options to set.
    pub options: Vec<(String, String)>,
}
