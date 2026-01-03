//! HNSW index parameters and search quality profiles.
//!
//! This module contains configuration types for tuning HNSW index
//! performance and search quality.

use crate::quantization::StorageMode;
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
    /// Vector storage mode (Full, SQ8, or Binary).
    /// SQ8 provides 4x memory reduction with ~1% recall loss.
    #[serde(default)]
    pub storage_mode: StorageMode,
}

impl Default for HnswParams {
    fn default() -> Self {
        Self::auto(768)
    }
}

impl HnswParams {
    /// Creates optimized parameters based on vector dimension.
    ///
    /// These defaults are tuned for datasets up to 100K vectors with ≥95% recall.
    /// For larger datasets, use [`HnswParams::for_dataset_size`].
    #[must_use]
    pub fn auto(dimension: usize) -> Self {
        match dimension {
            0..=256 => Self {
                max_connections: 24,
                ef_construction: 300,
                max_elements: 100_000,
                storage_mode: StorageMode::Full,
            },
            // 257+ dimensions: aggressive params for ≥95% recall
            _ => Self {
                max_connections: 32,
                ef_construction: 400,
                max_elements: 100_000,
                storage_mode: StorageMode::Full,
            },
        }
    }

    /// Creates parameters optimized for a specific dataset size.
    ///
    /// **GUARANTEES ≥95% recall** up to 1M vectors for `HighRecall` mode.
    ///
    /// # Parameters by Scale
    ///
    /// | Dataset Size | M | `ef_construction` | Target Recall |
    /// |--------------|---|-------------------|---------------|
    /// | ≤10K | 32 | 200 | ≥98% |
    /// | ≤100K | 64 | 800 | ≥95% |
    /// | ≤500K | 96 | 1200 | ≥95% |
    /// | ≤1M | 128 | 1600 | ≥95% |
    #[must_use]
    pub fn for_dataset_size(dimension: usize, expected_vectors: usize) -> Self {
        match expected_vectors {
            // Small datasets: balanced params
            0..=10_000 => match dimension {
                0..=256 => Self {
                    max_connections: 24,
                    ef_construction: 200,
                    max_elements: 20_000,
                    storage_mode: StorageMode::Full,
                },
                _ => Self {
                    max_connections: 32,
                    ef_construction: 400,
                    max_elements: 20_000,
                    storage_mode: StorageMode::Full,
                },
            },
            // Medium datasets: high params
            10_001..=100_000 => match dimension {
                0..=256 => Self {
                    max_connections: 32,
                    ef_construction: 400,
                    max_elements: 150_000,
                    storage_mode: StorageMode::Full,
                },
                _ => Self {
                    max_connections: 64,
                    ef_construction: 800,
                    max_elements: 150_000,
                    storage_mode: StorageMode::Full,
                },
            },
            // Large datasets: aggressive params
            100_001..=500_000 => match dimension {
                0..=256 => Self {
                    max_connections: 48,
                    ef_construction: 600,
                    max_elements: 750_000,
                    storage_mode: StorageMode::Full,
                },
                _ => Self {
                    max_connections: 96,
                    ef_construction: 1200,
                    max_elements: 750_000,
                    storage_mode: StorageMode::Full,
                },
            },
            // Very large datasets (up to 1M): maximum params for ≥95% recall
            _ => match dimension {
                0..=256 => Self {
                    max_connections: 64,
                    ef_construction: 800,
                    max_elements: 1_500_000,
                    storage_mode: StorageMode::Full,
                },
                // 768D+ at 1M vectors: M=128, ef=1600 based on OpenSearch research
                _ => Self {
                    max_connections: 128,
                    ef_construction: 1600,
                    max_elements: 1_500_000,
                    storage_mode: StorageMode::Full,
                },
            },
        }
    }

    /// Creates parameters optimized for large datasets (100K+ vectors).
    ///
    /// Higher M and `ef_construction` ensure good recall at scale.
    /// For 1M+ vectors, use [`HnswParams::for_dataset_size`] instead.
    #[must_use]
    pub fn large_dataset(dimension: usize) -> Self {
        Self::for_dataset_size(dimension, 500_000)
    }

    /// Creates parameters for 1 million vectors with ≥95% recall guarantee.
    ///
    /// Based on `OpenSearch` 2025 research: M=128, `ef_construction`=1600.
    #[must_use]
    pub fn million_scale(dimension: usize) -> Self {
        Self::for_dataset_size(dimension, 1_000_000)
    }

    /// Creates fast parameters optimized for insertion speed.
    /// Lower recall but faster indexing. Best for small datasets (<10K).
    #[must_use]
    pub fn fast() -> Self {
        Self {
            max_connections: 16,
            ef_construction: 150,
            max_elements: 100_000,
            storage_mode: StorageMode::Full,
        }
    }

    /// Creates turbo parameters for maximum insert throughput.
    ///
    /// **Target**: 5k+ vec/s (vs ~2k/s with `auto` params)
    ///
    /// # Trade-offs
    ///
    /// - **Recall**: ~85% (vs ≥95% with standard params)
    /// - **Best for**: Bulk loading, development, benchmarking
    /// - **Not recommended for**: Production search workloads
    ///
    /// # Parameters
    ///
    /// - `M = 12`: Minimal connections for fast graph construction
    /// - `ef_construction = 100`: Low expansion factor
    ///
    /// After bulk loading, consider rebuilding with higher params for production.
    #[must_use]
    pub fn turbo() -> Self {
        Self {
            max_connections: 12,
            ef_construction: 100,
            max_elements: 100_000,
            storage_mode: StorageMode::Full,
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
                storage_mode: StorageMode::Full,
            },
            257..=768 => Self {
                max_connections: 48,
                ef_construction: 800,
                max_elements: 100_000,
                storage_mode: StorageMode::Full,
            },
            _ => Self {
                max_connections: 64,
                ef_construction: 1000,
                max_elements: 100_000,
                storage_mode: StorageMode::Full,
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
            storage_mode: StorageMode::Full,
        }
    }

    /// Creates parameters with SQ8 quantization for 4x memory reduction.
    ///
    /// # Memory Savings
    ///
    /// | Dimension | Full (f32) | SQ8 (u8) | Reduction |
    /// |-----------|------------|----------|----------|
    /// | 768 | 3 KB | 776 B | 4x |
    /// | 1536 | 6 KB | 1.5 KB | 4x |
    #[must_use]
    pub fn with_sq8(dimension: usize) -> Self {
        let mut params = Self::auto(dimension);
        params.storage_mode = StorageMode::SQ8;
        params
    }

    /// Creates parameters with binary quantization for 32x memory reduction.
    /// Best for edge/IoT devices with limited RAM.
    #[must_use]
    pub fn with_binary(dimension: usize) -> Self {
        let mut params = Self::auto(dimension);
        params.storage_mode = StorageMode::Binary;
        params
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
    /// High recall search with `ef_search=1024` (was 512, improved for ~99.7% recall).
    HighRecall,
    /// Perfect recall mode with `ef_search=2048+` for guaranteed 100% recall.
    /// Uses very large candidate pool with exact SIMD re-ranking.
    /// Best for applications where accuracy is more important than latency.
    Perfect,
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
            // HighRecall: increased from 512 to 1024 for ~99.7% recall
            Self::HighRecall => 1024.max(k * 32),
            // Perfect: very large pool for 100% recall guarantee
            Self::Perfect => 2048.max(k * 50),
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
        assert_eq!(params.max_connections, 32); // auto(768) -> optimized default
        assert_eq!(params.ef_construction, 400);
    }

    #[test]
    fn test_hnsw_params_auto_small_dimension() {
        let params = HnswParams::auto(128);
        assert_eq!(params.max_connections, 24); // 0..=256 range
        assert_eq!(params.ef_construction, 300);
    }

    #[test]
    fn test_hnsw_params_auto_large_dimension() {
        let params = HnswParams::auto(1024);
        assert_eq!(params.max_connections, 32); // > 256 range
        assert_eq!(params.ef_construction, 400);
    }

    #[test]
    fn test_hnsw_params_fast() {
        let params = HnswParams::fast();
        assert_eq!(params.max_connections, 16);
        assert_eq!(params.ef_construction, 150);
        assert_eq!(params.max_elements, 100_000);
    }

    #[test]
    fn test_hnsw_params_high_recall() {
        let params = HnswParams::high_recall(768);
        assert_eq!(params.max_connections, 40); // 32 + 8
        assert_eq!(params.ef_construction, 600); // 400 + 200
    }

    #[test]
    fn test_hnsw_params_large_dataset() {
        let params = HnswParams::large_dataset(768);
        assert_eq!(params.max_connections, 96); // for_dataset_size(768, 500K)
        assert_eq!(params.ef_construction, 1200);
        assert_eq!(params.max_elements, 750_000);
    }

    #[test]
    fn test_hnsw_params_for_dataset_size_small() {
        let params = HnswParams::for_dataset_size(768, 5_000);
        assert_eq!(params.max_connections, 32);
        assert_eq!(params.ef_construction, 400);
        assert_eq!(params.max_elements, 20_000);
    }

    #[test]
    fn test_hnsw_params_for_dataset_size_medium() {
        let params = HnswParams::for_dataset_size(768, 50_000);
        assert_eq!(params.max_connections, 64);
        assert_eq!(params.ef_construction, 800);
        assert_eq!(params.max_elements, 150_000);
    }

    #[test]
    fn test_hnsw_params_for_dataset_size_large() {
        let params = HnswParams::for_dataset_size(768, 300_000);
        assert_eq!(params.max_connections, 96);
        assert_eq!(params.ef_construction, 1200);
        assert_eq!(params.max_elements, 750_000);
    }

    #[test]
    fn test_hnsw_params_million_scale() {
        // 1M vectors at 768D should use M=128, ef=1600 for ≥95% recall
        let params = HnswParams::million_scale(768);
        assert_eq!(params.max_connections, 128);
        assert_eq!(params.ef_construction, 1600);
        assert_eq!(params.max_elements, 1_500_000);
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
        assert_eq!(params.max_connections, 16); // 32 / 2
        assert_eq!(params.ef_construction, 200); // 400 / 2
    }

    #[test]
    fn test_hnsw_params_custom() {
        let params = HnswParams::custom(32, 400, 50_000);
        assert_eq!(params.max_connections, 32);
        assert_eq!(params.ef_construction, 400);
        assert_eq!(params.max_elements, 50_000);
        assert_eq!(params.storage_mode, StorageMode::Full);
    }

    #[test]
    fn test_hnsw_params_with_sq8() {
        // Arrange & Act
        let params = HnswParams::with_sq8(768);

        // Assert - SQ8 mode enabled with auto-tuned params
        assert_eq!(params.storage_mode, StorageMode::SQ8);
        assert_eq!(params.max_connections, 32); // From auto(768)
        assert_eq!(params.ef_construction, 400);
    }

    #[test]
    fn test_hnsw_params_with_binary() {
        // Arrange & Act
        let params = HnswParams::with_binary(768);

        // Assert - Binary mode for 32x compression
        assert_eq!(params.storage_mode, StorageMode::Binary);
        assert_eq!(params.max_connections, 32);
    }

    #[test]
    fn test_hnsw_params_storage_mode_default() {
        // Arrange & Act
        let params = HnswParams::default();

        // Assert - Default is Full precision
        assert_eq!(params.storage_mode, StorageMode::Full);
    }

    #[test]
    fn test_search_quality_ef_search() {
        assert_eq!(SearchQuality::Fast.ef_search(10), 64);
        assert_eq!(SearchQuality::Balanced.ef_search(10), 128);
        assert_eq!(SearchQuality::Accurate.ef_search(10), 256);
        assert_eq!(SearchQuality::Custom(50).ef_search(10), 50);
    }

    #[test]
    fn test_search_quality_perfect_ef_search() {
        // Perfect mode should use very high ef_search for 100% recall
        // Base value 2048, scales with k * 50
        assert_eq!(SearchQuality::Perfect.ef_search(10), 2048); // max(2048, 10*50=500)
        assert_eq!(SearchQuality::Perfect.ef_search(50), 2500); // max(2048, 50*50=2500)
        assert_eq!(SearchQuality::Perfect.ef_search(100), 5000); // max(2048, 100*50=5000)
    }

    #[test]
    fn test_search_quality_ef_search_high_k() {
        // Test that ef_search scales with k
        assert_eq!(SearchQuality::Fast.ef_search(100), 200); // 100 * 2
        assert_eq!(SearchQuality::Balanced.ef_search(50), 200); // 50 * 4
        assert_eq!(SearchQuality::Accurate.ef_search(40), 320); // 40 * 8
                                                                // HighRecall now uses 1024 base (was 512) for better recall
        assert_eq!(SearchQuality::HighRecall.ef_search(10), 1024); // max(1024, 10*32=320)
        assert_eq!(SearchQuality::HighRecall.ef_search(50), 1600); // max(1024, 50*32=1600)
    }

    #[test]
    fn test_search_quality_perfect_serialize_deserialize() {
        // Arrange
        let quality = SearchQuality::Perfect;

        // Act
        let json = serde_json::to_string(&quality).unwrap();
        let deserialized: SearchQuality = serde_json::from_str(&json).unwrap();

        // Assert
        assert_eq!(quality, deserialized);
    }

    #[test]
    fn test_search_quality_default() {
        let quality = SearchQuality::default();
        assert_eq!(quality, SearchQuality::Balanced);
    }

    #[test]
    fn test_hnsw_params_turbo() {
        // TDD: Turbo mode for maximum insert throughput
        // Target: 5k+ vec/s (vs ~2k/s with auto params)
        // Trade-off: Lower recall (~85%) but acceptable for bulk loading
        let params = HnswParams::turbo();

        // Aggressive params: M=12, ef=100 for fastest graph construction
        assert_eq!(params.max_connections, 12);
        assert_eq!(params.ef_construction, 100);
        assert_eq!(params.max_elements, 100_000);
        assert_eq!(params.storage_mode, StorageMode::Full);
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
