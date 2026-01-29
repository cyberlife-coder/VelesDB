# VelesDB Concurrency Model

> **EPIC-023**: Documentation du modèle de concurrence pour utilisateurs avancés et contributeurs.

## Overview

VelesDB utilise un modèle de concurrence basé sur:
- **Sharding**: Partitionnement des données pour réduire la contention
- **RwLock**: Lecture parallèle, écriture exclusive (parking_lot)
- **Lock-free atomics**: Pour compteurs et métriques
- **Lock ordering**: Ordre déterministe pour prévenir les deadlocks

## Architecture

### Sharding Strategy

```
┌─────────────────────────────────────────────────────────────────┐
│                    ConcurrentEdgeStore                           │
├─────────┬─────────┬─────────┬─────────┬─────────┬───────────────┤
│ Shard 0 │ Shard 1 │ Shard 2 │ Shard 3 │  ...    │ Shard N-1     │
│ RwLock  │ RwLock  │ RwLock  │ RwLock  │         │ RwLock        │
└─────────┴─────────┴─────────┴─────────┴─────────┴───────────────┘
                              │
                    Shard = hash(node_id) % num_shards
```

**Default shards**: 256 (configurable via `with_shards()` or `with_estimated_edges()`)

**Shard selection**:
- Small graphs (< 1K edges): 1 shard
- Medium graphs (1K-64K): 16-64 shards
- Large graphs (64K-1M): 64-128 shards
- Very large graphs (> 1M): 256 shards

### Lock Types

| Component | Lock Type | Contention | Notes |
|-----------|-----------|------------|-------|
| EdgeStore shards | `parking_lot::RwLock` | Low | Per-shard, fine-grained |
| HNSW layers | `parking_lot::RwLock` | Medium | Global, read-heavy |
| HNSW neighbors | `parking_lot::RwLock` | Medium | Per-node |
| PropertyIndex | `parking_lot::RwLock` | Low | Per-property |
| Metrics counters | `AtomicU64` | None | Lock-free |
| Edge ID registry | `RwLock<HashMap>` | Low | Global, for existence checks |

## Thread Safety Guarantees

### Send + Sync Types

These types are safe to share across threads and can be moved between threads:

```rust
// Safe to share and send
Collection: Send + Sync
HnswIndex: Send + Sync
ConcurrentEdgeStore: Send + Sync
ConcurrentNodeStore: Send + Sync
Database: Send + Sync
```

### !Send Types (Single-Thread Only)

These types contain non-thread-safe internal state:

```rust
// Must stay on creation thread
GraphTraversal: !Send  // Contains references
QueryCursor: !Send     // Iterator state
BfsIterator: !Send     // Traversal state
```

### Compile-Time Verification

VelesDB uses compile-time assertions to verify thread safety:

```rust
// In ConcurrentEdgeStore
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ConcurrentEdgeStore>();
};
```

## Lock Ordering (Deadlock Prevention)

### Rule: Always Acquire Locks in Ascending Order

When multiple locks are needed, acquire them in this order:

```
1. edge_ids (global registry)
2. shards[0]
3. shards[1]
4. ...
5. shards[N-1]
```

### Cross-Shard Operations

When an edge spans two shards (source in shard A, target in shard B):

```rust
// ✅ CORRECT: Ascending order
let (first_idx, second_idx) = if source_shard < target_shard {
    (source_shard, target_shard)
} else {
    (target_shard, source_shard)
};
let mut first = shards[first_idx].write();
let mut second = shards[second_idx].write();
```

```rust
// ❌ WRONG: May cause deadlock
let mut source = shards[source_shard].write();
let mut target = shards[target_shard].write();  // DEADLOCK if another thread holds target first!
```

### Cascade Delete (remove_node_edges)

Uses BTreeSet for automatic ascending order:

```rust
let mut shards_to_clean: BTreeSet<usize> = BTreeSet::new();
// BTreeSet iteration is already sorted ascending
for &idx in &shards_to_clean {
    guards.push(shards[idx].write());
}
```

## Performance vs Safety Tradeoffs

### Read-Heavy Workloads

- `RwLock` allows multiple concurrent readers
- Sharding distributes reads across independent locks
- **Recommendation**: Default 256 shards optimal for most workloads

### Write-Heavy Workloads

- Writers block readers on same shard
- Cross-shard writes require 2 locks
- **Recommendation**: 
  - Use batch inserts to amortize lock overhead
  - Consider `with_estimated_edges()` to optimize shard count

### Graph Traversal

- Uses "Read-Copy-Drop" pattern to minimize lock duration:

```rust
// ✅ CORRECT: Copy data, drop lock immediately
let neighbors: Vec<u64> = {
    let guard = shard.read();
    guard.get_outgoing(node).iter().map(|e| e.target()).collect()
}; // Guard dropped here

for neighbor in neighbors {
    // Process without holding lock
}
```

## Known Limitations

1. **Cross-shard operations hold multiple locks**: 
   - Edge spanning 2 shards requires 2 locks + edge_ids lock
   - Mitigation: Lock ordering prevents deadlocks

2. **Large traversals can block writers**:
   - BFS/DFS with many nodes may hold locks longer
   - Mitigation: Read-Copy-Drop pattern releases locks quickly

3. **HNSW rebuild is single-threaded**:
   - Index rebuild blocks all writes
   - Mitigation: Incremental updates preferred over full rebuild

4. **No transactional semantics**:
   - Operations are atomic per-operation, not per-batch
   - Mitigation: Use flush() for durability checkpoints

## Best Practices

### For Users

1. **Dimension shards appropriately**:
   ```rust
   // For 100K edges
   let store = ConcurrentEdgeStore::with_estimated_edges(100_000);
   ```

2. **Prefer batch operations**:
   ```rust
   // ✅ Better: One lock acquisition
   collection.upsert(vec![point1, point2, point3])?;
   
   // ❌ Worse: Three lock acquisitions
   collection.upsert(vec![point1])?;
   collection.upsert(vec![point2])?;
   collection.upsert(vec![point3])?;
   ```

3. **Limit traversal depth**:
   ```rust
   // Always specify max_depth to prevent runaway traversals
   let nodes = store.traverse_bfs(start, 5);  // Max 5 hops
   ```

### For Contributors

1. **Follow lock ordering strictly**:
   - Document lock order in new concurrent structures
   - Use BTreeSet/BTreeMap for automatic ordering

2. **Use Read-Copy-Drop pattern**:
   - Never hold locks while processing data
   - Copy what you need, release lock, then process

3. **Add compile-time Send+Sync checks**:
   ```rust
   const _: () = {
       const fn assert_send_sync<T: Send + Sync>() {}
       assert_send_sync::<YourNewConcurrentType>();
   };
   ```

4. **Write Loom tests for new concurrent code**:
   ```rust
   #[cfg(loom)]
   #[test]
   fn test_your_concurrent_operation() {
       loom::model(|| {
           // Test concurrent access patterns
       });
   }
   ```

## Testing Concurrency

### Running Loom Tests

```bash
# Run all loom tests
cargo +nightly test --features loom,persistence --test loom_tests

# With limited preemptions (faster)
LOOM_MAX_PREEMPTIONS=2 cargo +nightly test --features loom,persistence --test loom_tests
```

### Stress Testing

```bash
# Run stress tests with multiple threads
cargo test --test stress_concurrency_tests -- --test-threads=1
```

## References

- [Rust Atomics and Locks (Mara Bos)](https://marabos.nl/atomics/)
- [The Rustonomicon - Concurrency](https://doc.rust-lang.org/nomicon/concurrency.html)
- [parking_lot documentation](https://docs.rs/parking_lot/)
- [Loom crate](https://github.com/tokio-rs/loom)

---

*Last updated: 2026-01-29 (EPIC-023)*
