//! Multi-query fusion strategies for `VelesDB`.
//!
//! This module provides various strategies for combining results from
//! multiple vector searches into a single ranked list.
//!
//! # Strategies
//!
//! - **Average**: Mean of scores across queries
//! - **Maximum**: Best score across queries
//! - **RRF**: Reciprocal Rank Fusion (position-based)
//! - **Weighted**: Custom weighted combination (avg, max, `hit_count`)
//!
//! A surface that takes the strategy by name (REST, CLI, Tauri, WASM, Python)
//! builds it with [`FusionStrategy::from_name`], the one name grammar.
//!
//! # Example
//!
//! ```rust,ignore
//! use velesdb_core::fusion::FusionStrategy;
//!
//! // Fuse results from 4 queries using RRF
//! let strategy = FusionStrategy::RRF { k: 60 };
//! let fused = strategy.fuse(multi_query_results);
//! ```

mod name;
mod strategy;

#[cfg(test)]
mod name_tests;

#[cfg(test)]
mod strategy_tests;

#[cfg(test)]
mod weight_validation_tests;

pub use name::FusionParams;
pub use strategy::{
    min_max_normalize, sort_fused_results, FusionError, FusionStrategy, ScoreDirection,
    DEFAULT_RELATIVE_SCORE_DENSE_WEIGHT, DEFAULT_RELATIVE_SCORE_SPARSE_WEIGHT, DEFAULT_RRF_K,
    DEFAULT_WEIGHTED_AVG_WEIGHT, DEFAULT_WEIGHTED_HIT_WEIGHT, DEFAULT_WEIGHTED_MAX_WEIGHT,
};

// For the hybrid search's top-k heap (`collection::search`), which exists
// only with `persistence`.
#[cfg(feature = "persistence")]
pub(crate) use strategy::fused_score_cmp;
