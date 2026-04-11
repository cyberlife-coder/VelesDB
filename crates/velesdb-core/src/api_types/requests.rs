//! Canonical request DTOs for the `VelesDB` API.

use std::collections::BTreeMap;

use serde::Deserialize;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use super::{
    default_avg_weight, default_collection_type, default_dense_weight, default_fusion_strategy,
    default_hit_weight, default_index_type, default_max_weight, default_metric, default_rrf_k,
    default_sparse_weight, default_storage_mode, default_top_k, default_vector_weight, serde_id,
};

// ============================================================================
// Collection Types
// ============================================================================

/// Request to create a new collection.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateCollectionRequest {
    /// Collection name.
    #[cfg_attr(feature = "openapi", schema(example = "documents"))]
    pub name: String,
    /// Vector dimension (required for vector collections, ignored for `metadata_only`).
    #[cfg_attr(feature = "openapi", schema(example = 768))]
    pub dimension: Option<usize>,
    /// Distance metric (cosine, euclidean, dot, hamming, jaccard).
    #[serde(default = "default_metric")]
    #[cfg_attr(feature = "openapi", schema(example = "cosine"))]
    pub metric: String,
    /// Storage mode (full, sq8, binary). Defaults to full.
    #[serde(default = "default_storage_mode")]
    #[cfg_attr(feature = "openapi", schema(example = "full"))]
    pub storage_mode: String,
    /// Collection type: "vector" (default) or "`metadata_only`".
    #[serde(default = "default_collection_type")]
    #[cfg_attr(feature = "openapi", schema(example = "vector"))]
    pub collection_type: String,
    /// HNSW M parameter (number of bi-directional links per node).
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = 32, nullable))]
    pub hnsw_m: Option<usize>,
    /// HNSW `ef_construction` parameter (candidate list size during build).
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = 400, nullable))]
    pub hnsw_ef_construction: Option<usize>,
    /// VAMANA alpha for neighbor diversification (default derived from dimension).
    ///
    /// Values > 1.0 bias `select_neighbors` toward diversity over proximity,
    /// producing a more navigable graph with better recall. The engine
    /// default is 1.2 (VAMANA paper recommendation). Set to 1.0 for strict
    /// nearest-neighbor selection.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = 1.2, nullable))]
    pub hnsw_alpha: Option<f32>,
    /// Initial HNSW capacity (grows automatically if exceeded).
    ///
    /// Pre-sizing matters for bulk imports: the engine avoids repeated
    /// realloc-and-copy cycles when the final population is known upfront.
    /// Use this when migrating a large existing corpus into a fresh
    /// collection.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = 1_000_000, nullable))]
    pub hnsw_max_elements: Option<usize>,
    /// PQ rescore oversampling factor (default 4).
    ///
    /// The search pipeline fetches `max(k * factor, k + 32)` candidates
    /// from HNSW and rescores them with full-precision ADC. Only
    /// meaningful for quantised storage modes (SQ8, PQ). `Some(0)` is
    /// treated as "disabled".
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = 4, nullable))]
    pub pq_rescore_oversampling: Option<u32>,
    /// Deferred indexing configuration (`US-366`).
    ///
    /// When present, inserts are buffered in memory and batch-merged
    /// into the HNSW index when the buffer reaches `merge_threshold`.
    /// Accepted as a free-form JSON object matching
    /// `velesdb_core::collection::streaming::DeferredIndexerConfig`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred_indexing: Option<serde_json::Value>,
    /// Async index builder configuration (Issue `#488` — Bulk Insert V2).
    ///
    /// When present, enables the `AsyncIndexBuilder` for deferred HNSW
    /// insertion during bulk import. Accepted as a free-form JSON object
    /// matching
    /// `velesdb_core::collection::streaming::AsyncIndexBuilderConfig`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub async_index_builder: Option<serde_json::Value>,
    /// Graph schema (only for `collection_type = "graph"`).
    ///
    /// Accepted as a free-form JSON object matching
    /// `velesdb_core::GraphSchema`. `GraphSchema::schemaless()` is
    /// used when the field is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_schema: Option<serde_json::Value>,
}

// ============================================================================
// Sparse Vector Types
// ============================================================================

/// Input format for sparse vectors, supporting two JSON representations:
///
/// - **Parallel arrays**: `{"indices": [42, 1337], "values": [0.5, 1.2]}`
/// - **Dict format** (Qdrant-compatible): `{"42": 0.5, "1337": 1.2}`
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(untagged)]
#[non_exhaustive]
pub enum SparseVectorInput {
    /// Canonical parallel-array format with explicit indices and values.
    Parallel {
        /// Term/dimension indices (u32).
        indices: Vec<u32>,
        /// Corresponding weights.
        values: Vec<f32>,
    },
    /// Dictionary format mapping string term IDs to weights.
    Dict(BTreeMap<String, f32>),
}

impl SparseVectorInput {
    /// Converts this input into a core `SparseVector`, validating at the API boundary.
    ///
    /// # Errors
    ///
    /// Returns a descriptive error string on mismatched lengths, non-finite values,
    /// or dict keys that cannot be parsed as `u32`.
    pub fn into_sparse_vector(self) -> Result<crate::sparse_index::SparseVector, String> {
        match self {
            Self::Parallel { indices, values } => Self::convert_parallel(indices, values),
            Self::Dict(map) => Self::convert_dict(map),
        }
    }

    fn convert_parallel(
        indices: Vec<u32>,
        values: Vec<f32>,
    ) -> Result<crate::sparse_index::SparseVector, String> {
        if indices.len() != values.len() {
            return Err(format!(
                "Sparse vector indices/values length mismatch: {} indices vs {} values",
                indices.len(),
                values.len()
            ));
        }
        for (i, &v) in values.iter().enumerate() {
            if !v.is_finite() {
                return Err(format!(
                    "Sparse vector value at index {i} is not finite: {v}"
                ));
            }
        }
        let pairs: Vec<(u32, f32)> = indices.into_iter().zip(values).collect();
        Ok(crate::sparse_index::SparseVector::new(pairs))
    }

    fn convert_dict(
        map: BTreeMap<String, f32>,
    ) -> Result<crate::sparse_index::SparseVector, String> {
        let mut pairs = Vec::with_capacity(map.len());
        for (key, value) in map {
            if !value.is_finite() {
                return Err(format!(
                    "Sparse vector value for key '{key}' is not finite: {value}"
                ));
            }
            let idx: u32 = key
                .parse()
                .map_err(|_| format!("Sparse vector key '{key}' is not a valid u32 term ID"))?;
            pairs.push((idx, value));
        }
        Ok(crate::sparse_index::SparseVector::new(pairs))
    }
}

/// Fusion configuration for hybrid dense+sparse search.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FusionRequest {
    /// Fusion strategy: "rrf", "rsf" (alias "`relative_score`"),
    /// "average" (alias "avg"), "maximum" (alias "max"), or "weighted".
    #[cfg_attr(feature = "openapi", schema(example = "rrf"))]
    pub strategy: String,
    /// RRF k parameter (only for strategy = "rrf", default 60).
    #[serde(default)]
    pub k: Option<u32>,
    /// Dense weight (only for strategy = "rsf", default 0.5).
    #[serde(default)]
    pub dense_w: Option<f32>,
    /// Sparse weight (only for strategy = "rsf", default 0.5).
    #[serde(default)]
    pub sparse_w: Option<f32>,
    /// Weighted fusion: weight applied to the average score component
    /// (only for strategy = "weighted", default 0.5).
    #[serde(default)]
    pub avg_w: Option<f32>,
    /// Weighted fusion: weight applied to the maximum score component
    /// (only for strategy = "weighted", default 0.3).
    #[serde(default)]
    pub max_w: Option<f32>,
    /// Weighted fusion: weight applied to the hit-ratio component
    /// (only for strategy = "weighted", default 0.2).
    #[serde(default)]
    pub hit_w: Option<f32>,
}

// ============================================================================
// Point Types
// ============================================================================

/// Request to upsert points.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpsertPointsRequest {
    /// Points to upsert.
    pub points: Vec<PointRequest>,
}

/// A point in an upsert request.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PointRequest {
    /// Point ID.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub id: u64,
    /// Vector data.
    pub vector: Vec<f32>,
    /// Optional payload.
    pub payload: Option<serde_json::Value>,
    /// Single sparse vector (convenience, stored under default name `""`).
    #[serde(default)]
    pub sparse_vector: Option<SparseVectorInput>,
    /// Named sparse vectors map.
    #[serde(default)]
    pub sparse_vectors: Option<BTreeMap<String, SparseVectorInput>>,
}

/// Request body for the streaming insert endpoint (single point).
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct StreamInsertRequest {
    /// Point ID.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub id: u64,
    /// Dense vector data.
    pub vector: Vec<f32>,
    /// Optional JSON payload (metadata).
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

// ============================================================================
// Search Types
// ============================================================================

/// Request for vector search (dense, sparse, or hybrid).
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SearchRequest {
    /// Query vector for dense search.
    #[serde(default)]
    pub vector: Vec<f32>,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Search mode preset: fast, balanced, accurate, perfect.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = "balanced"))]
    pub mode: Option<String>,
    /// HNSW `ef_search` parameter.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = 128))]
    pub ef_search: Option<usize>,
    /// Query timeout in milliseconds.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = 30000))]
    pub timeout_ms: Option<u64>,
    /// Optional metadata filter.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
    /// Single sparse query vector.
    #[serde(default)]
    pub sparse_vector: Option<SparseVectorInput>,
    /// Named sparse query vectors map.
    #[serde(default)]
    pub sparse_vectors: Option<BTreeMap<String, SparseVectorInput>>,
    /// Which named sparse index to search.
    #[serde(default)]
    pub sparse_index: Option<String>,
    /// Fusion configuration for hybrid search.
    #[serde(default)]
    pub fusion: Option<FusionRequest>,
}

/// Request for batch vector search.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct BatchSearchRequest {
    /// List of search requests.
    pub searches: Vec<SearchRequest>,
}

/// Request for BM25 text search.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TextSearchRequest {
    /// Text query for full-text search.
    #[cfg_attr(feature = "openapi", schema(example = "rust programming"))]
    pub query: String,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    #[cfg_attr(feature = "openapi", schema(example = 10))]
    pub top_k: usize,
    /// Optional metadata filter.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

/// Request for hybrid search (vector + text).
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct HybridSearchRequest {
    /// Query vector for similarity search.
    pub vector: Vec<f32>,
    /// Text query for BM25 search.
    #[cfg_attr(feature = "openapi", schema(example = "rust programming"))]
    pub query: String,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    #[cfg_attr(feature = "openapi", schema(example = 10))]
    pub top_k: usize,
    /// Weight for vector similarity (0.0-1.0). Text weight = 1 - `vector_weight`.
    #[serde(default = "default_vector_weight")]
    #[cfg_attr(feature = "openapi", schema(example = 0.5))]
    pub vector_weight: f32,
    /// Optional metadata filter.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

/// Request for multi-query vector search with fusion.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MultiQuerySearchRequest {
    /// List of query vectors.
    pub vectors: Vec<Vec<f32>>,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    #[cfg_attr(feature = "openapi", schema(example = 10))]
    pub top_k: usize,
    /// Fusion strategy: "average", "maximum", "rrf", "weighted", "`relative_score`".
    #[serde(default = "default_fusion_strategy")]
    #[cfg_attr(feature = "openapi", schema(example = "rrf"))]
    pub strategy: String,
    /// RRF k parameter (only used when strategy = "rrf").
    #[serde(default = "default_rrf_k")]
    #[cfg_attr(feature = "openapi", schema(example = 60))]
    pub rrf_k: u32,
    /// Weighted fusion: weight for average score component.
    #[serde(default = "default_avg_weight")]
    #[cfg_attr(feature = "openapi", schema(example = 0.5))]
    pub avg_weight: f32,
    /// Weighted fusion: weight for max score component.
    #[serde(default = "default_max_weight")]
    #[cfg_attr(feature = "openapi", schema(example = 0.3))]
    pub max_weight: f32,
    /// Weighted fusion: weight for hit count component.
    #[serde(default = "default_hit_weight")]
    #[cfg_attr(feature = "openapi", schema(example = 0.2))]
    pub hit_weight: f32,
    /// Relative score fusion: weight for the dense (vector) branch.
    #[serde(default = "default_dense_weight")]
    #[cfg_attr(feature = "openapi", schema(example = 0.5))]
    pub dense_weight: f32,
    /// Relative score fusion: weight for the sparse branch.
    #[serde(default = "default_sparse_weight")]
    #[cfg_attr(feature = "openapi", schema(example = 0.5))]
    pub sparse_weight: f32,
    /// Optional metadata filter.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

// ============================================================================
// Query Types
// ============================================================================

/// Request for `VelesQL` query execution.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct QueryRequest {
    /// The `VelesQL` query string.
    pub query: String,
    /// Named parameters for the query.
    #[serde(default)]
    pub params: std::collections::HashMap<String, serde_json::Value>,
    /// Optional collection name (required for top-level MATCH queries via `/query`).
    #[serde(default)]
    pub collection: Option<String>,
}

/// Request for query EXPLAIN.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExplainRequest {
    /// The `VelesQL` query string to explain.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "SELECT * FROM docs WHERE category = 'tech' AND vector NEAR $v LIMIT 10")
    )]
    pub query: String,
    /// Named parameters for the query.
    #[serde(default)]
    pub params: std::collections::HashMap<String, serde_json::Value>,
    /// When true, execute the query and return actual statistics alongside the plan.
    #[serde(default)]
    pub analyze: bool,
}

// ============================================================================
// Index Management Types
// ============================================================================

/// Request to create a property index.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateIndexRequest {
    /// Node label to index.
    #[cfg_attr(feature = "openapi", schema(example = "Person"))]
    pub label: String,
    /// Property name to index.
    #[cfg_attr(feature = "openapi", schema(example = "email"))]
    pub property: String,
    /// Index type: "hash" (equality O(1)) or "range" (range queries O(log n)).
    #[serde(default = "default_index_type")]
    #[cfg_attr(feature = "openapi", schema(example = "hash"))]
    pub index_type: String,
}

// ============================================================================
// GuardRails Types
// ============================================================================

/// Request to configure query guard-rails.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GuardRailsConfigRequest {
    /// Maximum graph traversal depth.
    #[cfg_attr(feature = "openapi", schema(example = 10))]
    pub max_depth: Option<u32>,
    /// Maximum intermediate cardinality.
    #[cfg_attr(feature = "openapi", schema(example = 100_000))]
    pub max_cardinality: Option<usize>,
    /// Memory limit per query in bytes.
    #[cfg_attr(feature = "openapi", schema(example = 104_857_600))]
    pub memory_limit_bytes: Option<usize>,
    /// Query timeout in milliseconds (0 = no timeout).
    #[cfg_attr(feature = "openapi", schema(example = 30000))]
    pub timeout_ms: Option<u64>,
    /// Rate limit: max queries per second per client.
    #[cfg_attr(feature = "openapi", schema(example = 100))]
    pub rate_limit_qps: Option<u32>,
    /// Circuit breaker: failure threshold before tripping.
    #[cfg_attr(feature = "openapi", schema(example = 5))]
    pub circuit_failure_threshold: Option<u32>,
    /// Circuit breaker: recovery time in seconds.
    #[cfg_attr(feature = "openapi", schema(example = 30))]
    pub circuit_recovery_seconds: Option<u64>,
}
