//! Scalar Quantization (SQ8) and Binary Quantization for memory-efficient vector storage.
//!
//! This module implements quantization strategies to reduce memory usage:
//!
//! ## Benefits
//!
//! | Metric | f32 | SQ8 | Binary |
//! |--------|-----|-----|--------|
//! | RAM/vector (768d) | 3 KB | 770 bytes | 96 bytes |
//! | Cache efficiency | Baseline | ~4x better | ~32x better |
//! | Recall loss | 0% | ~0.5-1% | ~5-10% |

use serde::{Deserialize, Serialize};

mod binary;
mod pq;
mod rabitq;
mod scalar;

// Re-export binary quantization
pub use binary::BinaryQuantizedVector;
#[allow(unused_imports)]
pub(crate) use pq::distance_pq_l2;
#[cfg(feature = "persistence")]
pub use pq::train_opq;
pub use pq::{PQCodebook, PQVector, ProductQuantizer};

// Re-export RaBitQ quantization
pub use rabitq::{RaBitQCorrection, RaBitQIndex, RaBitQVector};

// Re-export scalar quantization
pub use scalar::{
    cosine_similarity_quantized, cosine_similarity_quantized_simd, dot_product_quantized,
    dot_product_quantized_simd, euclidean_squared_quantized, euclidean_squared_quantized_simd,
    QuantizedVector,
};

/// Storage mode for vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum StorageMode {
    /// Full precision f32 storage (default).
    #[default]
    Full,
    /// 8-bit scalar quantization for 4x memory reduction.
    SQ8,
    /// 1-bit binary quantization for 32x memory reduction.
    /// Best for edge/IoT devices with limited RAM.
    Binary,
    /// Product Quantization (PQ) for aggressive lossy compression (8x-16x typical).
    ProductQuantization,
    /// `RaBitQ` binary quantization for 32x compression with scalar correction.
    RaBitQ,
}
