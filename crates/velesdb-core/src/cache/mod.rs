//! Caching layer for VelesDB (SOTA 2026).
//!
//! Based on arXiv:2310.11703v2 recommendations:
//! - LRU cache for metadata-only collections
//! - Bloom filter for existence checks
//! - Cache statistics and monitoring
//!
//! # Thread-Safety & Lock Ordering
//!
//! All structures are thread-safe via `parking_lot::RwLock`.
//!
//! **Lock Hierarchy (acquire in this order to prevent deadlocks):**
//! 1. `BloomFilter.bits` (RwLock)
//! 2. `BloomFilter.count` (RwLock)
//! 3. `LruCache.inner` (RwLock)

mod bloom;
mod lru;

pub use bloom::BloomFilter;
pub use lru::{CacheStats, LruCache};

#[cfg(test)]
mod deadlock_tests;
#[cfg(test)]
mod tests;
