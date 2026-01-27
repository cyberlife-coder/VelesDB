//! Collection types and configuration.

use crate::collection::graph::{EdgeStore, GraphSchema, PropertyIndex, RangeIndex};
use crate::distance::DistanceMetric;
use crate::index::{Bm25Index, HnswIndex};
use crate::quantization::{BinaryQuantizedVector, QuantizedVector, StorageMode};
use crate::storage::{LogPayloadStorage, MmapStorage};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Type of collection: Vector-based or Metadata-only.
///
/// # Examples
///
/// ```rust,ignore
/// use velesdb_core::{CollectionType, DistanceMetric, StorageMode};
///
/// // Vector collection (standard)
/// let vector_type = CollectionType::Vector {
///     dimension: 768,
///     metric: DistanceMetric::Cosine,
///     storage_mode: StorageMode::Full,
/// };
///
/// // Metadata-only collection (no vectors)
/// let metadata_type = CollectionType::MetadataOnly;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CollectionType {
    /// Standard vector collection with HNSW index.
    Vector {
        /// Vector dimension (e.g., 768 for BERT embeddings).
        dimension: usize,
        /// Distance metric for similarity calculations.
        metric: DistanceMetric,
        /// Storage mode for vector quantization.
        storage_mode: StorageMode,
    },
    /// Metadata-only collection (no vectors, no HNSW index).
    ///
    /// Ideal for reference tables, catalogs, and metadata storage.
    /// Supports CRUD operations and `VelesQL` queries on payload.
    /// Does NOT support vector search operations.
    MetadataOnly,

    /// Graph collection for knowledge graph storage.
    ///
    /// Supports heterogeneous nodes (with optional embeddings) and typed edges.
    /// Ideal for agentic memory, knowledge graphs, and entity-relationship storage.
    Graph {
        /// Optional vector dimension for node embeddings.
        dimension: Option<usize>,
        /// Distance metric for similarity (if embeddings are used).
        metric: DistanceMetric,
        /// Graph schema (strict or schemaless).
        schema: GraphSchema,
    },
}

impl Default for CollectionType {
    fn default() -> Self {
        Self::Vector {
            dimension: 768,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
        }
    }
}

impl CollectionType {
    /// Returns true if this is a metadata-only collection.
    #[must_use]
    pub const fn is_metadata_only(&self) -> bool {
        matches!(self, Self::MetadataOnly)
    }

    /// Returns the dimension if this is a vector collection.
    #[must_use]
    pub fn dimension(&self) -> Option<usize> {
        match self {
            Self::Vector { dimension, .. } => Some(*dimension),
            Self::Graph { dimension, .. } => *dimension,
            Self::MetadataOnly => None,
        }
    }

    /// Returns true if this is a graph collection.
    #[must_use]
    pub const fn is_graph(&self) -> bool {
        matches!(self, Self::Graph { .. })
    }

    /// Returns the graph schema if this is a graph collection.
    #[must_use]
    pub fn graph_schema(&self) -> Option<&GraphSchema> {
        match self {
            Self::Graph { schema, .. } => Some(schema),
            _ => None,
        }
    }
}

/// Metadata for a collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionConfig {
    /// Name of the collection.
    pub name: String,

    /// Vector dimension (0 for metadata-only collections).
    pub dimension: usize,

    /// Distance metric.
    pub metric: DistanceMetric,

    /// Number of points in the collection.
    pub point_count: usize,

    /// Storage mode for vectors (Full, SQ8, Binary).
    #[serde(default)]
    pub storage_mode: StorageMode,

    /// Whether this is a metadata-only collection.
    #[serde(default)]
    pub metadata_only: bool,
}

/// A collection of vectors with associated metadata.
#[derive(Clone)]
pub struct Collection {
    /// Path to the collection data.
    pub(super) path: PathBuf,

    /// Collection configuration.
    pub(super) config: Arc<RwLock<CollectionConfig>>,

    /// Vector storage (on-disk, memory-mapped).
    pub(super) vector_storage: Arc<RwLock<MmapStorage>>,

    /// Payload storage (on-disk, log-structured).
    pub(super) payload_storage: Arc<RwLock<LogPayloadStorage>>,

    /// HNSW index for fast approximate nearest neighbor search.
    pub(super) index: Arc<HnswIndex>,

    /// BM25 index for full-text search.
    pub(super) text_index: Arc<Bm25Index>,

    /// SQ8 quantized vectors cache (for SQ8 storage mode).
    pub(super) sq8_cache: Arc<RwLock<HashMap<u64, QuantizedVector>>>,

    /// Binary quantized vectors cache (for Binary storage mode).
    pub(super) binary_cache: Arc<RwLock<HashMap<u64, BinaryQuantizedVector>>>,

    /// Property index for O(1) equality lookups on graph nodes (EPIC-009).
    pub(super) property_index: Arc<RwLock<PropertyIndex>>,

    /// Range index for O(log n) range queries on graph nodes (EPIC-009).
    pub(super) range_index: Arc<RwLock<RangeIndex>>,

    /// Edge store for knowledge graph relationships (EPIC-015).
    pub(super) edge_store: Arc<RwLock<EdgeStore>>,
}

impl Collection {
    /// Extracts all string values from a JSON payload for text indexing.
    pub(crate) fn extract_text_from_payload(payload: &serde_json::Value) -> String {
        let mut texts = Vec::new();
        Self::collect_strings(payload, &mut texts);
        texts.join(" ")
    }

    /// Recursively collects all string values from a JSON value.
    fn collect_strings(value: &serde_json::Value, texts: &mut Vec<String>) {
        match value {
            serde_json::Value::String(s) => texts.push(s.clone()),
            serde_json::Value::Array(arr) => {
                for item in arr {
                    Self::collect_strings(item, texts);
                }
            }
            serde_json::Value::Object(obj) => {
                for v in obj.values() {
                    Self::collect_strings(v, texts);
                }
            }
            _ => {}
        }
    }
}
