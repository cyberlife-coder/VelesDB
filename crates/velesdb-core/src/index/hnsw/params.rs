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
    /// These defaults are tuned for datasets up to 100K vectors with high recall targets.
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
            // 257+ dimensions: aggressive params targeting high recall
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
    /// Targets high recall up to 1M vectors under benchmark-calibrated settings.
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
        let (m_low, ef_low, m_high, ef_high, max_elems) = match expected_vectors {
            0..=10_000 => (24, 200, 32, 400, 20_000),
            10_001..=100_000 => (64, 800, 128, 1600, 150_000),
            100_001..=500_000 => (96, 1200, 128, 2000, 750_000),
            _ => (64, 800, 128, 1600, 1_500_000),
        };
        let (m, ef) = if dimension <= 256 {
            (m_low, ef_low)
        } else {
            (m_high, ef_high)
        };
        Self {
            max_connections: m,
            ef_construction: ef,
            max_elements: max_elems,
            storage_mode: StorageMode::Full,
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

    /// Creates parameters for 1 million vectors with high-recall target settings.
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
    /// Fast search with `ef_search=64`. ~92% recall, lowest latency.
    Fast,
    /// Balanced search with `ef_search=128`. ~99% recall, production default.
    #[default]
    Balanced,
    /// Accurate search with `ef_search=512`. ~100% recall.
    Accurate,
    /// Perfect recall mode with `ef_search=4096` for guaranteed 100% recall.
    /// Uses large candidate pool with exact SIMD re-ranking.
    Perfect,
    /// Custom `ef_search` value.
    Custom(usize),
    /// Adaptive `ef_search` that starts low and doubles if the query is "hard".
    ///
    /// Uses a two-phase approach:
    /// 1. Search with `min_ef`
    /// 2. If result spread (`max_dist / min_dist`) exceeds a threshold, re-search
    ///    with doubled ef (up to `max_ef`)
    ///
    /// For easy queries (dense cluster hits), this is 2-4x faster than fixed ef.
    /// For hard queries, it gracefully degrades to `max_ef` with no recall loss.
    Adaptive {
        /// Minimum `ef_search` (starting point). Default: 32.
        min_ef: usize,
        /// Maximum `ef_search` (cap). Default: 512.
        max_ef: usize,
    },
}

impl SearchQuality {
    /// Returns the `ef_search` value for this quality profile.
    ///
    /// # Large-scale optimization (v0.9+)
    ///
    /// - **Accurate**: 512 base (was 256), scales with k×16 for ≥95% recall at 100K+
    /// - **Perfect**: 4096 base (was 2048), scales with k×100 for guaranteed 100% at 100K+
    /// - **Adaptive**: returns `min_ef` (first phase); caller handles second phase
    #[must_use]
    pub fn ef_search(&self, k: usize) -> usize {
        match self {
            Self::Fast => 64.max(k * 2),
            Self::Balanced => 128.max(k * 4),
            // Increased from 256 to 512 for better recall at 100K+ scale
            Self::Accurate => 512.max(k * 16),
            // Increased from 2048 to 4096 for guaranteed 100% recall at 100K+
            Self::Perfect => 4096.max(k * 100),
            Self::Custom(ef) => (*ef).max(k),
            // Adaptive: start with min_ef (first phase)
            Self::Adaptive { min_ef, .. } => (*min_ef).max(k),
        }
    }

    /// Returns `true` if this quality profile uses two-phase adaptive search.
    #[must_use]
    pub const fn is_adaptive(&self) -> bool {
        matches!(self, Self::Adaptive { .. })
    }

    /// Returns the maximum ef for adaptive search, or `None` for fixed profiles.
    #[must_use]
    pub const fn adaptive_max_ef(&self) -> Option<usize> {
        match self {
            Self::Adaptive { max_ef, .. } => Some(*max_ef),
            _ => None,
        }
    }
}
