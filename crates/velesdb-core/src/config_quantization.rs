//! Quantization configuration types (PQ-06).
//!
//! Extracted from `config.rs` to keep file NLOC under 500.
//! Re-exported by both `config` and the crate root so existing import
//! paths remain unchanged.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// QuantizationType enum (PQ-06)
// ---------------------------------------------------------------------------

/// Default value for PQ codebook size (`k`).
const fn default_k() -> usize {
    256
}

/// Default value for PQ oversampling factor.
#[allow(clippy::unnecessary_wraps)]
const fn default_oversampling() -> Option<u32> {
    Some(4)
}

/// Quantization type for a collection (PQ-06).
///
/// Determines which quantization algorithm is applied to stored vectors.
/// Uses a serde-tagged representation for the new format, with backward
/// compatibility via [`QuantizationConfig`]'s custom deserializer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[non_exhaustive]
pub enum QuantizationType {
    /// No quantization -- full-precision vectors.
    #[default]
    None,
    /// Scalar quantization to 8-bit integers (4x compression).
    #[serde(alias = "sq8")]
    SQ8,
    /// Binary quantization (32x compression).
    Binary,
    /// Product quantization with configurable subspaces.
    #[serde(alias = "pq")]
    PQ {
        /// Number of subspaces (dimension must be divisible by `m`).
        m: usize,
        /// Codebook size per subspace.
        #[serde(default = "default_k")]
        k: usize,
        /// Enable Optimized Product Quantization (OPQ) rotation.
        #[serde(default)]
        opq_enabled: bool,
        /// Oversampling factor for training. `None` disables oversampling.
        #[serde(default = "default_oversampling")]
        oversampling: Option<u32>,
    },
    /// Randomized Binary Quantization.
    #[serde(alias = "rabitq")]
    RaBitQ,
}

impl QuantizationType {
    /// Returns `true` if this is Product Quantization.
    #[must_use]
    pub const fn is_pq(&self) -> bool {
        matches!(self, Self::PQ { .. })
    }

    /// Returns `true` if this is Randomized Binary Quantization.
    #[must_use]
    pub const fn is_rabitq(&self) -> bool {
        matches!(self, Self::RaBitQ)
    }
}

// ---------------------------------------------------------------------------
// QuantizationConfig
// ---------------------------------------------------------------------------

/// Quantization configuration section (EPIC-073/US-005, PQ-06).
///
/// Supports two JSON shapes for backward compatibility:
/// - **Old format:** `{"default_type": "sq8", "rerank_enabled": true, ...}`
/// - **New format:** `{"mode": {"type": "pq", "m": 8, ...}, "rerank_enabled": true, ...}`
#[derive(Debug, Clone, Serialize)]
pub struct QuantizationConfig {
    /// Quantization mode (replaces the old `default_type` string).
    pub mode: QuantizationType,
    /// Enable reranking after quantized search.
    pub rerank_enabled: bool,
    /// Reranking multiplier for candidates.
    pub rerank_multiplier: usize,
    /// Auto-enable quantization for large collections (EPIC-073/US-005).
    pub auto_quantization: bool,
    /// Threshold for auto-quantization (number of vectors).
    pub auto_quantization_threshold: usize,
}

impl Default for QuantizationConfig {
    fn default() -> Self {
        Self {
            mode: QuantizationType::None,
            rerank_enabled: true,
            rerank_multiplier: 2,
            auto_quantization: true,
            auto_quantization_threshold: 10_000,
        }
    }
}

impl QuantizationConfig {
    /// Returns a reference to the quantization mode.
    #[must_use]
    pub const fn mode(&self) -> &QuantizationType {
        &self.mode
    }

    /// Returns whether quantization should be used based on vector count (EPIC-073/US-005).
    #[must_use]
    pub fn should_quantize(&self, vector_count: usize) -> bool {
        self.auto_quantization && vector_count >= self.auto_quantization_threshold
    }
}

// ---------------------------------------------------------------------------
// Custom Deserialize for backward compatibility (PQ-06)
// ---------------------------------------------------------------------------

impl<'de> Deserialize<'de> for QuantizationConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        /// Raw intermediate struct that accepts either old or new format.
        #[derive(Deserialize)]
        struct RawQuantizationConfig {
            /// New format: structured mode object.
            #[serde(default)]
            mode: Option<QuantizationType>,
            /// Old format: plain string ("none", "sq8", "binary").
            #[serde(default)]
            default_type: Option<String>,
            #[serde(default = "default_rerank_enabled")]
            rerank_enabled: bool,
            #[serde(default = "default_rerank_multiplier")]
            rerank_multiplier: usize,
            #[serde(default = "default_auto_quantization")]
            auto_quantization: bool,
            #[serde(default = "default_auto_quantization_threshold")]
            auto_quantization_threshold: usize,
        }

        fn default_rerank_enabled() -> bool {
            true
        }
        fn default_rerank_multiplier() -> usize {
            2
        }
        fn default_auto_quantization() -> bool {
            true
        }
        fn default_auto_quantization_threshold() -> usize {
            10_000
        }

        let raw = RawQuantizationConfig::deserialize(deserializer)?;

        let mode = if let Some(m) = raw.mode {
            m
        } else if let Some(ref s) = raw.default_type {
            match s.as_str() {
                "none" | "" => QuantizationType::None,
                "sq8" => QuantizationType::SQ8,
                "binary" => QuantizationType::Binary,
                other => {
                    return Err(serde::de::Error::custom(format!(
                        "unknown quantization type: '{other}'"
                    )));
                }
            }
        } else {
            QuantizationType::None
        };

        Ok(Self {
            mode,
            rerank_enabled: raw.rerank_enabled,
            rerank_multiplier: raw.rerank_multiplier,
            auto_quantization: raw.auto_quantization,
            auto_quantization_threshold: raw.auto_quantization_threshold,
        })
    }
}
