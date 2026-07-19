//! Loom concurrency tests for `VelesDB` concurrent structures.
//!
//! These tests use the Loom library to verify the absence of data races
//! and deadlocks by exploring all possible thread interleavings.
//!
//! # Running Loom Tests
//!
//! ```bash
//! cargo +nightly test --features loom --test loom_tests
//! ```
//!
//! # EPIC-023: Loom Concurrency Testing
//!
//! ## Tested Components
//!
//! - `ConcurrentEdgeStore`: Thread-safe edge storage with sharding
//! - Lock ordering verification (deadlock prevention)
//! - Cross-shard operations
//!
//! ## References
//!
//! - [Loom crate](https://github.com/tokio-rs/loom)
//! - [Loom user guide](https://docs.rs/loom/latest/loom/)

#![cfg(all(loom, feature = "persistence"))]

use loom::sync::Arc;
use loom::thread;

// Note: For loom tests, we need simplified versions of our structures
// that use loom's sync primitives. This module tests the concurrency
// patterns rather than the actual implementation.

/// Simplified edge for loom testing
#[derive(Clone, Debug, PartialEq)]
struct TestEdge {
    id: u64,
    source: u64,
    target: u64,
    label: String,
}

impl TestEdge {
    fn new(id: u64, source: u64, target: u64, label: &str) -> Self {
        Self {
            id,
            source,
            target,
            label: label.to_string(),
        }
    }
}

/// Simplified concurrent store for loom testing
/// Uses loom's RwLock and HashMap to verify lock ordering patterns
mod loom_edge_store {
    use super::TestEdge;
    use loom::sync::{Arc, RwLock};
    use std::collections::HashMap;

    pub struct LoomEdgeStore {
        shards: Vec<RwLock<HashMap<u64, TestEdge>>>,
        edge_ids: RwLock<HashMap<u64, u64>>, // edge_id -> source_id
        num_shards: usize,
    }

    impl LoomEdgeStore {
        pub fn new(num_shards: usize) -> Self {
            let shards = (0..num_shards)
                .map(|_| RwLock::new(HashMap::new()))
                .collect();
            Self {
                shards,
                edge_ids: RwLock::new(HashMap::new()),
                num_shards,
            }
        }

        fn shard_index(&self, node_id: u64) -> usize {
            (node_id as usize) % self.num_shards
        }

        pub fn add_edge(&self, edge: TestEdge) -> Result<(), &'static str> {
            let edge_id = edge.id;
            let source_id = edge.source;

            // Lock ordering: edge_ids FIRST, then shards in ascending order
            let mut ids = self.edge_ids.write().unwrap();
            if ids.contains_key(&edge_id) {
                return Err("edge exists");
            }

            let source_shard = self.shard_index(edge.source);
            let target_shard = self.shard_index(edge.target);

            if source_shard == target_shard {
                let mut guard = self.shards[source_shard].write().unwrap();
                guard.insert(edge_id, edge);
            } else {
                // Acquire locks in ascending order to prevent deadlock
                let (first_idx, second_idx) = if source_shard < target_shard {
                    (source_shard, target_shard)
                } else {
                    (target_shard, source_shard)
                };

                let mut first = self.shards[first_idx].write().unwrap();
                let mut second = self.shards[second_idx].write().unwrap();

                // Store edge in source shard
                if source_shard < target_shard {
                    first.insert(edge_id, edge.clone());
                } else {
                    second.insert(edge_id, edge.clone());
                }
            }

            ids.insert(edge_id, source_id);
            Ok(())
        }

        pub fn contains_edge(&self, edge_id: u64) -> bool {
            self.edge_ids.read().unwrap().contains_key(&edge_id)
        }

        pub fn get_outgoing(&self, node_id: u64) -> Vec<TestEdge> {
            let shard = &self.shards[self.shard_index(node_id)];
            let guard = shard.read().unwrap();
            guard
                .values()
                .filter(|e| e.source == node_id)
                .cloned()
                .collect()
        }

        pub fn edge_count(&self) -> usize {
            self.edge_ids.read().unwrap().len()
        }
    }
}

use loom_edge_store::LoomEdgeStore;

// ============================================================================
// Test 2: Concurrent edge insertion (cross-shard, lock ordering)
// ============================================================================

#[test]
fn test_loom_concurrent_edge_insert_cross_shard() {
    loom::model(|| {
        let store = Arc::new(LoomEdgeStore::new(4));

        let s1 = Arc::clone(&store);
        let t1 = thread::spawn(move || {
            // Shard 0 -> Shard 1 (requires both locks)
            let _ = s1.add_edge(TestEdge::new(1, 0, 1, "edge_a"));
        });

        let s2 = Arc::clone(&store);
        let t2 = thread::spawn(move || {
            // Shard 1 -> Shard 0 (opposite direction - tests lock ordering)
            let _ = s2.add_edge(TestEdge::new(2, 1, 0, "edge_b"));
        });

        t1.join().unwrap();
        t2.join().unwrap();

        // Both edges should exist - no deadlock
        assert!(store.contains_edge(1));
        assert!(store.contains_edge(2));
        assert_eq!(store.edge_count(), 2);
    });
}

// ============================================================================
// Test 3: Concurrent read/write (reader-writer pattern)
// ============================================================================

#[test]
fn test_loom_concurrent_read_write() {
    loom::model(|| {
        let store = Arc::new(LoomEdgeStore::new(4));

        // Pre-populate with an edge
        store.add_edge(TestEdge::new(1, 0, 1, "initial")).unwrap();

        let s1 = Arc::clone(&store);
        let t1 = thread::spawn(move || {
            // Writer: add new edge
            let _ = s1.add_edge(TestEdge::new(2, 0, 2, "new_edge"));
        });

        let s2 = Arc::clone(&store);
        let t2 = thread::spawn(move || {
            // Reader: query outgoing edges
            let edges = s2.get_outgoing(0);
            // Reader must always observe the committed initial edge (id=1, 0->1),
            // regardless of interleaving with the concurrent writer (no torn read).
            assert!(edges
                .iter()
                .any(|e| e.id == 1 && e.source == 0 && e.target == 1));
        });

        t1.join().unwrap();
        t2.join().unwrap();

        // Final state should have both edges
        assert!(store.contains_edge(1));
        assert!(store.contains_edge(2));
    });
}

// ============================================================================
// Test 4: Duplicate edge insertion (race condition)
// ============================================================================

#[test]
fn test_loom_duplicate_edge_prevention() {
    loom::model(|| {
        let store = Arc::new(LoomEdgeStore::new(4));

        let s1 = Arc::clone(&store);
        let t1 = thread::spawn(move || s1.add_edge(TestEdge::new(1, 0, 1, "edge")));

        let s2 = Arc::clone(&store);
        let t2 = thread::spawn(move || s2.add_edge(TestEdge::new(1, 0, 1, "edge")));

        let r1 = t1.join().unwrap();
        let r2 = t2.join().unwrap();

        // Exactly one should succeed, one should fail
        assert!((r1.is_ok() && r2.is_err()) || (r1.is_err() && r2.is_ok()));
        assert_eq!(store.edge_count(), 1);
    });
}

// ============================================================================
// Test 5: Multiple threads inserting to different shards (no contention)
// ============================================================================

#[test]
fn test_loom_parallel_insert_no_contention() {
    loom::model(|| {
        let store = Arc::new(LoomEdgeStore::new(4));

        let s1 = Arc::clone(&store);
        let t1 = thread::spawn(move || {
            // Shard 0
            let _ = s1.add_edge(TestEdge::new(1, 0, 0, "self_loop_0"));
        });

        let s2 = Arc::clone(&store);
        let t2 = thread::spawn(move || {
            // Shard 1
            let _ = s2.add_edge(TestEdge::new(2, 1, 1, "self_loop_1"));
        });

        let s3 = Arc::clone(&store);
        let t3 = thread::spawn(move || {
            // Shard 2
            let _ = s3.add_edge(TestEdge::new(3, 2, 2, "self_loop_2"));
        });

        t1.join().unwrap();
        t2.join().unwrap();
        t3.join().unwrap();

        // All edges should exist, each placed in its own shard (no loss/mis-shard).
        assert_eq!(store.edge_count(), 3);
        assert!(store.contains_edge(1) && store.contains_edge(2) && store.contains_edge(3));
        assert_eq!(store.get_outgoing(0).len(), 1);
        assert_eq!(store.get_outgoing(1).len(), 1);
        assert_eq!(store.get_outgoing(2).len(), 1);
    });
}

// ============================================================================
// Test 6: add_edge / delete phantom-edge race closure (#1442 re-fix)
// ============================================================================
//
// Models the fix in `collection/core/graph_api.rs`: `add_edge` holds the
// payload-store READ guard from the endpoint-existence check through the
// edge-store write (the WAL mutex is acquired WHILE that read guard is
// still held). `delete()` needs the payload-store WRITE guard to remove a
// node, so it cannot interleave between the check and the write anymore.
//
// The real path is loom-incompatible (the WAL append fsyncs to disk, and
// loom cannot model syscalls) — same limitation already documented for the
// rest of this file, hence a simplified model of just the lock
// interleaving, which is exactly what the fix changes.
mod loom_payload_guard {
    use loom::sync::{Mutex, RwLock};
    use std::collections::HashSet;

    /// Simplified graph: `payload` mirrors `payload_storage`, `wal` mirrors
    /// `edge_wal_lock`, `edges` mirrors the edge store.
    pub struct FixedGraph {
        payload: RwLock<HashSet<u64>>,
        wal: Mutex<()>,
        edges: RwLock<HashSet<(u64, u64)>>,
    }

    impl FixedGraph {
        pub fn new() -> Self {
            Self {
                payload: RwLock::new(HashSet::new()),
                wal: Mutex::new(()),
                edges: RwLock::new(HashSet::new()),
            }
        }

        pub fn seed(&self, node: u64) {
            self.payload.write().unwrap().insert(node);
        }

        /// Mirrors the fixed `add_edge`: the payload read guard spans the
        /// WAL-mutex acquisition and the edge-store write.
        pub fn add_edge(&self, source: u64, target: u64) -> bool {
            let payload = self.payload.read().unwrap();
            if !payload.contains(&source) || !payload.contains(&target) {
                return false;
            }
            let _wal = self.wal.lock().unwrap();
            self.edges.write().unwrap().insert((source, target));
            true
        }

        /// Mirrors `delete()`: the payload write guard is dropped BEFORE
        /// the WAL mutex is taken for the edge cascade (same as
        /// `crud_read_delete.rs`'s `cascade_delete_node_edges`).
        pub fn delete(&self, node: u64) {
            self.payload.write().unwrap().remove(&node);
            let _wal = self.wal.lock().unwrap();
            self.edges
                .write()
                .unwrap()
                .retain(|(s, t)| *s != node && *t != node);
        }

        pub fn has_phantom_edge(&self, node: u64) -> bool {
            let payload_gone = !self.payload.read().unwrap().contains(&node);
            let edge_remains = self
                .edges
                .read()
                .unwrap()
                .iter()
                .any(|(s, t)| *s == node || *t == node);
            payload_gone && edge_remains
        }
    }
}

use loom_payload_guard::FixedGraph;

#[test]
fn loom_payload_guard_excludes_delete_during_edge_write() {
    loom::model(|| {
        let graph = Arc::new(FixedGraph::new());
        graph.seed(1);
        graph.seed(2);

        let g1 = Arc::clone(&graph);
        let t1 = thread::spawn(move || {
            g1.add_edge(1, 2);
        });

        let g2 = Arc::clone(&graph);
        let t2 = thread::spawn(move || {
            g2.delete(1);
        });

        t1.join().unwrap();
        t2.join().unwrap();

        // Whichever order the two threads actually ran in, the fixed lock
        // nesting must never leave an edge whose endpoint payload is gone.
        assert!(
            !graph.has_phantom_edge(1),
            "fixed lock order must never leave a phantom edge"
        );
    });
}
