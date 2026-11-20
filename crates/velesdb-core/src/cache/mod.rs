//! Caching layer for `VelesDB` (SOTA 2026).
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
//! 1. `BloomFilter.bits` (`RwLock`)
//! 2. `BloomFilter.count` (`RwLock`)
//! 3. `LockFreeLruCache` L1 shard (`DashMap`), held while demoting into L2
//! 4. `LruCache.inner` (`RwLock`)

mod bloom;
#[cfg(test)]
mod bloom_tests;
mod lockfree;
#[cfg(test)]
mod lockfree_tests;
mod lru;
mod lru_list;
#[cfg(test)]
mod lru_list_tests;
#[cfg(feature = "persistence")]
pub mod plan_cache;
#[cfg(all(test, feature = "persistence"))]
mod plan_cache_tests;
mod striped_counter;
#[cfg(test)]
mod striped_counter_tests;

pub use bloom::BloomFilter;
pub use lockfree::{LockFreeCacheStats, LockFreeLruCache};
pub use lru::{CacheStats, LruCache};
#[cfg(feature = "persistence")]
pub use plan_cache::{CompiledPlan, CompiledPlanCache, PlanCacheMetrics, PlanKey};

#[cfg(test)]
mod deadlock_tests;
#[cfg(test)]
mod lru_optimization_tests;
#[cfg(test)]
mod performance_tests;
#[cfg(test)]
mod tests;
