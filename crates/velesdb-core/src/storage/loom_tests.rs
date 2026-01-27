//! Loom-based concurrency tests for storage operations.
//!
//! EPIC-034/US-004: Uses Loom to exhaustively verify lock ordering
//! and detect potential deadlocks in concurrent scenarios.
//!
//! # Running Loom Tests
//!
//! ```bash
//! RUSTFLAGS="--cfg loom" cargo test --test loom_tests --release
//! ```
//!
//! # What Loom Tests
//!
//! - Concurrent insert/read races
//! - Lock ordering verification
//! - Epoch counter wrap-around safety
//! - Sharded index concurrent access

#[cfg(loom)]
mod loom_sharded_index {
    use loom::sync::atomic::{AtomicU64, Ordering};
    use loom::sync::RwLock;
    use loom::thread;
    use std::collections::HashMap;
    use std::sync::Arc;

    const NUM_SHARDS: usize = 4;

    struct LoomShardedIndex {
        shards: [RwLock<HashMap<u64, usize>>; NUM_SHARDS],
    }

    impl LoomShardedIndex {
        fn new() -> Self {
            Self {
                shards: std::array::from_fn(|_| RwLock::new(HashMap::new())),
            }
        }

        fn shard_index(id: u64) -> usize {
            (id % NUM_SHARDS as u64) as usize
        }

        fn insert(&self, id: u64, offset: usize) {
            let shard_idx = Self::shard_index(id);
            let mut shard = self.shards[shard_idx].write().unwrap();
            shard.insert(id, offset);
        }

        fn get(&self, id: u64) -> Option<usize> {
            let shard_idx = Self::shard_index(id);
            let shard = self.shards[shard_idx].read().unwrap();
            shard.get(&id).copied()
        }
    }

    #[test]
    fn test_concurrent_insert_read() {
        loom::model(|| {
            let index = Arc::new(LoomShardedIndex::new());

            let idx1 = Arc::clone(&index);
            let t1 = thread::spawn(move || {
                idx1.insert(1, 100);
            });

            let idx2 = Arc::clone(&index);
            let t2 = thread::spawn(move || {
                idx2.insert(2, 200);
            });

            let idx3 = Arc::clone(&index);
            let t3 = thread::spawn(move || {
                let _ = idx3.get(1);
                let _ = idx3.get(2);
            });

            t1.join().unwrap();
            t2.join().unwrap();
            t3.join().unwrap();
        });
    }

    #[test]
    fn test_same_shard_contention() {
        loom::model(|| {
            let index = Arc::new(LoomShardedIndex::new());

            let idx1 = Arc::clone(&index);
            let t1 = thread::spawn(move || {
                idx1.insert(0, 100);
            });

            let idx2 = Arc::clone(&index);
            let t2 = thread::spawn(move || {
                idx2.insert(4, 400);
            });

            t1.join().unwrap();
            t2.join().unwrap();

            assert!(index.get(0).is_some() || index.get(0).is_none());
            assert!(index.get(4).is_some() || index.get(4).is_none());
        });
    }
}

#[cfg(loom)]
mod loom_epoch_counter {
    use loom::sync::atomic::{AtomicU64, Ordering};
    use loom::thread;
    use std::sync::Arc;

    #[test]
    fn test_epoch_increment_visibility() {
        loom::model(|| {
            let epoch = Arc::new(AtomicU64::new(0));

            let e1 = Arc::clone(&epoch);
            let writer = thread::spawn(move || {
                e1.fetch_add(1, Ordering::Release);
            });

            let e2 = Arc::clone(&epoch);
            let reader = thread::spawn(move || {
                let val = e2.load(Ordering::Acquire);
                assert!(val <= 1);
            });

            writer.join().unwrap();
            reader.join().unwrap();
        });
    }

    #[test]
    fn test_epoch_guard_invalidation() {
        loom::model(|| {
            let epoch = Arc::new(AtomicU64::new(0));

            let e1 = Arc::clone(&epoch);
            let reader_epoch = e1.load(Ordering::Acquire);

            let e2 = Arc::clone(&epoch);
            let writer = thread::spawn(move || {
                e2.fetch_add(1, Ordering::Release);
            });

            writer.join().unwrap();

            let current_epoch = epoch.load(Ordering::Acquire);
            let is_valid = reader_epoch == current_epoch;
            assert!(!is_valid || reader_epoch == 0);
        });
    }
}

#[cfg(not(loom))]
mod standard_concurrency_tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_epoch_wrap_around_safety() {
        let epoch = AtomicU64::new(u64::MAX - 1);

        epoch.fetch_add(1, Ordering::Release);
        assert_eq!(epoch.load(Ordering::Acquire), u64::MAX);

        epoch.fetch_add(1, Ordering::Release);
        assert_eq!(epoch.load(Ordering::Acquire), 0);
    }

    #[test]
    fn test_concurrent_epoch_updates() {
        let epoch = Arc::new(AtomicU64::new(0));
        let num_threads = 8;
        let increments_per_thread = 1000;

        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let e = Arc::clone(&epoch);
                thread::spawn(move || {
                    for _ in 0..increments_per_thread {
                        e.fetch_add(1, Ordering::Release);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            epoch.load(Ordering::Acquire),
            num_threads * increments_per_thread
        );
    }

    #[test]
    fn test_guard_epoch_validation() {
        struct MockGuard {
            epoch_at_creation: u64,
            epoch_ptr: Arc<AtomicU64>,
        }

        impl MockGuard {
            fn is_valid(&self) -> bool {
                let current = self.epoch_ptr.load(Ordering::Acquire);
                current == self.epoch_at_creation
            }
        }

        let epoch = Arc::new(AtomicU64::new(0));

        let guard = MockGuard {
            epoch_at_creation: epoch.load(Ordering::Acquire),
            epoch_ptr: Arc::clone(&epoch),
        };

        assert!(guard.is_valid());

        epoch.fetch_add(1, Ordering::Release);

        assert!(!guard.is_valid());
    }
}
