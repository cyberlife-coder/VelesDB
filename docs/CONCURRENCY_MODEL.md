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

The rank ordinals below are **not a documentation-only convention**: they are
first-class, typed ranks defined by the `LockRank` newtype in
`crates/velesdb-core/src/lock_rank.rs`. `LockRank` derives a total ordering
over its ordinals, and the debug-only `assert_lock_order` helper panics on any
out-of-order acquisition in debug builds (compiling to nothing in release, so
there is zero release overhead). This section is the **single authoritative
record** of the total rank ordering; the code constants and this table are kept
in lock-step and MUST NOT diverge.

For HNSW index operations that touch the GPU snapshot cache, vector storage,
the PDX columnar layout, graph layers, and neighbor lists, the global lock
acquisition order is:

```
gpu_vectors_snapshot (rank 5) → vectors (rank 10) → columnar (rank 15)
    → layers (rank 20) → neighbors (rank 30)
```

| Lock | Rank | `LockRank` constant | Component | Notes |
|------|------|---------------------|-----------|-------|
| `gpu_vectors_snapshot` | 5 | `LockRank::GPU_VECTORS_SNAPSHOT` | GPU flat-vector snapshot cache (`Mutex`) | Acquired before `vectors` in the GPU path (`gpu` feature); writers release `vectors` before reacquiring it to invalidate |
| `vectors` | 10 | `LockRank::VECTORS` | `ContiguousVectors` (single vector store since PERF1) | Acquired first among the core HNSW locks in upsert and search paths |
| `columnar` | 15 | `LockRank::COLUMNAR` | `ColumnarVectors` (PDX block-columnar layout of the HNSW vectors) | SIMD-parallel distance layout, acquired after vectors |
| `layers` | 20 | `LockRank::LAYERS` | HNSW layer structure (`RwLock`) | Global graph topology |
| `neighbors` | 30 | `LockRank::NEIGHBORS` | Per-node neighbor lists (`RwLock`) | Fine-grained, acquired last |

**Rule**: Never acquire a lower-rank lock while holding a higher-rank lock.
For example, acquiring `vectors` while holding `neighbors` is forbidden.
`assert_lock_order(previously_held, about_to_acquire)` encodes this rule
directly and fires a debug assertion when it is violated.

### Reserved Premium Rank Range [40, 59]

The inclusive ordinal range **`[40, 59]`** is reserved for premium-owned lock
classes (cluster state, tenant store, server-level locks). Core **never**
assigns a rank at or above `40`, leaving that band exclusively for premium so
it can order its own locks relative to core without collision. The bounds are
exposed as `LockRank::PREMIUM_MIN` (40) and `LockRank::PREMIUM_MAX` (59), and
`LockRank::premium(value)` constructs a premium rank, returning `None` for any
value outside the reserved range. Because all core ranks (5–30) sit strictly
below the premium band, the union of core and premium ranks forms one
authoritative total ordering shared across both engines: premium locks are
always acquired after the core locks whose data they wrap.

| Range | Owner | Ordinals |
|-------|-------|----------|
| Core | `velesdb-core` | 5, 10, 15, 20, 30 |
| Premium (reserved) | `velesdb-premium` | 40–59 inclusive |

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

Deleting a graph node now **cascades to its edges**: the node-delete path
(`collection/core/crud_read_delete.rs`) calls
`ConcurrentEdgeStore::remove_node_edges(id)` so both outgoing and incoming
edges are removed, leaving no dangling edges pointing at a phantom node (#900).

The cascade follows the global lock order: it acquires `edge_ids` **first**,
then the affected shards in **ascending** index order, using a `BTreeSet` whose
iteration is already sorted:

```rust
let mut shards_to_clean: BTreeSet<usize> = BTreeSet::new();
// BTreeSet iteration is already sorted ascending
for &idx in &shards_to_clean {
    guards.push(shards[idx].write());
}
```

The snapshot invalidation / debounced rebuild (see below) runs **after** the
`edge_ids` write lock is released, never while holding it, to avoid deadlocking
against the downstream rebuild lock acquisition.

### Collection-level lock order

Separate from the HNSW `LockRank` total ordering above, `Collection`'s own
fields (`config`, `vector_storage`, `payload_storage`, the quantization
caches, `edge_wal_lock`, the property/range/label indexes, ...) follow a
documentation-enforced ascending order recorded as a plain comment block at
the top of `crates/velesdb-core/src/collection/types.rs` (`=== LOCK
ORDERING ===`). It is not backed by a typed `LockRank`-style assertion —
this section only narrates one addition to it: position **3b**,
`edge_wal_lock`, sitting right after `payload_storage` (3).

`edge_wal_lock` has two acquisition patterns:

- `add_edge`/`add_edges_batch` (`collection/core/graph_api.rs`) hold
  `payload_storage`'s **read** guard from the endpoint referential-integrity
  check through the end of the edge write, and acquire `edge_wal_lock`
  **while that read guard is still held** — so the acquisition order for
  this path is `payload_storage(3) → edge_wal_lock(3b)`.
- `remove_edge` and the delete-cascade path
  (`collection/core/crud_read_delete.rs`'s `cascade_delete_node_edges`)
  acquire `edge_wal_lock` **alone**, with no other collection lock held —
  the delete path has already released its `payload_storage` write guard by
  the time it reaches the cascade.

This closes a race left open by the original #1442 fix: `add_edge` used to
release `payload_storage`'s read guard immediately after checking that both
endpoints exist, then separately acquire `edge_wal_lock` to write the edge.
A concurrent `delete()` of an endpoint could land in that window — its own
write guard acquisition would not contend with anything, since the reader
had already let go — and finish removing the node (and, if the edge
happened to already exist, cascading its removal) before `add_edge` ever
wrote it. The result was a "phantom" edge: present in the edge store,
invisible to `all_node_ids()`/`MATCH`, since both resolve nodes from the
payload store.

Holding `payload_storage`(3) across `edge_wal_lock`(3b) fixes this: a
concurrent `delete()` needs `payload_storage`'s **write** guard, which
cannot be acquired while `add_edge` holds the read guard, so the delete
blocks until the edge is fully durable (WAL + edge-store apply). By the
time the delete proceeds, the edge already exists, so its own cascade sees
and removes it — no phantom, no compensation logic, no rollback. The same
guard is held once for the whole batch in `add_edges_batch`, so "an
endpoint disappears mid-batch" is impossible by construction.

The order stays acyclic: neither `remove_edge` nor the delete cascade ever
acquires `payload_storage` while holding `edge_wal_lock`, so there is no
path back from 3b to 3.

**Residual latency**: writers to `payload_storage` (upserts, deletes) can
now stall behind an in-flight edge write for up to one `fsync` (the edge
WAL append) — typically 0.05–5 ms on an SSD, consistent with the
single-writer-per-collection model this section belongs under (see
[`guides/WRITE_CONCURRENCY.md`](guides/WRITE_CONCURRENCY.md#edge-writes-and-payload-contention)).

**Known limitation (accepted)**: this only protects edges written through
`Collection::add_edge`/`add_edges_batch`. Edges loaded from a pre-existing
WAL/snapshot at `Collection::open` (replay) are trusted as-is and never
re-validated — replay intentionally bypasses referential-integrity
validation so legitimate edge-only databases created before the #1442 fix
keep their data. A follow-up CLI tool to audit/repair such legacy phantom
edges is tracked in issue
[#1469](https://github.com/cyberlife-coder/VelesDB/issues/1469) (see also
`guides/GRAPH_PATTERNS.md`).

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

`install_trained_rabitq()` (quantizer restore at open / `TRAIN QUANTIZER`
live install) follows the same `rabitq_index → rabitq_store →
training_buffer` order. It holds `rabitq_index.write()` for the whole
re-encode so concurrent inserts cannot interleave store pushes, and it
reads the `inner.vectors` snapshot **and releases it** before taking
`rabitq_store.write()` — never waiting on the store lock while holding
`vectors` (a search thread holds `rabitq_store.read()` while acquiring
`inner.vectors.read()`).

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
   `remove_node_edges`) sets `csr_snapshot = None` and increments a
   pending-write counter. Subsequent reads fall back to per-shard edge lookup
   until the snapshot is rebuilt.

**Debounced rebuild** (#905): The actual O(N+E) CSR rebuild (which clones every
edge into a fresh `EdgeStore`) is **debounced** rather than run on the first
read after any write. A mutation only flips the dirty flag and bumps
`pending_writes`; the rebuild is deferred until the accumulated write count
reaches `CSR_REBUILD_WRITE_THRESHOLD` (64). Batch writes count their full size
toward the threshold. While dirty-but-below-threshold, reads remain correct by
falling back to per-shard lookup — debouncing trades a slightly slower fallback
read for avoiding a full rebuild on every interleaved read/write. A completed
rebuild clears the dirty flag and resets the counter to 0.

**Thread safety** (ConcurrentEdgeStore): The snapshot is stored under the
same `RwLock` as the shard data. A read lock on the snapshot shard is
sufficient for BFS access. Write operations acquire write locks and
invalidate the snapshot as part of the same critical section. The deferred
rebuild path acquires `edge_ids` **read-only** (never write) and releases
per-shard read locks promptly, so it never violates the `edge_ids → shards`
ordering and the caller must not hold an `edge_ids` write lock across it.

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
│     ├─ Record WAL-touched ids (drained by step 4, pass 3)              │
│     └─ Truncate WAL after successful replay                            │
│                                                                        │
│  2. load_or_create_hnsw()                                              │
│     ├─ Gate: native_meta.bin present? (commit point, written LAST)     │
│     ├─ Load native_hnsw.graph/.vectors/.gen + native_mappings.bin —   │
│     │  all generation-stamped (#617); a legacy native_vectors.bin      │
│     │  is generation-checked then discarded (PERF1)                    │
│     └─ Load failure or config mismatch → empty index (rebuild below)   │
│                                                                        │
│  3. reconcile_point_count()                                            │
│     └─ Set config.point_count = storage.len() (authoritative source)   │
│                                                                        │
│  4. recover_index_state() — 3-pass reconciliation                      │
│     ├─ Pass 1 (gap): recover_hnsw_gap                                  │
│     │  ├─ Early exit: if storage.len() == hnsw.len() → no gap          │
│     │  ├─ find_gap_ids: storage.ids() \ index.mappings                 │
│     │  ├─ retrieve_valid_vectors: load from mmap, validate dimension   │
│     │  └─ reindex_vectors: insert_batch_parallel into HNSW             │
│     ├─ Pass 2 (orphans): ids in index.mappings \ storage → remove      │
│     ├─ Pass 3 (stale): WAL-touched ids on both sides — re-upsert when  │
│     │  the indexed vector ≠ storage (storage is the source of truth)   │
│     └─ Any pass mutated the index → index.save() before open returns  │
│        (the WAL was truncated; the delta has no other witness)         │
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

### Layer 2: HNSW 3-Pass Reconciliation

**Module**: `crates/velesdb-core/src/collection/core/recovery.rs`

After storage is fully reconstructed (Layer 1) and the persisted HNSW
index is loaded (or an empty one built when the load fails),
`Collection::open()` calls `run_crash_recovery()`, which runs three
passes against the storage state:

**Pass 1 — gap** (`recover_hnsw_gap`):

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

**Pass 2 — orphans** (`remove_orphan_ids`): ids present in
`index.mappings` but absent from storage (a delete reached the vector
WAL but not the next HNSW save) are soft-deleted from the index so the
tombstone cannot resurface in search results.

**Pass 3 — stale** (`reindex_stale_wal_ids`): for every id touched by
the Layer-1 WAL replay that is present on both sides, the indexed
sidecar vector is compared against the storage bytes; on mismatch the
storage value is re-upserted (an upsert landed in the WAL after the
last HNSW save). An index loaded without sidecar vector storage cannot
be compared — when WAL-touched ids overlap its mappings it is replaced
by an empty index and fully rebuilt by pass 1
(`rebuild_if_unverifiable`).

When any pass mutated the index, `Collection::open()` re-saves it
before returning: the vector WAL was truncated during replay, so
without a fresh save the reconciled delta would be undetectable after
the next crash. For the same reason, `compact_vector_storage()` (which
also truncates the WAL) re-saves the HNSW index after compaction.

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
Each save writes six files, every one stamped with the same monotonic
`generation: u64` (#617) so a crash between two renames is detected on
load. `native_meta.bin` is written LAST — its generation is the
authoritative commit point that `load_sidecars` checks the other
artefacts against, and its presence is the gate `load_or_create_hnsw`
uses to attempt a load at all.

| File | Contents | Format |
|------|----------|--------|
| `native_hnsw.vectors` | Vector data in `NodeId` order | Custom binary via `file_dump` |
| `native_hnsw.graph` | Graph structure (layers, neighbors) + params incl. VAMANA `alpha` (header v2) | Custom binary via `file_dump` |
| `native_hnsw.gen` | Graph generation marker | postcard-serialized `u64` |
| `native_mappings.bin` | `id_to_idx`, `idx_to_id`, `next_idx`, generation | postcard-serialized HashMaps |
| `native_vectors.bin` (legacy, pre-PERF1) | `Vec<(internal_idx, Vec<f32>)>`, generation | Read for the generation check only, payload discarded; deleted on the next save |
| `native_meta.bin` | Dimension, metric, vector storage flag, storage mode, generation | postcard-serialized tuple |

### HNSW Delta WAL — Removed from Core (Disposition)

> **Decision**: the standalone `hnsw_delta_wal` module has been **removed
> from `velesdb-core`**. O(delta) fast / warm-standby recovery becomes a
> premium concern, built by consuming the `WalCursor` seam
> (`crates/velesdb-core/src/storage/wal_cursor.rs`) rather than a
> core-owned delta-WAL. This disposition is recorded here, alongside the
> WAL shippability contract, because both concern the WAL/recovery
> boundary.

Core previously carried a standalone `storage/hnsw_delta_wal.rs` module
that logged incremental graph mutations (edge add/remove, entry-point
changes) with the intent of enabling O(delta) graph recovery instead of a
full O(N*M) rebuild. It was **never wired** into the open / flush /
recovery path — `Collection::open()` and the recovery flow described above
never read or wrote it.

**Why it was not wired, and was removed instead of activated:**

- **Dead-in-core infrastructure.** Core recovery correctness is already
  fully provided by the 3-pass reconciliation
  (`collection/core/recovery.rs`) against **storage as the single source of
  truth**. The delta WAL added nothing to correctness; the persisted graph
  load plus reconciliation is sufficient.
- **No speculative infrastructure.** An unexercised WAL format that must be
  kept byte-compatible with a graph format it never observes is exactly the
  silent-drift hazard the disposition set out to eliminate. Wiring it would
  make the HNSW graph a partial source of truth alongside storage,
  re-opening soundness invariants for a performance gain that only matters
  at cluster scale.
- **The O(delta) benefit is an enterprise concern.** Fast failover and
  warm-standby recovery belong to premium's clustering/replication story,
  not the local-first core. Premium can build it by **consuming the
  `WalCursor` seam** — the clean, supported extension point — which unifies
  "shippable WAL" and "delta recovery" behind one open seam rather than two
  half-built mechanisms. Core never depends on premium; premium consumes
  the cursor.

**On-disk / migration impact:** none. Current versions never wrote a
delta-WAL file, so the removal is a pure source deletion with no on-disk
artifact to migrate and no format-compatibility work required.

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

*Last updated: 2026-06-12 (HNSW persisted-graph reload at open; storage compaction concurrency)*
