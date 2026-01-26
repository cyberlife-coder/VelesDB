//! Sharded index for MmapStorage.
//!
//! EPIC-033/US-004: Reduces lock contention for read-heavy workloads.
//!
//! # Performance
//!
//! - **16 shards**: Reduces lock contention by 16x on concurrent reads
//! - **Hash-based routing**: O(1) shard selection using ID % 16
//! - **Independent locks**: Reads to different shards don't block each other

use parking_lot::RwLock;
use rustc_hash::FxHashMap;

/// Number of shards for the index.
/// 16 is optimal for most systems (power of 2, matches common core counts).
pub(crate) const NUM_SHARDS: usize = 16;

/// A single shard containing ID -> offset mappings.
#[derive(Debug, Default)]
struct IndexShard {
    /// Maps vector ID to file offset.
    entries: FxHashMap<u64, usize>,
}

/// Sharded index with 16 partitions for reduced lock contention.
///
/// Uses hash-based sharding to distribute entries across partitions,
/// enabling parallel reads without global lock contention.
#[derive(Debug)]
pub struct ShardedIndex {
    /// 16 independent shards, each with its own lock.
    shards: [RwLock<IndexShard>; NUM_SHARDS],
}

impl Default for ShardedIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl ShardedIndex {
    /// Creates a new empty sharded index.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shards: std::array::from_fn(|_| RwLock::new(IndexShard::default())),
        }
    }

    /// Creates a sharded index from an existing HashMap.
    #[must_use]
    pub fn from_hashmap(map: FxHashMap<u64, usize>) -> Self {
        let index = Self::new();
        for (id, offset) in map {
            index.insert(id, offset);
        }
        index
    }

    /// Computes the shard index for a given ID.
    ///
    /// Uses simple modulo for O(1) routing.
    #[inline]
    const fn shard_index(id: u64) -> usize {
        (id % NUM_SHARDS as u64) as usize
    }

    /// Inserts an entry into the index.
    ///
    /// This only locks the target shard, not the entire index.
    pub fn insert(&self, id: u64, offset: usize) {
        let shard_idx = Self::shard_index(id);
        let mut shard = self.shards[shard_idx].write();
        shard.entries.insert(id, offset);
    }

    /// Gets an offset by ID.
    ///
    /// This only locks the target shard for reading.
    #[must_use]
    pub fn get(&self, id: u64) -> Option<usize> {
        let shard_idx = Self::shard_index(id);
        let shard = self.shards[shard_idx].read();
        shard.entries.get(&id).copied()
    }

    /// Checks if an ID exists in the index.
    #[must_use]
    pub fn contains_key(&self, id: u64) -> bool {
        let shard_idx = Self::shard_index(id);
        let shard = self.shards[shard_idx].read();
        shard.entries.contains_key(&id)
    }

    /// Removes an entry from the index.
    ///
    /// Returns the old offset if it existed.
    pub fn remove(&self, id: u64) -> Option<usize> {
        let shard_idx = Self::shard_index(id);
        let mut shard = self.shards[shard_idx].write();
        shard.entries.remove(&id)
    }

    /// Returns the total number of entries across all shards.
    #[must_use]
    pub fn len(&self) -> usize {
        self.shards.iter().map(|s| s.read().entries.len()).sum()
    }

    /// Returns true if the index is empty.
    #[must_use]
    #[allow(dead_code)] // API completeness
    pub fn is_empty(&self) -> bool {
        self.shards.iter().all(|s| s.read().entries.is_empty())
    }

    /// Clears all entries from all shards.
    pub fn clear(&self) {
        for shard in &self.shards {
            shard.write().entries.clear();
        }
    }

    /// Collects all IDs from all shards.
    #[must_use]
    pub fn keys(&self) -> Vec<u64> {
        let mut keys = Vec::with_capacity(self.len());
        for shard in &self.shards {
            let guard = shard.read();
            keys.extend(guard.entries.keys().copied());
        }
        keys
    }

    /// Collects all entries into a single HashMap for serialization.
    #[must_use]
    pub fn to_hashmap(&self) -> FxHashMap<u64, usize> {
        let mut map = FxHashMap::default();
        map.reserve(self.len());
        for shard in &self.shards {
            let guard = shard.read();
            for (&id, &offset) in &guard.entries {
                map.insert(id, offset);
            }
        }
        map
    }

    /// Returns the maximum offset value across all shards.
    #[must_use]
    #[allow(dead_code)] // API completeness
    pub fn max_offset(&self) -> Option<usize> {
        let mut max = None;
        for shard in &self.shards {
            let guard = shard.read();
            for &offset in guard.entries.values() {
                max = Some(max.map_or(offset, |m: usize| m.max(offset)));
            }
        }
        max
    }

    /// Reserves capacity in all shards.
    ///
    /// Distributes the expected capacity evenly across shards.
    #[allow(dead_code)] // API completeness
    pub fn reserve(&self, additional: usize) {
        let per_shard = additional / NUM_SHARDS + 1;
        for shard in &self.shards {
            shard.write().entries.reserve(per_shard);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sharded_index_insert_get() {
        let index = ShardedIndex::new();
        index.insert(1, 100);
        index.insert(2, 200);
        index.insert(17, 1700); // Same shard as 1 (17 % 16 = 1)

        assert_eq!(index.get(1), Some(100));
        assert_eq!(index.get(2), Some(200));
        assert_eq!(index.get(17), Some(1700));
        assert_eq!(index.get(99), None);
    }

    #[test]
    fn test_sharded_index_remove() {
        let index = ShardedIndex::new();
        index.insert(1, 100);
        assert_eq!(index.remove(1), Some(100));
        assert_eq!(index.get(1), None);
        assert_eq!(index.remove(1), None);
    }

    #[test]
    fn test_sharded_index_len() {
        let index = ShardedIndex::new();
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);

        for i in 0..100u64 {
            index.insert(i, i as usize * 10);
        }
        assert_eq!(index.len(), 100);
        assert!(!index.is_empty());
    }

    #[test]
    fn test_sharded_index_to_hashmap() {
        let index = ShardedIndex::new();
        for i in 0..50u64 {
            index.insert(i, i as usize * 10);
        }

        let map = index.to_hashmap();
        assert_eq!(map.len(), 50);
        assert_eq!(map.get(&25), Some(&250));
    }

    #[test]
    fn test_sharded_index_from_hashmap() {
        let mut map = FxHashMap::default();
        for i in 0..50u64 {
            map.insert(i, i as usize * 10);
        }

        let index = ShardedIndex::from_hashmap(map);
        assert_eq!(index.len(), 50);
        assert_eq!(index.get(25), Some(250));
    }

    #[test]
    fn test_sharded_index_max_offset() {
        let index = ShardedIndex::new();
        assert_eq!(index.max_offset(), None);

        index.insert(1, 100);
        index.insert(2, 500);
        index.insert(3, 200);
        assert_eq!(index.max_offset(), Some(500));
    }

    #[test]
    fn test_sharded_index_concurrent_reads() {
        use std::sync::Arc;
        use std::thread;

        let index = Arc::new(ShardedIndex::new());
        for i in 0..1000u64 {
            index.insert(i, i as usize * 10);
        }

        // Spawn multiple readers
        let mut handles = vec![];
        for _ in 0..8 {
            let idx = Arc::clone(&index);
            handles.push(thread::spawn(move || {
                for i in 0..1000u64 {
                    assert_eq!(idx.get(i), Some(i as usize * 10));
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }
}
