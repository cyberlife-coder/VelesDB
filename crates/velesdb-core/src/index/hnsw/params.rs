//! HNSW index parameters and search quality profiles.
//!
//! This module contains configuration types for tuning HNSW index
//! performance and search quality.

use serde::{Deserialize, Serialize};

/// HNSW index parameters for tuning performance and recall.
///
/// Use [`HnswParams::auto`] for automatic tuning based on vector dimension,
/// or create custom parameters for specific workloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HnswParams {
    /// Number of bi-directional links per node (M parameter).
    /// Higher = better recall, more memory, slower insert.
    pub max_connections: usize,
    /// Size of dynamic candidate list during construction.
    /// Higher = better recall, slower indexing.
    pub ef_construction: usize,
    /// Initial capacity (grows automatically if exceeded).
    pub max_elements: usize,
}

impl Default for HnswParams {
    fn default() -> Self {
        Self::auto(768)
    }
}

impl HnswParams {
    /// Creates optimized parameters based on vector dimension.
    #[must_use]
    pub fn auto(dimension: usize) -> Self {
        match dimension {
            0..=768 => Self {
                max_connections: 16,
                ef_construction: 200,
                max_elements: 100_000,
            },
            _ => Self {
                max_connections: 24,
                ef_construction: 300,
                max_elements: 100_000,
            },
        }
    }

    /// Creates fast parameters optimized for insertion speed.
    #[must_use]
    pub fn fast() -> Self {
        Self {
            max_connections: 16,
            ef_construction: 200,
            max_elements: 100_000,
        }
    }

    /// Creates parameters optimized for high recall.
    #[must_use]
    pub fn high_recall(dimension: usize) -> Self {
        let base = Self::auto(dimension);
        Self {
            max_connections: base.max_connections + 8,
            ef_construction: base.ef_construction + 200,
            ..base
        }
    }

    /// Creates parameters optimized for maximum recall.
    #[must_use]
    pub fn max_recall(dimension: usize) -> Self {
        match dimension {
            0..=256 => Self {
                max_connections: 32,
                ef_construction: 500,
                max_elements: 100_000,
            },
            257..=768 => Self {
                max_connections: 48,
                ef_construction: 800,
                max_elements: 100_000,
            },
            _ => Self {
                max_connections: 64,
                ef_construction: 1000,
                max_elements: 100_000,
            },
        }
    }

    /// Creates parameters optimized for fast indexing.
    #[must_use]
    pub fn fast_indexing(dimension: usize) -> Self {
        let base = Self::auto(dimension);
        Self {
            max_connections: (base.max_connections / 2).max(8),
            ef_construction: base.ef_construction / 2,
            ..base
        }
    }

    /// Creates custom parameters.
    #[must_use]
    pub const fn custom(
        max_connections: usize,
        ef_construction: usize,
        max_elements: usize,
    ) -> Self {
        Self {
            max_connections,
            ef_construction,
            max_elements,
        }
    }
}

/// Search quality profile controlling the recall/latency tradeoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SearchQuality {
    /// Fast search with `ef_search=64`.
    Fast,
    /// Balanced search with `ef_search=128`.
    #[default]
    Balanced,
    /// Accurate search with `ef_search=256`.
    Accurate,
    /// High recall search with `ef_search=512`.
    HighRecall,
    /// Custom `ef_search` value.
    Custom(usize),
}

impl SearchQuality {
    /// Returns the `ef_search` value for this quality profile.
    #[must_use]
    pub fn ef_search(&self, k: usize) -> usize {
        match self {
            Self::Fast => 64.max(k * 2),
            Self::Balanced => 128.max(k * 4),
            Self::Accurate => 256.max(k * 8),
            Self::HighRecall => 512.max(k * 16),
            Self::Custom(ef) => (*ef).max(k),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hnsw_params_default() {
        let params = HnswParams::default();
        assert_eq!(params.max_connections, 16);
        assert_eq!(params.ef_construction, 200);
    }

    #[test]
    fn test_hnsw_params_auto_small_dimension() {
        let params = HnswParams::auto(128);
        assert_eq!(params.max_connections, 16);
    }

    #[test]
    fn test_hnsw_params_auto_large_dimension() {
        let params = HnswParams::auto(1024);
        assert_eq!(params.max_connections, 24);
    }

    #[test]
    fn test_hnsw_params_fast() {
        let params = HnswParams::fast();
        assert_eq!(params.max_connections, 16);
        assert_eq!(params.ef_construction, 200);
        assert_eq!(params.max_elements, 100_000);
    }

    #[test]
    fn test_hnsw_params_high_recall() {
        let params = HnswParams::high_recall(768);
        assert_eq!(params.max_connections, 24); // 16 + 8
        assert_eq!(params.ef_construction, 400); // 200 + 200
    }

    #[test]
    fn test_hnsw_params_max_recall_small() {
        let params = HnswParams::max_recall(128);
        assert_eq!(params.max_connections, 32);
        assert_eq!(params.ef_construction, 500);
    }

    #[test]
    fn test_hnsw_params_max_recall_medium() {
        let params = HnswParams::max_recall(512);
        assert_eq!(params.max_connections, 48);
        assert_eq!(params.ef_construction, 800);
    }

    #[test]
    fn test_hnsw_params_max_recall_large() {
        let params = HnswParams::max_recall(1024);
        assert_eq!(params.max_connections, 64);
        assert_eq!(params.ef_construction, 1000);
    }

    #[test]
    fn test_hnsw_params_fast_indexing() {
        let params = HnswParams::fast_indexing(768);
        assert_eq!(params.max_connections, 8); // 16 / 2
        assert_eq!(params.ef_construction, 100); // 200 / 2
    }

    #[test]
    fn test_hnsw_params_custom() {
        let params = HnswParams::custom(32, 400, 50_000);
        assert_eq!(params.max_connections, 32);
        assert_eq!(params.ef_construction, 400);
        assert_eq!(params.max_elements, 50_000);
    }

    #[test]
    fn test_search_quality_ef_search() {
        assert_eq!(SearchQuality::Fast.ef_search(10), 64);
        assert_eq!(SearchQuality::Balanced.ef_search(10), 128);
        assert_eq!(SearchQuality::Accurate.ef_search(10), 256);
        assert_eq!(SearchQuality::Custom(50).ef_search(10), 50);
    }

    #[test]
    fn test_search_quality_ef_search_high_k() {
        // Test that ef_search scales with k
        assert_eq!(SearchQuality::Fast.ef_search(100), 200); // 100 * 2
        assert_eq!(SearchQuality::Balanced.ef_search(50), 200); // 50 * 4
        assert_eq!(SearchQuality::Accurate.ef_search(40), 320); // 40 * 8
        assert_eq!(SearchQuality::HighRecall.ef_search(10), 512);
        assert_eq!(SearchQuality::HighRecall.ef_search(50), 800); // 50 * 16
    }

    #[test]
    fn test_search_quality_default() {
        let quality = SearchQuality::default();
        assert_eq!(quality, SearchQuality::Balanced);
    }

    #[test]
    fn test_hnsw_params_serialize_deserialize() {
        let params = HnswParams::custom(32, 400, 50_000);
        let json = serde_json::to_string(&params).unwrap();
        let deserialized: HnswParams = serde_json::from_str(&json).unwrap();
        assert_eq!(params, deserialized);
    }

    #[test]
    fn test_search_quality_serialize_deserialize() {
        let quality = SearchQuality::Custom(100);
        let json = serde_json::to_string(&quality).unwrap();
        let deserialized: SearchQuality = serde_json::from_str(&json).unwrap();
        assert_eq!(quality, deserialized);
    }
}
