//! Collection configuration and schema versioning.

use crate::collection::streaming::AsyncIndexBuilderConfig;
use crate::distance::DistanceMetric;
use crate::index::hnsw::HnswParams;
use crate::quantization::StorageMode;
use serde::{Deserialize, Serialize};

use crate::collection::graph::GraphSchema;

/// Current on-disk schema version for `config.json`.
///
/// Increment this constant when the persisted format changes in a way that
/// older VelesDB versions cannot safely read. The `Collection::open()` path
/// rejects any `schema_version > CURRENT_SCHEMA_VERSION` with a clear error.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Returns the default schema version for backward-compatible deserialization.
///
/// Old `config.json` files written before schema versioning was introduced
/// will deserialize with this default, which is equivalent to version 1.
fn default_schema_version() -> u32 {
    1
}

/// Returns `Some(4)` as the default PQ rescore oversampling factor.
/// Returns `Option` because the field type is `Option<u32>` (None = disabled).
#[allow(clippy::unnecessary_wraps)]
fn default_pq_rescore_oversampling() -> Option<u32> {
    Some(4)
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

    /// On-disk schema version for forward-compatibility detection.
    ///
    /// When a newer VelesDB version writes a `config.json` with a higher
    /// schema version, older versions will refuse to open the collection
    /// rather than silently corrupting data.
    ///
    /// Backward compatible: old `config.json` files without this field
    /// deserialize to `1` (the initial version).
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

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

    /// Custom HNSW index parameters (M, `ef_construction`, etc.).
    ///
    /// When `Some`, these parameters are used to rebuild the HNSW index on
    /// collection reopen if `hnsw.bin` does not yet exist (empty collection).
    /// When `None`, the default `HnswParams::auto(dimension)` is used.
    ///
    /// Backward compatible: old `config.json` files without this field
    /// deserialize to `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hnsw_params: Option<HnswParams>,

    /// Deferred indexing configuration (US-366).
    ///
    /// When `Some` and `enabled`, inserts are buffered in memory and
    /// batch-merged into the HNSW index when the buffer reaches
    /// `merge_threshold`. This decouples write latency from index cost.
    ///
    /// Backward compatible: old `config.json` files without this field
    /// deserialize to `None` (disabled).
    #[cfg(feature = "persistence")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred_indexing: Option<crate::collection::streaming::DeferredIndexerConfig>,

    /// Async index builder configuration (Issue #488 — Bulk Insert V2).
    ///
    /// When `Some`, enables the `AsyncIndexBuilder` for deferred HNSW
    /// construction during bulk insert. Vectors are buffered and indexed
    /// asynchronously via `HnswSegmentBuilder`.
    ///
    /// Backward compatible: old `config.json` files without this field
    /// deserialize to `None` (disabled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub async_index_builder: Option<AsyncIndexBuilderConfig>,
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
            schema_version: CURRENT_SCHEMA_VERSION,
            storage_mode: StorageMode::ProductQuantization,
            metadata_only: false,
            graph_schema: None,
            embedding_dimension: None,
            pq_rescore_oversampling: oversampling,
            hnsw_params: None,
            #[cfg(feature = "persistence")]
            deferred_indexing: None,
            async_index_builder: None,
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
