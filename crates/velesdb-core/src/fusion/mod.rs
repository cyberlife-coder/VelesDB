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
//! # Example
//!
//! ```rust,ignore
//! use velesdb_core::fusion::FusionStrategy;
//!
//! // Fuse results from 4 queries using RRF
//! let strategy = FusionStrategy::RRF { k: 60 };
//! let fused = strategy.fuse(multi_query_results);
//! ```

mod strategy;

#[cfg(test)]
mod strategy_tests;

pub use strategy::{FusionError, FusionStrategy};
