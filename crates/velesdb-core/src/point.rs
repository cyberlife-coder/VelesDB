//! Point data structure representing a vector with metadata.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::sparse_index::{SparseVector, DEFAULT_SPARSE_INDEX_NAME};

/// A point in the vector database.
///
/// A point consists of:
/// - A unique identifier
/// - A dense vector (embedding)
/// - Optional payload (metadata)
/// - Optional named sparse vectors (e.g., SPLADE, BM25 term weights)
#[derive(Debug, Clone, Serialize)]
pub struct Point {
    /// Unique identifier for the point.
    pub id: u64,

    /// The dense vector embedding.
    pub vector: Vec<f32>,

    /// Optional JSON payload containing metadata.
    #[serde(default)]
    pub payload: Option<JsonValue>,

    /// Optional named sparse vectors for hybrid dense+sparse search.
    ///
    /// Keys are sparse vector names (e.g., `""` for default, `"title"`, `"body"`).
    /// Enables multi-model support (BGE-M3, SPLADE title+body).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sparse_vectors: Option<BTreeMap<String, SparseVector>>,
}

/// Custom deserializer that accepts both:
/// - `"sparse_vectors": {"name": {...}}` (new named map format)
/// - `"sparse_vector": {...}` (old single format, wraps in map under `""` key)
impl<'de> Deserialize<'de> for Point {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct PointHelper {
            id: u64,
            vector: Vec<f32>,
            #[serde(default)]
            payload: Option<JsonValue>,
            #[serde(default)]
            sparse_vectors: Option<BTreeMap<String, SparseVector>>,
            /// Old single-vector field for backward compat.
            #[serde(default)]
            sparse_vector: Option<SparseVector>,
        }

        let helper = PointHelper::deserialize(deserializer)?;

        // Prefer new `sparse_vectors` field; fall back to old `sparse_vector`.
        let sparse_vectors = if helper.sparse_vectors.is_some() {
            helper.sparse_vectors
        } else {
            helper.sparse_vector.map(|sv| {
                let mut map = BTreeMap::new();
                // Use the canonical constant to avoid magic empty-string literals.
                map.insert(DEFAULT_SPARSE_INDEX_NAME.to_string(), sv);
                map
            })
        };

        Ok(Point {
            id: helper.id,
            vector: helper.vector,
            payload: helper.payload,
            sparse_vectors,
        })
    }
}

impl Point {
    /// Creates a new point with the given ID, vector, and optional payload.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier
    /// * `vector` - Vector embedding
    /// * `payload` - Optional metadata
    #[must_use]
    pub fn new(id: u64, vector: Vec<f32>, payload: Option<JsonValue>) -> Self {
        Self {
            id,
            vector,
            payload,
            sparse_vectors: None,
        }
    }

    /// Creates a new point without payload.
    #[must_use]
    pub fn without_payload(id: u64, vector: Vec<f32>) -> Self {
        Self::new(id, vector, None)
    }

    /// Creates a metadata-only point (no vector, only payload).
    ///
    /// Used for metadata-only collections that don't store vectors.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier
    /// * `payload` - Metadata (JSON value)
    #[must_use]
    pub fn metadata_only(id: u64, payload: JsonValue) -> Self {
        Self {
            id,
            vector: Vec::new(), // Empty vector
            payload: Some(payload),
            sparse_vectors: None,
        }
    }

    /// Creates a point with both dense and named sparse vectors.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier
    /// * `vector` - Dense vector embedding
    /// * `payload` - Optional metadata
    /// * `sparse_vectors` - Optional named sparse vectors
    #[must_use]
    pub fn with_sparse(
        id: u64,
        vector: Vec<f32>,
        payload: Option<JsonValue>,
        sparse_vectors: Option<BTreeMap<String, SparseVector>>,
    ) -> Self {
        Self {
            id,
            vector,
            payload,
            sparse_vectors,
        }
    }

    /// Creates a sparse-only point (no dense vector).
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier
    /// * `sparse_vector` - The sparse vector (stored under the default `""` name)
    /// * `payload` - Optional metadata
    #[must_use]
    pub fn sparse_only(id: u64, sparse_vector: SparseVector, payload: Option<JsonValue>) -> Self {
        let mut map = BTreeMap::new();
        // Use the canonical constant to avoid magic empty-string literals.
        map.insert(DEFAULT_SPARSE_INDEX_NAME.to_string(), sparse_vector);
        Self {
            id,
            vector: Vec::new(),
            payload,
            sparse_vectors: Some(map),
        }
    }

    /// Returns `true` if this point has any sparse vectors.
    #[must_use]
    pub fn has_sparse_vectors(&self) -> bool {
        self.sparse_vectors.as_ref().is_some_and(|m| !m.is_empty())
    }

    /// Returns the dimension of the vector.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.vector.len()
    }

    /// Returns true if this point has no vector (metadata-only).
    #[must_use]
    pub fn is_metadata_only(&self) -> bool {
        self.vector.is_empty()
    }
}

/// Per-component score breakdown for hybrid search results.
///
/// Stores individual scores from each search pipeline component (vector,
/// BM25, graph, sparse) so arithmetic ORDER BY expressions like
/// `0.7 * vector_score + 0.3 * bm25_score` can resolve each variable
/// independently instead of mapping everything to the fused score.
///
/// Uses `SmallVec<4>` to avoid heap allocation for typical queries
/// (at most 2-3 components: vector, text, sparse).
pub type ComponentScores = smallvec::SmallVec<[(String, f32); 4]>;

/// A search result containing a point and its similarity score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The matching point.
    pub point: Point,

    /// Similarity score (interpretation depends on the distance metric).
    pub score: f32,

    /// Optional per-component score breakdown for arithmetic ORDER BY.
    ///
    /// When present, variables like `vector_score` and `bm25_score` resolve
    /// to their individual component values. When `None`, all score variables
    /// fall back to `score` (the fused/primary score).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_scores: Option<ComponentScores>,
}

impl SearchResult {
    /// Creates a new search result with no component score breakdown.
    #[must_use]
    pub fn new(point: Point, score: f32) -> Self {
        Self {
            point,
            score,
            component_scores: None,
        }
    }

    /// Creates a search result with per-component score breakdown.
    #[must_use]
    pub fn with_component_scores(
        point: Point,
        score: f32,
        component_scores: ComponentScores,
    ) -> Self {
        Self {
            point,
            score,
            component_scores: if component_scores.is_empty() {
                None
            } else {
                Some(component_scores)
            },
        }
    }
}
