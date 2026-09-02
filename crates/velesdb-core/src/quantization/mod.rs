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
//!
//! ## Engine integration status
//!
//! The figures above describe the quantization primitives themselves. In the
//! collection query path: `RaBitQ` (binary traversal backend), SQ8 (int8
//! traversal backend, Euclidean/Cosine) and PQ (ADC rescoring) are wired
//! end-to-end. Persistence across reopens covers TRAIN-QUANTIZER-produced
//! artifacts (`rabitq.idx`, `sq8.idx`, `codebook.pq`) plus lazily-trained
//! `RaBitQ`/SQ8 quantizers (persisted by the full flush); a PQ quantizer
//! trained lazily from inserts (no TRAIN statement) is in-memory only and
//! retrains after a restart. The Binary collection mode stays full-precision
//! f32 in the search path. See `docs/guides/QUANTIZATION.md`.

use std::io;

use serde::{Deserialize, Serialize};

/// Validate that a flat row-major rotation matrix has exactly `dimension^2`
/// elements, returning [`crate::error::Error::IndexCorrupted`] otherwise.
///
/// Shared by the PQ (OPQ) and `RaBitQ` load-time validators so the unchecked
/// `matrix[i * d + j]` indexing in their rotation kernels stays in bounds.
pub(crate) fn validate_rotation_len(
    len: usize,
    dimension: usize,
    label: &str,
) -> Result<(), crate::error::Error> {
    // `checked_mul`: `dimension` is attacker-controlled post-deserialize; a wrapping
    // `dimension * dimension` (esp. on 32-bit targets) could yield a small `expected`
    // that a tampered `len` matches, false-passing the shape check that the unchecked
    // `matrix[i * d + j]` indexing relies on.
    let Some(expected) = dimension.checked_mul(dimension) else {
        return Err(crate::error::Error::IndexCorrupted(format!(
            "{label} rotation dimension {dimension} squared overflows usize"
        )));
    };
    if len != expected {
        return Err(crate::error::Error::IndexCorrupted(format!(
            "{label} rotation has {len} elements, expected dimension^2 = {expected}"
        )));
    }
    Ok(())
}

mod binary;
pub(crate) mod codec_helpers;
mod pq;
pub(crate) mod pq_kmeans;
pub(crate) mod pq_opq;
#[cfg(feature = "persistence")]
mod pq_persistence;
mod rabitq;
pub(crate) mod rabitq_store;
mod scalar;

// Re-export binary quantization
pub use binary::BinaryQuantizedVector;
#[allow(unused_imports)] // Called from vector.rs search path (persistence-gated).
pub(crate) use pq::distance_pq_l2;
#[allow(unused_imports)] // Called from vector.rs search path (persistence-gated).
pub(crate) use pq::pq_adc_batch_rescore;
pub use pq::{PQCodebook, PQVector, ProductQuantizer};
#[cfg(feature = "persistence")]
pub use pq_opq::train_opq;

// Re-export RaBitQ quantization
#[cfg(feature = "persistence")]
pub use rabitq::PreparedQuery;
pub use rabitq::{RaBitQCorrection, RaBitQIndex, RaBitQVector};
#[cfg(feature = "persistence")]
pub(crate) use rabitq_store::RaBitQVectorStore;

// Re-export scalar quantization
pub use scalar::{
    cosine_similarity_quantized, cosine_similarity_quantized_simd, dot_product_quantized,
    dot_product_quantized_simd, euclidean_squared_quantized, euclidean_squared_quantized_simd,
    QuantizedVector,
};

/// Trait for serializing and deserializing quantized vectors to/from bytes.
///
/// Provides a uniform interface for byte-level serialization across
/// different quantization strategies (SQ8, Binary).
pub trait QuantizationCodec: Sized {
    /// Serializes the quantized vector to a byte representation.
    fn to_bytes(&self) -> Vec<u8>;

    /// Deserializes a quantized vector from bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the byte slice is too short or contains invalid data.
    fn from_bytes(bytes: &[u8]) -> io::Result<Self>;
}

/// Canonical names of every [`StorageMode`] variant, in declaration order.
///
/// Single source of truth for the storage-mode name set exported to downstream
/// crates and bindings (Python `velesdb.STORAGE_MODES`, the integrations
/// security guard). Each entry is the variant's
/// [`canonical_name`](StorageMode::canonical_name); a unit test asserts the
/// slice stays exhaustive so adding a variant without updating it fails CI.
pub const STORAGE_MODE_NAMES: &[&str] = &["full", "sq8", "binary", "pq", "rabitq"];

/// Storage mode for vectors.
///
/// # What each mode actually does
///
/// | Mode | Collection storage + search path |
/// |------|----------------------------------|
/// | `Full` | f32 (baseline) |
/// | `SQ8` | int8 graph traversal + exact f32 re-ranking (Euclidean/Cosine; other metrics stay f32) |
/// | `Binary` | f32 — behaves as `Full` today (use `RaBitQ` for compressed search) |
/// | `ProductQuantization` | f32 storage + ADC-rescored search (wired) |
/// | `RaBitQ` | quantized traversal, wired end-to-end |
///
/// **Search-path modes (`RaBitQ`, `SQ8`, `ProductQuantization`)** are the
/// quantized paths wired into the query hot path. All of them keep the f32
/// vectors for exact re-ranking, so total resident memory is not reduced —
/// for `RaBitQ` and `SQ8` it rises, since the codes are additive. What those
/// two shrink is the *un-evictable* floor: their f32 lives in a file-backed
/// arena the kernel can reclaim. Measured at 100 000 x 768-d, anonymous RSS
/// falls 61% (385 -> 150 MiB) while total RSS rises 11%. See the measured
/// tables in `docs/guides/QUANTIZATION.md`. `Binary` is accepted and persisted so
/// the intent survives a reopen, but changes neither memory use nor the
/// search path today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum StorageMode {
    /// Full precision f32 storage (default).
    #[default]
    Full,
    /// 8-bit scalar quantization. Search-path mode for Euclidean and Cosine:
    /// graph traversal compares int8 codes (1 byte/dimension read instead of
    /// 4) and the final top-k is re-ranked with exact f32 distances. The
    /// quantizer trains lazily after 1000 inserts (or via `TRAIN QUANTIZER
    /// type=sq8`) and persists to `sq8.idx`; traversal engages at 10 000+
    /// vectors, below which search stays exact f32. On other metrics (int8
    /// L2 cannot preserve their ordering) the collection behaves as
    /// [`Full`]. The f32 kept for re-ranking sits in a file-backed arena, so
    /// it is evictable rather than pinned. Measured at 100 000 x 768-d against
    /// [`Full`]: anonymous RSS 385 -> 150 MiB (-61%), total RSS +11% because
    /// the codes are additive, and the first re-rank after a reclaim pays
    /// 8-10 ms per 100 candidates.
    ///
    /// [`Full`]: StorageMode::Full
    SQ8,
    /// Accepted and persisted, but currently behaves exactly like [`Full`] —
    /// same status as [`SQ8`](StorageMode::SQ8). For a real quantized search
    /// path use [`RaBitQ`](StorageMode::RaBitQ) (32x, wired end-to-end).
    ///
    /// [`Full`]: StorageMode::Full
    Binary,
    /// Product Quantization (PQ) for aggressive lossy compression (8x-16x
    /// typical). Search-path mode: wired into the query hot path for ADC
    /// (Asymmetric Distance Computation) rescoring.
    ProductQuantization,
    /// `RaBitQ` binary quantization for 32x compression with scalar correction.
    /// Search-path mode: the performant quantized search path, wired
    /// end-to-end into the query hot path.
    RaBitQ,
}

impl StorageMode {
    /// Returns the canonical lowercase name for this storage mode.
    ///
    /// This is the single source of truth for string representations,
    /// used by [`std::fmt::Display`], [`std::str::FromStr`], and downstream crates.
    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::SQ8 => "sq8",
            Self::Binary => "binary",
            Self::ProductQuantization => "pq",
            Self::RaBitQ => "rabitq",
        }
    }

    /// Parses a storage mode string with alias support.
    ///
    /// Accepted aliases (case-insensitive):
    /// - `full`, `f32` -> `Full`
    /// - `sq8`, `int8` -> `SQ8`
    /// - `binary`, `bit` -> `Binary`
    /// - `pq`, `product_quantization` -> `ProductQuantization`
    /// - `rabitq` -> `RaBitQ`
    ///
    /// # Examples
    ///
    /// ```
    /// use velesdb_core::StorageMode;
    ///
    /// assert_eq!(StorageMode::parse_alias("sq8"), Some(StorageMode::SQ8));
    /// assert_eq!(StorageMode::parse_alias("INT8"), Some(StorageMode::SQ8));
    /// assert_eq!(StorageMode::parse_alias("unknown"), None);
    /// ```
    #[must_use]
    pub fn parse_alias(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "full" | "f32" => Some(Self::Full),
            "sq8" | "int8" => Some(Self::SQ8),
            "binary" | "bit" => Some(Self::Binary),
            "pq" | "product_quantization" => Some(Self::ProductQuantization),
            "rabitq" => Some(Self::RaBitQ),
            _ => None,
        }
    }
}

impl std::fmt::Display for StorageMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.canonical_name())
    }
}

impl std::str::FromStr for StorageMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_alias(s).ok_or_else(|| {
            format!(
                "Unknown storage mode '{s}'. Valid options: full, f32, sq8, int8, binary, bit, pq, product_quantization, rabitq"
            )
        })
    }
}

#[cfg(test)]
#[path = "storage_mode_parsing_tests.rs"]
mod storage_mode_parsing_tests;
