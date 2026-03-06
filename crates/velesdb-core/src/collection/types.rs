//! Collection types and configuration.

use crate::collection::graph::{EdgeStore, GraphSchema, PropertyIndex, RangeIndex};
use crate::collection::stats::CollectionStats;
use crate::distance::DistanceMetric;
use crate::guardrails::GuardRails;
use crate::index::{Bm25Index, HnswIndex, SecondaryIndex};
use crate::quantization::{
    BinaryQuantizedVector, PQVector, ProductQuantizer, QuantizedVector, StorageMode,
};
use crate::storage::{LogPayloadStorage, MmapStorage};
use crate::velesql::{QueryCache, QueryPlanner};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

type PqTrainingSample = (u64, Vec<f32>);

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

    /// Vector dimension (0 for metadata-only or graph-without-embeddings collections).
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

    /// Graph schema — `Some` iff this is a graph collection.
    /// Persisted to config.json; `None` for vector and metadata collections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_schema: Option<GraphSchema>,

    /// Embedding dimension for graph node vectors (None = no embeddings).
    /// Only meaningful when `graph_schema` is `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_dimension: Option<usize>,

    /// PQ rescore oversampling factor. `Some(4)` by default.
    ///
    /// The search pipeline fetches `max(k * factor, k + 32)` candidates from HNSW
    /// and rescores them with full-precision ADC.
    ///
    /// - `None`: disables rescore entirely (expert-only; risks silent recall collapse).
    /// - `Some(0)`: treated as disabled (equivalent to `None`) — the oversampling factor
    ///   of 0 produces a candidates count of 0, which falls back to raw HNSW results.
    /// - `Some(n)` where `n > 0`: enables rescore with `n`-fold oversampling.
    #[serde(default = "default_pq_rescore_oversampling")]
    pub pq_rescore_oversampling: Option<u32>,
}

/// Returns `Some(4)` as the default PQ rescore oversampling factor.
/// Returns `Option` because the field type is `Option<u32>` (None = disabled).
#[allow(clippy::unnecessary_wraps)]
fn default_pq_rescore_oversampling() -> Option<u32> {
    Some(4)
}

// === LOCK ORDERING ===
// All code acquiring multiple locks on Collection MUST follow this order.
// Acquiring in any other order risks deadlock under concurrent access.
//
// Canonical order (acquire lower numbers first):
//   1. config
//   2. vector_storage
//   3. payload_storage
//   4. sq8_cache / binary_cache / pq_cache  (any order among themselves)
//   5. pq_quantizer → pq_training_buffer
//   6. secondary_indexes
//   7. property_index / range_index         (any order among themselves)
//   8. edge_store

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

    /// PQ quantized vectors cache (for ProductQuantization storage mode).
    pub(super) pq_cache: Arc<RwLock<HashMap<u64, PQVector>>>,

    /// Trained ProductQuantizer (lazy-trained on first inserted vectors).
    pub(super) pq_quantizer: Arc<RwLock<Option<ProductQuantizer>>>,

    /// Buffer of first vectors used to train PQ codebooks.
    /// Stores `(point_id, vector)` so trained quantizers can backfill cache entries.
    pub(super) pq_training_buffer: Arc<RwLock<VecDeque<PqTrainingSample>>>,

    /// Property index for O(1) equality lookups on graph nodes (EPIC-009).
    pub(super) property_index: Arc<RwLock<PropertyIndex>>,

    /// Range index for O(log n) range queries on graph nodes (EPIC-009).
    pub(super) range_index: Arc<RwLock<RangeIndex>>,

    /// Edge store for knowledge graph relationships (EPIC-015).
    pub(super) edge_store: Arc<RwLock<EdgeStore>>,

    /// Secondary indexes for metadata payload fields.
    pub(super) secondary_indexes: Arc<RwLock<HashMap<String, SecondaryIndex>>>,

    /// Guard-rails for query execution (EPIC-048).
    pub(crate) guard_rails: Arc<GuardRails>,

    /// Query planner for cost-based optimization (EPIC-046).
    pub(crate) query_planner: Arc<QueryPlanner>,

    /// Query parse cache for amortizing repeated query parsing (P1-A).
    pub(crate) query_cache: Arc<QueryCache>,

    /// Cached CBO statistics with TTL (avoids O(n) scan per query).
    pub(crate) cached_stats: Arc<Mutex<Option<(CollectionStats, std::time::Instant)>>>,
}

impl Collection {
    /// Extracts all string values from a JSON payload for text indexing.
    pub(crate) fn extract_text_from_payload(payload: &serde_json::Value) -> String {
        crate::collection::text_utils::extract_text(payload)
    }
}

#[cfg(test)]
mod rescore_config_tests {
    use super::*;
    use crate::distance::DistanceMetric;
    use crate::quantization::StorageMode;

    fn make_config(oversampling: Option<u32>) -> CollectionConfig {
        CollectionConfig {
            name: "test".to_string(),
            dimension: 128,
            metric: DistanceMetric::Euclidean,
            point_count: 0,
            storage_mode: StorageMode::ProductQuantization,
            metadata_only: false,
            graph_schema: None,
            embedding_dimension: None,
            pq_rescore_oversampling: oversampling,
        }
    }

    #[test]
    fn rescore_default_oversampling_is_4() {
        let config = make_config(default_pq_rescore_oversampling());
        assert_eq!(config.pq_rescore_oversampling, Some(4));
    }

    #[test]
    fn rescore_candidates_k_formula_default() {
        // Default factor = 4, k = 10
        // candidates_k = max(10 * 4, 10 + 32) = max(40, 42) = 42
        let factor = 4_usize;
        let k = 10_usize;
        let candidates_k = k.saturating_mul(factor).max(k + 32);
        assert_eq!(candidates_k, 42);
    }

    #[test]
    fn rescore_candidates_k_formula_custom_factor_6() {
        // factor = 6, k = 10
        // candidates_k = max(10 * 6, 10 + 32) = max(60, 42) = 60
        let factor = 6_usize;
        let k = 10_usize;
        let candidates_k = k.saturating_mul(factor).max(k + 32);
        assert_eq!(candidates_k, 60);
    }

    #[test]
    fn rescore_none_disables_oversampling() {
        let config = make_config(None);
        let oversampling = config.pq_rescore_oversampling.unwrap_or(0);
        assert_eq!(oversampling, 0, "None should map to 0 (disabled)");
    }

    #[test]
    fn rescore_active_by_default_for_pq() {
        let config = make_config(default_pq_rescore_oversampling());
        assert!(
            config.pq_rescore_oversampling.is_some(),
            "Rescore must be active by default for PQ"
        );
        assert!(
            config.pq_rescore_oversampling.unwrap() > 0,
            "Default oversampling must be > 0"
        );
    }

    #[test]
    fn rescore_serde_default_backward_compat() {
        // Simulate deserializing a config without pq_rescore_oversampling field.
        // The serde default should kick in and set Some(4).
        let json = r#"{
            "name": "old_collection",
            "dimension": 128,
            "metric": "Euclidean",
            "point_count": 100,
            "storage_mode": "productquantization"
        }"#;
        let config: CollectionConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            config.pq_rescore_oversampling,
            Some(4),
            "Missing field must deserialize to Some(4) for backward compat"
        );
    }

    #[test]
    fn rescore_minimum_floor_preserved() {
        // Even with small k, the floor k + 32 must dominate
        let factor = 4_usize;
        let k = 5_usize;
        let candidates_k = k.saturating_mul(factor).max(k + 32);
        // max(20, 37) = 37
        assert_eq!(candidates_k, 37);
    }
}
