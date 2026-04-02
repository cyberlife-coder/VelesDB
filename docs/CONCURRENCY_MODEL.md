# VelesDB Concurrency Model

> **EPIC-023**: Documentation du modèle de concurrence pour utilisateurs avancés et contributeurs.

## Overview

VelesDB utilise un modèle de concurrence basé sur:
- **Sharding**: Partitionnement des données pour réduire la contention
- **RwLock**: Lecture parallèle, écriture exclusive (parking_lot)
- **Lock-free atomics**: Pour compteurs, métriques, and HNSW entry-point promotion
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
| HNSW entry point | `AtomicUsize` | None | Lock-free CAS promotion |
| HNSW max layer | `AtomicUsize` | None | Lock-free CAS promotion |
| Metrics counters | `AtomicU64` | None | Lock-free |
| Edge ID registry | `RwLock<HashMap>` | Low | Global, for existence checks |
| CsrSnapshot | `Option<CsrSnapshot>` | None (after build) | Built on-demand, invalidated by writes |
| RaBitQ index | `parking_lot::RwLock` | None (after training) | Write-once then read-only |
| RaBitQ store | `parking_lot::RwLock` | Low | Write per insert (~10ns hold) |
| RaBitQ training buffer | `parking_lot::Mutex` | Low | Pre-training only |

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

### Global Lock Order (HNSW + Storage)

For HNSW index operations that touch vector storage, columnar metadata,
graph layers, and neighbor lists, the global lock acquisition order is:

```
vectors (rank 10) → columnar (rank 15) → layers (rank 20) → neighbors (rank 30)
```

| Lock | Rank | Component | Notes |
|------|------|-----------|-------|
| `vectors` | 10 | `ShardedVectors` / `ContiguousVectors` | Acquired first in upsert and search paths |
| `columnar` | 15 | `ColumnStore` / typed columns | Metadata columns, acquired after vectors |
| `layers` | 20 | HNSW layer structure (`RwLock`) | Global graph topology |
| `neighbors` | 30 | Per-node neighbor lists (`RwLock`) | Fine-grained, acquired last |

**Rule**: Never acquire a lower-rank lock while holding a higher-rank lock.
For example, acquiring `vectors` while holding `neighbors` is forbidden.

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

## RaBitQ Interior Mutability

### Lock Layout

`RaBitQPrecisionHnsw` uses interior mutability for its quantization index,
encoded vector store, and pre-training buffer:

| Field | Type | Access Pattern |
|-------|------|---------------|
| `rabitq_index` | `RwLock<Option<Arc<RaBitQIndex>>>` | Write-locked once during training, then read-only |
| `rabitq_store` | `RwLock<Option<RaBitQVectorStore>>` | Write during insert (push encoded vector) |
| `training_buffer` | `Mutex<Vec<Vec<f32>>>` | Write during pre-training inserts |

### Training Lock Order

During `train_rabitq()`, locks are acquired and released in this order:

```
rabitq_index.write() → rabitq_store.write() → training_buffer.lock()
```

Locks are released between acquisitions (not held simultaneously). The
training function uses a **double-check locking** pattern: it first checks
`rabitq_index` under a read lock, and if training is needed, re-checks
under a write lock to prevent duplicate training from concurrent threads.

### Store-Before-Index Ordering

When training completes, the store is set BEFORE the index:

```
rabitq_store.write() ← set encoded vectors
rabitq_index.write() ← set trained index
```

This ordering prevents an inconsistent snapshot where a search thread sees
a trained index but an empty store. A search thread that acquires
`rabitq_index.read()` and sees `Some(...)` is guaranteed that
`rabitq_store.read()` also contains `Some(...)` with all pre-training
vectors already encoded.

## RaBitQ Contention Analysis

### Known Contention Patterns

| Pattern | Impact | Acceptable? |
|---------|--------|-------------|
| `rabitq_store.write()` serializes post-training inserts | ~10ns per push (single encoded vector append) | Yes: store push is a trivial `Vec::push` operation |
| Training blocks all inserts for ~60ms | One-time event per index lifetime | Yes: training runs once when the buffer threshold is reached |
| `reorder_for_locality()` is offline-only | Takes `&self` but must not run during concurrent search | Yes: only called during explicit maintenance, not on the hot path |

### Mitigation

- Post-training inserts hold `rabitq_store.write()` for the minimum
  duration needed to push a single encoded vector (~10ns).
- Training is amortized: it runs once when the training buffer reaches its
  threshold, then never again for the lifetime of the index.
- `reorder_for_locality()` is documented as offline-only and is not exposed
  through any concurrent API path.

## HNSW Entry-Point CAS Promotion

### Lock-Free Entry-Point Updates

HNSW entry-point promotion (selecting which node is the graph entry) uses
lock-free atomic CAS (compare-and-swap) instead of a mutex. The entry point
and max layer are stored as `AtomicUsize` fields in `NativeHnsw`:

```rust
entry_point: AtomicUsize,  // NO_ENTRY_POINT (usize::MAX) when empty
max_layer: AtomicUsize,    // Current maximum layer
```

`promote_entry_point()` handles two cases with CAS:

1. **Empty index**: CAS on `entry_point` from `NO_ENTRY_POINT` to the new
   node ID. Only one thread wins the first-insert race.
2. **Layer promotion**: CAS on `max_layer` from `current_max` to `node_layer`.
   Only the CAS winner updates `entry_point`, ensuring consistency.

**Transient inconsistency window**: Between the `max_layer` CAS success and
the subsequent `entry_point` store, a concurrent reader may see the new
`max_layer` with the old `entry_point`. This is safe: `search_layer_single`
returns `None` (via `with_neighbors`) for layers where the old EP has no
edges, causing a no-op greedy descent.

Entry-point promotion is extremely rare (O(log_M(N)) times per index
lifetime), so the CAS loop almost never retries.

## CsrSnapshot Invalidation Pattern

### Graph Edge CSR Read Snapshot

`EdgeStore` and `ConcurrentEdgeStore` maintain an optional `CsrSnapshot` --
a Compressed Sparse Row representation of outgoing edges for zero-copy
neighbor access during BFS/DFS traversal.

**Lifecycle**:

1. **Build**: `build_read_snapshot()` materializes edges into contiguous
   arrays (`targets: Vec<u64>`, `edge_ids: Vec<u64>`) with a
   `FxHashMap<u64, (offset, len)>` index. Auto-built after loading from
   disk and after `flush()`.
2. **Read**: `csr_snapshot().neighbors(source_id)` returns `&[u64]` -- a
   zero-copy slice into the contiguous target array.
3. **Invalidate**: Every write operation (`add_edge`, `remove_edge`,
   `remove_node_edges`) sets `csr_snapshot = None`. Subsequent reads fall
   back to per-shard edge lookup until the snapshot is rebuilt.

**Thread safety** (ConcurrentEdgeStore): The snapshot is stored under the
same `RwLock` as the shard data. A read lock on the snapshot shard is
sufficient for BFS access. Write operations acquire write locks and
invalidate the snapshot as part of the same critical section.

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

- **CsrSnapshot fast-path**: When a CSR read snapshot is available (built
  after load or after `build_read_snapshot()`), BFS/DFS reads neighbors via
  a contiguous `&[u64]` slice instead of per-shard edge lookup. Falls back
  to the shard-based path when the snapshot is invalidated by writes.

- **Parent-pointer path reconstruction**: BFS traversal uses a
  `FxHashMap<u64, (u64, u64)>` parent-pointer map instead of cloning path
  vectors at every edge expansion. Paths are reconstructed on-demand via
  `reconstruct_path()` only when a result is emitted, avoiding O(depth)
  allocations per expansion step.

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

5. **Enlarged crash recovery window during batch upsert**:
   - The 3-phase upsert pipeline (`batch_store_all` -> `per_point_updates` -> `bulk_index_or_defer`) writes vectors and payloads to storage before inserting into the HNSW graph. A crash between Phase 1 and Phase 3 leaves vectors in storage but missing from the HNSW index.
   - Mitigation: On `Collection::open()`, gap detection compares `storage.ids()` against `index.mappings` and re-indexes any missing vectors. See [Crash Recovery Testing](contributing/CRASH_RECOVERY_TESTING.md) and [SOUNDNESS.md](SOUNDNESS.md#hnsw-batch-insertion-ordering) for details.

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

### HNSW Batch Insertion Ordering

For soundness analysis of the batch insertion pipeline and its ordering
invariants, see [SOUNDNESS.md: HNSW Batch Insertion Ordering](SOUNDNESS.md#hnsw-batch-insertion-ordering).

## References

- [Rust Atomics and Locks (Mara Bos)](https://marabos.nl/atomics/)
- [The Rustonomicon - Concurrency](https://doc.rust-lang.org/nomicon/concurrency.html)
- [parking_lot documentation](https://docs.rs/parking_lot/)
- [Loom crate](https://github.com/tokio-rs/loom)

---

*Last updated: 2026-04-02 (EPIC-023 + RaBitQ concurrency + CAS entry-point + CsrSnapshot + parent-pointer BFS)*
