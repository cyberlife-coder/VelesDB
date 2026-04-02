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

use std::io;

use serde::{Deserialize, Serialize};

mod binary;
pub(crate) mod codec_helpers;
mod pq;
pub(crate) mod pq_kmeans;
pub(crate) mod pq_opq;
mod rabitq;
pub(crate) mod rabitq_store;
mod scalar;

// Re-export binary quantization
pub use binary::BinaryQuantizedVector;
#[allow(unused_imports)]
pub(crate) use pq::distance_pq_l2;
pub use pq::{PQCodebook, PQVector, ProductQuantizer};
#[cfg(feature = "persistence")]
pub use pq_opq::train_opq;

// Re-export RaBitQ quantization
#[cfg(feature = "persistence")]
pub(crate) use rabitq::PreparedQuery;
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

impl StorageMode {
    /// Returns the canonical lowercase name for this storage mode.
    ///
    /// This is the single source of truth for string representations,
    /// used by [`Display`], [`FromStr`], and downstream crates.
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
mod storage_mode_parsing_tests {
    use super::StorageMode;

    #[test]
    fn test_parse_all_canonical_names() {
        assert_eq!("full".parse::<StorageMode>().unwrap(), StorageMode::Full);
        assert_eq!("sq8".parse::<StorageMode>().unwrap(), StorageMode::SQ8);
        assert_eq!(
            "binary".parse::<StorageMode>().unwrap(),
            StorageMode::Binary
        );
        assert_eq!(
            "pq".parse::<StorageMode>().unwrap(),
            StorageMode::ProductQuantization
        );
        assert_eq!(
            "rabitq".parse::<StorageMode>().unwrap(),
            StorageMode::RaBitQ
        );
    }

    #[test]
    fn test_parse_aliases() {
        assert_eq!("f32".parse::<StorageMode>().unwrap(), StorageMode::Full);
        assert_eq!("int8".parse::<StorageMode>().unwrap(), StorageMode::SQ8);
        assert_eq!("bit".parse::<StorageMode>().unwrap(), StorageMode::Binary);
        assert_eq!(
            "product_quantization".parse::<StorageMode>().unwrap(),
            StorageMode::ProductQuantization
        );
    }

    #[test]
    fn test_parse_case_insensitive() {
        assert_eq!("SQ8".parse::<StorageMode>().unwrap(), StorageMode::SQ8);
        assert_eq!("FULL".parse::<StorageMode>().unwrap(), StorageMode::Full);
        assert_eq!(
            "RaBitQ".parse::<StorageMode>().unwrap(),
            StorageMode::RaBitQ
        );
    }

    #[test]
    fn test_parse_unknown_returns_error() {
        assert!("unknown".parse::<StorageMode>().is_err());
        assert!("".parse::<StorageMode>().is_err());
    }

    #[test]
    fn test_canonical_name_roundtrip() {
        for mode in [
            StorageMode::Full,
            StorageMode::SQ8,
            StorageMode::Binary,
            StorageMode::ProductQuantization,
            StorageMode::RaBitQ,
        ] {
            let name = mode.canonical_name();
            assert_eq!(name.parse::<StorageMode>().unwrap(), mode);
        }
    }

    #[test]
    fn test_display_uses_canonical_name() {
        assert_eq!(format!("{}", StorageMode::Full), "full");
        assert_eq!(format!("{}", StorageMode::SQ8), "sq8");
        assert_eq!(format!("{}", StorageMode::Binary), "binary");
        assert_eq!(format!("{}", StorageMode::ProductQuantization), "pq");
        assert_eq!(format!("{}", StorageMode::RaBitQ), "rabitq");
    }
}
