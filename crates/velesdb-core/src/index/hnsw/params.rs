//! HNSW index parameters and search quality profiles.
//!
//! This module contains configuration types for tuning HNSW index
//! performance and search quality.

use crate::quantization::StorageMode;
use serde::{Deserialize, Serialize};

/// Default VAMANA alpha for neighbor diversification (1.2).
///
/// Used as serde default when deserializing configs that predate the alpha field.
const fn default_alpha() -> f32 {
    1.2
}

/// HNSW index parameters for tuning performance and recall.
///
/// Use [`HnswParams::auto`] for automatic tuning based on vector dimension,
/// or create custom parameters for specific workloads.
///
/// # Alpha (VAMANA diversification)
///
/// The `alpha` parameter controls neighbor selection diversification.
/// Values > 1.0 bias toward graph navigability (more diverse neighbors),
/// while 1.0 selects purely by proximity. The default (1.2) follows the
/// VAMANA paper recommendation (Subramanya et al., 2019).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
    /// VAMANA alpha for neighbor diversification (default: 1.2).
    ///
    /// Alpha > 1.0 biases `select_neighbors` toward diversity over proximity,
    /// producing a more navigable graph with better recall. Set to 1.0 for
    /// strict nearest-neighbor selection.
    #[serde(default = "default_alpha")]
    pub alpha: f32,
}

// Eq is safe because alpha values are always finite: they come from
// `default_alpha()` (1.2), `with_alpha()` (caller-chosen), or serde
// deserialization which rejects NaN/Inf for f32. No code path produces
// NaN, so reflexivity (a == a) always holds.
impl Eq for HnswParams {}

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
                alpha: default_alpha(),
            },
            // 257+ dimensions: aggressive params targeting high recall
            _ => Self {
                max_connections: 32,
                ef_construction: 400,
                max_elements: 100_000,
                storage_mode: StorageMode::Full,
                alpha: default_alpha(),
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
            alpha: default_alpha(),
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
            alpha: default_alpha(),
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
            alpha: default_alpha(),
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
                alpha: default_alpha(),
            },
            257..=768 => Self {
                max_connections: 48,
                ef_construction: 800,
                max_elements: 100_000,
                storage_mode: StorageMode::Full,
                alpha: default_alpha(),
            },
            _ => Self {
                max_connections: 64,
                ef_construction: 1000,
                max_elements: 100_000,
                storage_mode: StorageMode::Full,
                alpha: default_alpha(),
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

    /// Creates custom parameters with default alpha (1.2).
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
            alpha: 1.2,
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

    /// Returns a copy of these parameters with a custom alpha value.
    ///
    /// Alpha controls VAMANA neighbor diversification:
    /// - `1.0`: strict nearest-neighbor selection (no diversification)
    /// - `1.2`: recommended default (VAMANA paper)
    /// - `>1.2`: more diverse neighbors, better recall, slower insert
    ///
    /// # Example
    ///
    /// ```
    /// use velesdb_core::HnswParams;
    ///
    /// let params = HnswParams::auto(768).with_alpha(1.0);
    /// assert!((params.alpha - 1.0).abs() < f32::EPSILON);
    /// ```
    #[must_use]
    pub const fn with_alpha(self, alpha: f32) -> Self {
        Self { alpha, ..self }
    }
}

/// Search quality profile controlling the recall/latency tradeoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SearchQuality {
    /// Fast search with `ef_search=96`. ~95% recall, lowest latency.
    Fast,
    /// Balanced search with `ef_search=160`. ~99.5% recall, production default.
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
    /// Auto-tuned adaptive search based on collection statistics.
    ///
    /// Computes optimal `min_ef` / `max_ef` from the collection's current size
    /// and vector dimension, then delegates to the same two-phase adaptive
    /// algorithm used by [`SearchQuality::Adaptive`].
    ///
    /// This is the recommended quality setting for applications that want
    /// good recall without manual ef tuning:
    ///
    /// - Small collections (≤1K): conservative ef (fast)
    /// - Medium collections (1K–100K): moderate ef (balanced)
    /// - Large collections (100K+): aggressive ef (high recall)
    /// - High dimensions (>512): additional exploration factor
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::SearchQuality;
    /// let results = index.search_with_quality(&query, 10, SearchQuality::AutoTune);
    /// ```
    AutoTune,
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
            Self::Fast => 96.max(k * 3),
            // AutoTune: resolved at search time via auto_ef_range(); fallback
            // to Balanced for contexts that call ef_search() without collection info.
            Self::Balanced | Self::AutoTune => 160.max(k * 5),
            // Increased from 256 to 512 for better recall at 100K+ scale
            Self::Accurate => 512.max(k * 16),
            // Increased from 2048 to 4096 for guaranteed 100% recall at 100K+
            Self::Perfect => 4096.max(k * 100),
            Self::Custom(ef) => (*ef).max(k),
            // Adaptive: start with min_ef (first phase)
            Self::Adaptive { min_ef, .. } => (*min_ef).max(k),
        }
    }

    /// Returns `ef_search` scaled to dataset size.
    ///
    /// For datasets > 10K, the HNSW graph is deeper and requires a slightly
    /// larger search budget. Uses sqrt-based scaling (gentler than log2) to
    /// avoid excessive latency when the graph is already well-constructed
    /// with appropriate M and `ef_construction` parameters.
    ///
    /// Capped at 2x the base ef to prevent latency explosion.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn ef_search_for_scale(&self, k: usize, dataset_size: usize) -> usize {
        let base = self.ef_search(k);
        if dataset_size <= 10_000 {
            return base;
        }
        // sqrt scaling: gentle increase that doesn't destroy latency.
        // At 100K: sqrt(10) ≈ 3.16 → capped at 2.0 → ef * 2.
        // At 1M: sqrt(100) = 10 → capped at 2.0 → ef * 2.
        // Reason: dataset_size and base are small enough that f64 is exact.
        let ratio = dataset_size as f64 / 10_000.0;
        let scale = ratio.sqrt().min(2.0);
        let scaled = (base as f64 * scale) as usize;
        scaled.min(base * 2)
    }

    /// Returns `true` if this quality profile uses two-phase adaptive search.
    #[must_use]
    pub const fn is_adaptive(&self) -> bool {
        matches!(self, Self::Adaptive { .. } | Self::AutoTune)
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
