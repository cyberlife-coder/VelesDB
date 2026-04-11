# VelesDB Concurrency Model

> **EPIC-023**: Documentation du modèle de concurrence pour utilisateurs avancés et contributeurs.

> **User-facing write throughput guidance**: see
> [`docs/guides/WRITE_CONCURRENCY.md`](guides/WRITE_CONCURRENCY.md) for
> the single-writer-per-collection model, batching patterns, and the
> Community/Enterprise split. This document covers the internal lock
> ordering and concurrency primitives used across the engine.

## Overview

VelesDB utilise un modèle de concurrence basé sur:
- **Sharding**: Partitionnement des données pour réduire la contention
- **RwLock**: Lecture parallèle, écriture exclusive (parking_lot)
- **Lock-free atomics**: For compteurs, métriques, HNSW entry-point promotion, and CSR snapshot swap
- **ArcSwap**: Lock-free CSR snapshot reads for graph traversal (zero contention on reads)
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
| CsrSnapshot | `ArcSwap<Arc<CsrSnapshot>>` | None | Lock-free reads via atomic swap; lazy rebuild on dirty flag |
| RaBitQ index | `parking_lot::RwLock` | None (after training) | Write-once then read-only |
| RaBitQ store | `parking_lot::RwLock` | Low | Write per insert (~10ns hold) |
| RaBitQ training buffer | `parking_lot::Mutex` | Low | Pre-training only |
| MmapStorage (compaction) | `parking_lot::RwLock` | High (during compaction) | Exclusive write lock for full compaction duration |

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
   - Mitigation: On `Collection::open()`, gap detection compares `storage.ids()` against `index.mappings` and re-indexes any missing vectors. See [HNSW Crash Recovery](#hnsw-crash-recovery) for the full recovery architecture and [SOUNDNESS.md](SOUNDNESS.md#hnsw-batch-insertion-ordering) for batch insertion ordering invariants.

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

## HNSW Crash Recovery

### Problem Statement

HNSW graph persistence is intentionally deferred: `Collection::flush()` only
saves the HNSW graph to disk when `inserts_since_last_hnsw_save` exceeds
`HNSW_SAVE_THRESHOLD` (10 000 inserts). This amortizes the cost of
serializing the full HNSW graph (metadata, mappings, vectors, and graph
structure) across many write operations instead of paying it on every flush.

The trade-off is an enlarged crash recovery window: if the process crashes
between a vector storage write and the next HNSW save, the HNSW index on
disk will be missing those vectors. Two complementary recovery layers
ensure no data is lost.

### Recovery Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        Collection::open()                              │
│                                                                        │
│  1. MmapStorage::new()                                                 │
│     ├─ Load vectors.idx (ID → offset mapping)                          │
│     ├─ Replay vectors.wal → restore writes since last flush_index()    │
│     └─ Truncate WAL after successful replay                            │
│                                                                        │
│  2. load_or_create_hnsw()                                              │
│     └─ Load hnsw.bin (graph), native_mappings.bin, native_vectors.bin  │
│                                                                        │
│  3. reconcile_point_count()                                            │
│     └─ Set config.point_count = storage.len() (authoritative source)   │
│                                                                        │
│  4. run_crash_recovery()                                               │
│     └─ recover_hnsw_gap(vector_storage, index, dimension)              │
│        ├─ Early exit: if storage.len() == hnsw.len() → no gap          │
│        ├─ find_gap_ids: storage.ids() \ index.mappings                 │
│        ├─ retrieve_valid_vectors: load from mmap, validate dimension   │
│        └─ reindex_vectors: insert_batch_parallel into HNSW             │
└─────────────────────────────────────────────────────────────────────────┘
```

### Layer 1: Vector Storage WAL Replay

**Module**: `crates/velesdb-core/src/storage/mmap/wal_replay.rs`

Every `MmapStorage::store()` and `store_batch()` call writes a CRC32-framed
entry to `vectors.wal` before updating the mmap and in-memory index. The
WAL format uses a single-byte opcode prefix:

| Op | Name | Frame Layout |
|----|------|-------------|
| `0x01` | Store | `[op:1B][id:8B LE][len:4B LE][data:N B][crc32:4B LE]` |
| `0x02` | Delete | `[op:1B][id:8B LE][crc32:4B LE]` |

On `MmapStorage::new()`, the constructor calls `replay_wal_to_index()` which:

1. Opens `vectors.wal` and validates it uses the CRC32-framed format
   (legacy pre-#317 WAL files without CRC are detected and skipped).
2. Reads entries sequentially, verifying each CRC32 checksum. A CRC
   mismatch or truncated entry indicates a crash mid-write; replay stops
   at the corruption boundary (all prior valid entries are applied).
3. For store entries: writes the vector data into the mmap at the correct
   offset and updates the sharded index.
4. For delete entries: removes the ID from the sharded index.
5. Truncates the WAL file to zero after successful replay, preventing
   double-replay on the next startup.

This layer recovers vectors that were written to the WAL but not yet
persisted to `vectors.idx` (the index file is only written by
`flush_index()` or `flush_full()`, not by the fast `flush()` path).

### Layer 2: HNSW Gap Detection

**Module**: `crates/velesdb-core/src/collection/core/recovery.rs`

After storage is fully reconstructed (Layer 1), `Collection::open()` calls
`run_crash_recovery()`, which invokes `recover_hnsw_gap()`:

1. **Early exit heuristic**: If `storage.len() == 0` or
   `storage.len() == hnsw.len()`, returns 0 (no gap). This check is
   O(1) and avoids a full scan in the common case.

2. **Gap ID detection** (`find_gap_ids`): Iterates all IDs in
   `storage.ids()` and filters those not present in
   `index.mappings.contains(id)`. This is O(storage_count) with O(1)
   per-ID lookup in the sharded mappings.

3. **Vector retrieval** (`retrieve_valid_vectors`): Loads each gap
   vector from mmap storage, skipping entries with mismatched dimension
   (corruption) or missing data (concurrent deletion between `ids()` and
   `retrieve()` calls).

4. **Re-indexing** (`reindex_vectors`): Batch-inserts all valid gap
   vectors into the HNSW graph via `insert_batch_parallel`. The re-index
   uses the same parallel rayon-based insertion as normal upserts.

### Gap Sources

Three distinct write paths can leave vectors in storage but absent from HNSW:

| Gap Source | Mechanism | Typical Window |
|-----------|-----------|---------------|
| **Normal insert gap** | `batch_store_all` writes vectors before `bulk_index_or_defer` inserts into HNSW. Crash between Phase 1 and Phase 3 of the 3-phase upsert pipeline. | Duration of Phase 2 (secondary indexes, quantization, text indexing) |
| **Deferred indexer gap** | `DeferredIndexer` buffers vectors in memory (up to `merge_threshold`, default 1 024) before batch-merging into HNSW. Crash before merge loses the buffer. | Up to `merge_threshold` vectors (memory-only, not WAL-protected) |
| **Delta buffer gap** | `DeltaBuffer` accumulates vectors during background HNSW rebuild. Crash before `deactivate_and_drain` loses the buffer. | Duration of the rebuild operation |

All three gaps are recovered by the same `recover_hnsw_gap` mechanism
because the recovery compares the final storage state against the HNSW
mappings, regardless of how the gap originated.

### Known Limitation: Delete-Insert Ambiguity

If a crash occurs between an HNSW delete and the corresponding storage
delete being persisted, a previously deleted vector may appear in storage
but not in HNSW. This is indistinguishable from an insert gap. Recovery
will re-index the deleted vector, effectively "resurrecting" it. This is
an intentional trade-off: resurrecting a deleted vector is preferable to
silently losing an inserted one. The window for this scenario is very
small (within a single `delete()` call).

### Startup Latency Impact

Recovery latency depends on the number of gap vectors:

| Gap Size | Expected Recovery Time | Dominant Cost |
|----------|----------------------|---------------|
| 0 (no gap) | < 1 ms | O(1) early exit heuristic |
| 1–100 vectors | < 10 ms | Storage retrieval + HNSW insert |
| 100–1 000 vectors | 10–100 ms | Parallel HNSW batch insert |
| 1 000–10 000 vectors | 100 ms–1 s | Parallel HNSW batch insert (rayon) |
| > 10 000 vectors | > 1 s | Proportional to gap size; mitigated by `HNSW_SAVE_THRESHOLD` |

The `HNSW_SAVE_THRESHOLD` (10 000) bounds the maximum gap size in practice:
`flush()` forces an HNSW save after 10 000 inserts, so the worst-case
recovery inserts at most ~10 000 vectors into the graph. A graceful
shutdown via `flush_full()` saves the HNSW graph unconditionally, reducing
the gap to zero for planned restarts.

### Configuration Knobs

| Parameter | Default | Location | Effect |
|-----------|---------|----------|--------|
| `HNSW_SAVE_THRESHOLD` | 10 000 | `Collection::flush()` in `flush.rs` | Maximum inserts before `flush()` forces an HNSW save. Lower values reduce worst-case recovery time but increase flush latency. |
| `DurabilityMode` | `Fsync` | `MmapStorage` | Controls WAL write behavior. `Fsync`: full durability. `FlushOnly`: user-space flush only (faster, risk of OS-crash data loss). `None`: no WAL writes (for bulk import; no WAL replay possible). |
| `DeferredIndexerConfig.merge_threshold` | 1 024 | `collection.streaming.deferred` | Number of buffered vectors before deferred merge into HNSW. Larger values increase the deferred indexer gap window. |
| `DeferredIndexerConfig.max_buffer_age_ms` | 5 000 | `collection.streaming.deferred` | Maximum age of buffered vectors before a time-based merge. Provides a time bound on the deferred gap. |

### Flush Variants

| Method | WAL fsync | mmap flush | vectors.idx | HNSW save | Use Case |
|--------|-----------|-----------|-------------|-----------|----------|
| `Collection::flush()` | Yes | Yes | No | Only if > 10K inserts | Normal operation, periodic durability |
| `Collection::flush_full()` | Yes | Yes | Yes | Always | Graceful shutdown, before compaction |
| `MmapStorage::flush()` | Yes | Yes | No | N/A | Storage-level fast barrier |
| `MmapStorage::flush_full()` | Yes | Yes | Yes | N/A | Storage-level complete barrier |

### Persistence Format

HNSW index persistence uses atomic write-tmp-fsync-rename for crash safety.
Each save writes four files:

| File | Contents | Format |
|------|----------|--------|
| `native_meta.bin` | Dimension, metric, vector storage flag, storage mode | postcard-serialized tuple |
| `native_mappings.bin` | `id_to_idx`, `idx_to_id`, `next_idx` | postcard-serialized HashMaps |
| `native_vectors.bin` | `Vec<(internal_idx, Vec<f32>)>` | postcard-serialized vector pairs |
| `native_hnsw` (dir/file) | Graph structure (layers, edges, neighbors) | Custom binary via `file_dump` |

### HNSW Delta WAL (Incremental Graph Logging)

**Module**: `crates/velesdb-core/src/storage/hnsw_delta_wal.rs`

In addition to the vector storage WAL, VelesDB provides an HNSW delta WAL
that logs incremental graph mutations (edge additions, edge removals,
entry-point changes). This enables O(delta) recovery instead of full graph
rebuild O(N*M).

| Op | Name | Frame Layout |
|----|------|-------------|
| `0x01` | AddEdge | `[op:1B][from:4B LE][to:4B LE][layer:1B][crc32:4B LE]` (14 bytes) |
| `0x02` | RemoveEdge | `[op:1B][from:4B LE][to:4B LE][layer:1B][crc32:4B LE]` (14 bytes) |
| `0x03` | SetEntry | `[op:1B][node:4B LE][max_layer:1B][crc32:4B LE]` (10 bytes) |

Each entry is CRC32-framed. On recovery, `HnswDeltaReader::read_all()`
reads entries sequentially until EOF or the first corrupted frame, which
marks the crash boundary.

### Test Coverage

Recovery behavior is validated by the following test suite:

| Test | File | Scenario |
|------|------|----------|
| `test_no_gap_returns_zero` | `recovery_tests.rs` | No gap: storage and HNSW counts match |
| `test_empty_collection_no_recovery` | `recovery_tests.rs` | Empty collection: early exit |
| `test_crash_gap_detected_and_recovered` | `recovery_tests.rs` | Simulated gap: 2 vectors in storage but not HNSW |
| `test_gap_recovery_on_collection_reopen` | `recovery_tests.rs` | End-to-end: create, gap, flush, drop, reopen, verify search |
| `test_metadata_only_skips_recovery` | `recovery_tests.rs` | Metadata-only collections skip recovery |
| WAL replay tests | `wal_recovery_tests.rs` | CRC validation, legacy format detection, truncation |
| HNSW delta WAL tests | `hnsw_delta_wal_tests.rs` | Delta entry serialization, CRC verification, crash boundary |

## Storage Compaction Concurrency

### Exclusive Lock Scope

Storage compaction holds the `MmapStorage` write lock for the entire
duration of the operation. This is enforced at two levels:

1. **Synchronous path** (`MmapStorage::compact(&mut self)`): The method
   takes `&mut self`, so the caller must already hold an exclusive
   reference. No concurrent reads or writes are possible while compaction
   runs.

2. **Asynchronous path** (`compact_async(storage: Arc<RwLock<MmapStorage>>)`):
   Acquires `storage.write()` inside a `spawn_blocking` task and holds
   the write guard for the full compaction cycle. All readers and writers
   on the same `RwLock` are blocked until the guard is dropped.

```
compact_async()
├─ spawn_blocking
│  ├─ storage.write()          ← exclusive lock acquired
│  ├─ MmapStorage::compact()   ← rewrite active vectors to .tmp
│  │   ├─ build temp file
│  │   ├─ copy active vectors
│  │   ├─ atomic_replace(.tmp → .dat)
│  │   └─ rebuild index + flush
│  └─ drop(guard)              ← exclusive lock released
```

### Latency Impact

On large collections (>1M vectors), compaction rewrites the entire active
vector set to a new file and atomically replaces the original. This can
block all reads and writes for seconds, depending on disk throughput and
vector dimensionality. This is an intentional correctness-over-performance
trade-off: holding the exclusive lock prevents readers from observing a
partially rewritten file and writers from appending to a file that is about
to be replaced.

### Crash Recovery

`recover_compaction_artifacts()` runs automatically during
`MmapStorage::new()` to repair any interrupted compaction. The recovery
logic inspects leftover intermediate files:

| State on Disk | Interpretation | Recovery Action |
|---------------|---------------|-----------------|
| `.bak` exists, original missing | Crash after rename-to-backup, before new file swap | Restore `.bak` as original |
| `.bak` exists, original exists | Compaction completed, backup not yet cleaned up | Remove `.bak` |
| `.tmp` exists | Incomplete compaction (temp file never swapped in) | Remove `.tmp` |

This ensures the storage directory is always in a consistent state before
the mmap file is opened, regardless of when the previous process crashed.

**Module**: `crates/velesdb-core/src/storage/compaction.rs`
(`recover_compaction_artifacts`, `atomic_replace`)

### Future Roadmap

Copy-on-write compaction that allows concurrent reads during the rewrite
phase is planned for the enterprise edition. The current exclusive-lock
design is the baseline for correctness validation.

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

*Last updated: 2026-04-09 (Storage compaction concurrency documentation)*
