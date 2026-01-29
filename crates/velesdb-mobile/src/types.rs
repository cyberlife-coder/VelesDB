//! Mobile types and enums module (EPIC-061/US-005 refactoring).
//!
//! Extracted from lib.rs to improve modularity.

use velesdb_core::DistanceMetric as CoreDistanceMetric;
use velesdb_core::FusionStrategy as CoreFusionStrategy;

// ============================================================================
// Error Types
// ============================================================================

/// Errors that can occur when using VelesDB on mobile.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum VelesError {
    /// Database operation failed.
    #[error("Database error: {message}")]
    Database { message: String },

    /// Collection operation failed.
    #[error("Collection error: {message}")]
    Collection { message: String },

    /// Vector dimension mismatch.
    #[error("Dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: u32, actual: u32 },
}

impl From<velesdb_core::Error> for VelesError {
    fn from(err: velesdb_core::Error) -> Self {
        match err {
            velesdb_core::Error::DimensionMismatch { expected, actual } =>
            {
                #[allow(clippy::cast_possible_truncation)]
                VelesError::DimensionMismatch {
                    expected: expected as u32,
                    actual: actual as u32,
                }
            }
            velesdb_core::Error::CollectionNotFound(name) => VelesError::Collection {
                message: format!("Collection not found: {name}"),
            },
            velesdb_core::Error::CollectionExists(name) => VelesError::Collection {
                message: format!("Collection already exists: {name}"),
            },
            other => VelesError::Database {
                message: other.to_string(),
            },
        }
    }
}

// ============================================================================
// Enums
// ============================================================================

/// Distance metric for vector similarity.
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum DistanceMetric {
    /// Cosine similarity (1 - cosine_distance). Higher is more similar.
    Cosine,
    /// Euclidean (L2) distance. Lower is more similar.
    Euclidean,
    /// Dot product. Higher is more similar (for normalized vectors).
    DotProduct,
    /// Hamming distance for binary vectors. Lower is more similar.
    Hamming,
    /// Jaccard similarity for set-like vectors. Higher is more similar.
    Jaccard,
}

impl From<DistanceMetric> for CoreDistanceMetric {
    fn from(metric: DistanceMetric) -> Self {
        match metric {
            DistanceMetric::Cosine => CoreDistanceMetric::Cosine,
            DistanceMetric::Euclidean => CoreDistanceMetric::Euclidean,
            DistanceMetric::DotProduct => CoreDistanceMetric::DotProduct,
            DistanceMetric::Hamming => CoreDistanceMetric::Hamming,
            DistanceMetric::Jaccard => CoreDistanceMetric::Jaccard,
        }
    }
}

/// Storage mode for vector quantization (IoT/Edge optimization).
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum StorageMode {
    /// Full f32 precision (4 bytes/dimension). Best recall.
    Full,
    /// SQ8: 8-bit scalar quantization (1 byte/dimension). 4x compression, ~1% recall loss.
    Sq8,
    /// Binary: 1-bit quantization (1 bit/dimension). 32x compression, ~5-10% recall loss.
    Binary,
}

impl From<StorageMode> for velesdb_core::StorageMode {
    fn from(mode: StorageMode) -> Self {
        match mode {
            StorageMode::Full => velesdb_core::StorageMode::Full,
            StorageMode::Sq8 => velesdb_core::StorageMode::SQ8,
            StorageMode::Binary => velesdb_core::StorageMode::Binary,
        }
    }
}

/// Fusion strategy for combining results from multiple vector searches.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum FusionStrategy {
    /// Average scores across all queries.
    Average,
    /// Take the maximum score for each document.
    Maximum,
    /// Reciprocal Rank Fusion with configurable k parameter.
    Rrf {
        /// RRF k parameter (default: 60). Lower k emphasizes top ranks more.
        k: u32,
    },
    /// Weighted combination of average, maximum, and hit ratio.
    Weighted {
        /// Weight for average score (0.0-1.0).
        avg_weight: f32,
        /// Weight for maximum score (0.0-1.0).
        max_weight: f32,
        /// Weight for hit ratio (0.0-1.0).
        hit_weight: f32,
    },
}

impl From<FusionStrategy> for CoreFusionStrategy {
    fn from(strategy: FusionStrategy) -> Self {
        match strategy {
            FusionStrategy::Average => CoreFusionStrategy::Average,
            FusionStrategy::Maximum => CoreFusionStrategy::Maximum,
            FusionStrategy::Rrf { k } => CoreFusionStrategy::RRF { k },
            FusionStrategy::Weighted {
                avg_weight,
                max_weight,
                hit_weight,
            } => CoreFusionStrategy::Weighted {
                avg_weight,
                max_weight,
                hit_weight,
            },
        }
    }
}

impl Default for FusionStrategy {
    fn default() -> Self {
        Self::Rrf { k: 60 }
    }
}

// ============================================================================
// Data Types
// ============================================================================

/// A search result containing an ID and similarity score.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SearchResult {
    /// Vector ID.
    pub id: u64,
    /// Similarity score.
    pub score: f32,
}

/// A point to insert into the database.
#[derive(Debug, Clone, uniffi::Record)]
pub struct VelesPoint {
    /// Unique identifier.
    pub id: u64,
    /// Vector embedding.
    pub vector: Vec<f32>,
    /// Optional JSON payload as string.
    pub payload: Option<String>,
}

/// Individual search request within a batch.
#[derive(Debug, Clone, uniffi::Record)]
pub struct IndividualSearchRequest {
    /// Query vector.
    pub vector: Vec<f32>,
    /// Number of results.
    pub top_k: u32,
    /// Optional metadata filter as JSON string.
    pub filter: Option<String>,
}
