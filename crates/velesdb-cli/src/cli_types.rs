//! CLI argument types for clap `ValueEnum` derivation.
//!
//! Contains `MetricArg`, `StorageModeArg`, and `IndexTypeArg` plus their
//! `From` conversions into the core domain types.

use clap::ValueEnum;
use velesdb_core::{DistanceMetric, StorageMode};

/// CLI metric option
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum MetricArg {
    #[default]
    Cosine,
    Euclidean,
    Dot,
    Hamming,
    Jaccard,
}

impl From<MetricArg> for DistanceMetric {
    fn from(m: MetricArg) -> Self {
        match m {
            MetricArg::Cosine => DistanceMetric::Cosine,
            MetricArg::Euclidean => DistanceMetric::Euclidean,
            MetricArg::Dot => DistanceMetric::DotProduct,
            MetricArg::Hamming => DistanceMetric::Hamming,
            MetricArg::Jaccard => DistanceMetric::Jaccard,
        }
    }
}

/// CLI storage mode option
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum StorageModeArg {
    #[default]
    Full,
    Sq8,
    Binary,
    Pq,
    Rabitq,
}

impl From<StorageModeArg> for StorageMode {
    fn from(m: StorageModeArg) -> Self {
        match m {
            StorageModeArg::Full => StorageMode::Full,
            StorageModeArg::Sq8 => StorageMode::SQ8,
            StorageModeArg::Binary => StorageMode::Binary,
            StorageModeArg::Pq => StorageMode::ProductQuantization,
            StorageModeArg::Rabitq => StorageMode::RaBitQ,
        }
    }
}

/// CLI index type option
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum IndexTypeArg {
    Secondary,
    Property,
    Range,
}
