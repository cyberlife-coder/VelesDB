//! Caching layer for `VelesDB` (SOTA 2026).
//!
//! Based on arXiv:2310.11703v2 recommendations:
//! - LRU cache for metadata-only collections
//! - Bloom filter for existence checks
//! - Cache statistics and monitoring

mod bloom;
mod lru;

pub use bloom::BloomFilter;
pub use lru::{CacheStats, LruCache};

#[cfg(test)]
mod tests;
