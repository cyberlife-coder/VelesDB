# Concurrency & Locking Guide

This guide explains how VelesDB handles concurrent access, file locking,
and thread safety. It covers single-process multi-threading, multi-process
protection, and best practices for production deployments.

> **Looking for write throughput tuning?** See
> [Write Concurrency Model](WRITE_CONCURRENCY.md) for the single-writer-per-collection
> model, batching patterns, and Enterprise tier options.

## Table of Contents

- [Quick Start](#quick-start)
- [File-Level Locking (Multi-Process)](#file-level-locking-multi-process)
- [Thread Safety (Multi-Thread)](#thread-safety-multi-thread)
- [Read vs Write Concurrency](#read-vs-write-concurrency)
- [Graph Edge Sharding](#graph-edge-sharding)
- [Storage Durability Modes](#storage-durability-modes)
- [Error Codes](#error-codes)
- [Best Practices](#best-practices)
- [FAQ](#faq)

---

## Quick Start

VelesDB is designed for **single-process, multi-threaded** access:

```rust
use std::sync::Arc;
use velesdb_core::Database;

// Open the database (acquires exclusive file lock)
let db = Arc::new(Database::open("./my_data")?);

// Share across threads safely — Database is Send + Sync
let db_clone = Arc::clone(&db);
std::thread::spawn(move || {
    let results = db_clone.search("documents", &query_vec, 10).unwrap();
});
```

**Key rules:**
- One process per database directory (enforced by OS-level file lock)
- Multiple threads can read and write concurrently within that process
- Reads scale linearly with thread count
- Batch writes for best throughput

---

## File-Level Locking (Multi-Process)

### How It Works

When you call `Database::open("./data")`, VelesDB creates a lock file
at `./data/velesdb.lock` and acquires an **exclusive OS-level lock**
using the `fs2` crate. This prevents any other process from opening the
same database directory.

```
./data/
  velesdb.lock      <-- Exclusive lock held by the process
  my_collection/
    config.json
    vectors.bin
    ...
```

### What Happens on Conflict

If a second process tries to open the same directory:

```rust
let db2 = Database::open("./data");
// Returns: Err([VELES-031] Database is already opened by another process: ./data)
```

The lock is **non-blocking** — it fails immediately rather than waiting.
This is intentional: VelesDB is a local-first embedded database, not a
client-server system.

### Lock Release

The lock is released automatically when the `Database` instance is dropped:

```rust
{
    let db = Database::open("./data")?;
    // ... use db ...
} // Lock released here (RAII)

// Another process can now open ./data
let db2 = Database::open("./data")?; // OK
```

If the process crashes, the OS automatically releases the file lock.
No stale lock files are left behind.

### Multi-Process Alternatives

If you need multiple processes to access VelesDB data:

| Approach | Use Case |
|----------|----------|
| **velesdb-server** (REST API) | Multiple clients via HTTP |
| **Separate directories** | Each process owns its own data |
| **Process coordination** | Open/close sequentially with external lock |

---

## Thread Safety (Multi-Thread)

### All Types Are `Send + Sync`

`Database`, `VectorCollection`, `GraphCollection`, and `MetadataCollection`
all implement `Send + Sync`. You can safely share them across threads
using `Arc<Database>`.

### Lock Hierarchy

VelesDB uses `parking_lot::RwLock` (not `std::sync::RwLock`) throughout.
Benefits:
- **No poisoning** — a panicking thread does not corrupt locks
- **Multiple concurrent readers** — reads never block each other
- **Fast** — optimized for read-heavy workloads

Internally, each collection maintains independent locks for different
subsystems:

```
Database
  |-- collections registry (RwLock)     -- short-lived, metadata only
  |-- vector_colls registry (RwLock)
  |-- graph_colls registry (RwLock)
  |-- schema_version (AtomicU64)        -- lock-free
  |
  +-- Collection "docs"
        |-- config (RwLock)             -- rarely written
        |-- vector_storage (RwLock)     -- mmap-backed vectors
        |-- payload_storage (RwLock)    -- append-only WAL
        |-- hnsw_index (internal RwLock) -- read-heavy search
        |-- edge_store (sharded RwLocks) -- graph edges
        |-- query_cache (lock-free)     -- parsed query cache
        +-- write_generation (AtomicU64) -- lock-free counter
```

**Lock ordering** is strictly enforced to prevent deadlocks. The full
ordering is documented in the source code and `docs/CONCURRENCY_MODEL.md`.

---

## Read vs Write Concurrency

### Reads (Search, Query, Traverse)

- Multiple threads can search **simultaneously** with zero contention
- The HNSW index uses a `RwLock` that allows unlimited concurrent readers
- ID-to-index mappings use `DashMap` (lock-free concurrent hash map)
- Parsed query plans are cached lock-free

**Scalability:** Search throughput scales linearly with thread count.
On an 8-core machine, 8 concurrent search threads achieve ~8x the
throughput of a single thread.

### Writes (Upsert, Delete)

- Writes acquire a **write lock** on the affected subsystems
- A write blocks other writes to the same collection, but only briefly
- Writes do **not** block reads on the HNSW index (index rebuild is
  deferred via a delta buffer)
- Payload writes are append-only (WAL), minimizing lock duration

### Mixed Workloads

VelesDB is optimized for **read-heavy, write-light** workloads typical
of AI/RAG applications:

| Operation | Lock Type | Duration | Blocks Reads? |
|-----------|-----------|----------|---------------|
| Vector search | Read | Microseconds | No |
| Graph traversal | Read (per-shard) | Microseconds | No |
| Payload lookup | Read | Microseconds | No |
| Single upsert | Write | ~100us | Briefly |
| Batch upsert (1000) | Write | ~10ms | Briefly |
| Collection create | Write (registry) | ~1ms | No (different lock) |
| HNSW rebuild | Write (index) | Seconds | Yes (rare) |

---

## Graph Edge Sharding

For graph-heavy workloads, `ConcurrentEdgeStore` distributes edges across
**multiple independent shards**, each with its own lock:

```
ConcurrentEdgeStore
  |-- shard[0]  (RwLock<EdgeStore>)  -- nodes 0, 256, 512, ...
  |-- shard[1]  (RwLock<EdgeStore>)  -- nodes 1, 257, 513, ...
  |-- ...
  +-- shard[255] (RwLock<EdgeStore>) -- nodes 255, 511, 767, ...
```

Shard assignment: `shard_index = node_id % num_shards`

### Automatic Shard Sizing

VelesDB automatically sizes the number of shards based on estimated
edge count:

| Estimated Edges | Shards | Rationale |
|----------------|--------|-----------|
| < 1,000 | 1 | No contention at small scale |
| 1K - 64K | 16-64 | Moderate parallelism |
| 64K - 1M | 64-128 | High parallelism |
| > 1M | 256 | Maximum distribution |

### Cross-Shard Operations

Adding an edge between nodes in different shards requires locking both
shards. VelesDB always acquires shard locks in **ascending order** to
prevent deadlocks:

```
Edge: node 5 --> node 300
  shard(5)   = 5 % 256 = 5
  shard(300) = 300 % 256 = 44

  Lock order: shard[5] first, then shard[44]  (5 < 44)
```

---

## Storage Durability Modes

The payload WAL (Write-Ahead Log) supports three durability modes,
each with different locking characteristics:

| Mode | Safety | Performance | Use Case |
|------|--------|-------------|----------|
| `Fsync` (default) | Full durability | Slower writes | Production data |
| `FlushOnly` | Survives crashes, not power loss | Faster | Non-critical data |
| `None` | Best-effort | Maximum throughput | Re-derivable data (embeddings) |

In `Fsync` mode, each write batch issues an `fsync()` call that
forces data to disk. This holds the write lock slightly longer but
guarantees no data loss on crash or power failure.

### Mmap Epoch Protection

Vector storage uses memory-mapped files with an **epoch counter**
(`AtomicU64`). When the mmap is resized (e.g., after a large batch
insert), the epoch increments. Any outstanding read guards from the
previous epoch are invalidated:

```
Thread A: guard = storage.read()     // epoch = 5
Thread B: storage.upsert(big_batch)  // triggers remap, epoch = 6
Thread A: guard.access_vector(42)    // ERROR: EpochMismatch (VELES-026)
Thread A: guard = storage.read()     // re-acquire, epoch = 6, OK
```

This prevents use-after-free on resized memory maps without
requiring exclusive locks during reads.

---

## Error Codes

| Code | Error | Cause | Recovery |
|------|-------|-------|----------|
| `VELES-026` | `EpochMismatch` | Read guard invalidated by mmap resize | Re-acquire the guard and retry |
| `VELES-031` | `DatabaseLocked` | Another process holds the lock file | Close the other process, or use a different data directory |

---

## Best Practices

### 1. Share One Database Instance

```rust
// GOOD: One instance shared via Arc
let db = Arc::new(Database::open("./data")?);
// Pass Arc::clone(&db) to each thread

// BAD: Multiple instances of the same directory
let db1 = Database::open("./data")?;
let db2 = Database::open("./data")?; // ERROR: VELES-031
```

### 2. Batch Your Writes

Each write acquires and releases locks. Batching amortizes this cost:

```rust
// GOOD: One lock acquisition for 1000 points
collection.upsert(points_batch_of_1000)?;

// BAD: 1000 separate lock acquisitions
for point in &points {
    collection.upsert(vec![point.clone()])?;
}
```

**Rule of thumb:** batch 100-10,000 points per upsert call.

### 3. Prefer Read-Heavy Architectures

VelesDB excels when reads far outnumber writes. Typical AI/RAG patterns
fit naturally:

- **Ingest phase:** bulk load embeddings (write-heavy, but one-time)
- **Query phase:** many concurrent similarity searches (read-heavy)

### 4. Use Appropriate Durability

For embeddings that can be re-computed from source documents:

```rust
// Faster writes for re-derivable data
let config = CollectionConfig::new(384, DistanceMetric::Cosine)
    .with_durability(DurabilityMode::FlushOnly);
```

### 5. Size Graph Shards Appropriately

If you know the approximate edge count upfront:

```rust
// Pre-size for 500K edges
let edge_store = ConcurrentEdgeStore::with_estimated_edges(500_000);
// Automatically selects 128 shards
```

### 6. Avoid Long-Held References

Do not hold collection references across `await` points in async code.
Acquire, use, and drop:

```rust
// GOOD: Short-lived borrow
let results = db.search("docs", &query, 10)?;
// Lock released, process results freely

// BAD: Holding a collection guard across async boundaries
let coll = db.get_collection("docs")?;
tokio::time::sleep(Duration::from_secs(1)).await; // Lock held!
let results = coll.search(&query, 10)?;
```

---

## FAQ

### Can I use VelesDB from multiple threads?

Yes. All VelesDB types are `Send + Sync`. Wrap the `Database` in an
`Arc` and clone it for each thread. Reads scale linearly.

### Can I use VelesDB from multiple processes?

No, not on the same data directory. VelesDB enforces single-process
access via OS-level file locking (`velesdb.lock`). For multi-process
access, use `velesdb-server` which exposes a REST API.

### What happens if my process crashes while writing?

The WAL (Write-Ahead Log) ensures durability in `Fsync` mode. On next
`Database::open()`, the WAL is replayed to recover uncommitted writes.
The OS automatically releases the file lock on crash.

Additionally, HNSW gap detection runs on open: if vectors were written
to storage but not yet indexed in the HNSW graph (e.g., due to a crash
during the batch upsert pipeline), they are automatically re-indexed.
See [Crash Recovery Testing](../contributing/CRASH_RECOVERY_TESTING.md)
for details.

### Can reads and writes happen at the same time?

Yes. VelesDB uses `RwLock` which allows multiple concurrent readers.
A write briefly blocks other writers on the same collection, but reads
continue unimpeded on the HNSW index (writes go through a delta buffer
first).

### How do I avoid deadlocks?

You don't need to worry about this — VelesDB enforces strict lock
ordering internally. All internal locks follow a numbered hierarchy
(config < vector_storage < payload_storage < ... < edge_store).
Cross-shard graph operations always lock in ascending shard order.

### Is there a lock-free mode?

The HNSW ID mappings (`DashMap`), query cache, schema version, and
write generation counter are already lock-free. The core data structures
(vectors, payloads, graph edges) require locks for consistency, but
lock duration is minimized through batching and sharding.

---

## Further Reading

- [Concurrency Model (internal)](../CONCURRENCY_MODEL.md) — Full lock
  ordering specification and deadlock prevention proofs
- [Soundness Audit](../SOUNDNESS.md) — Unsafe code audit with
  concurrency invariants
- [Tuning Guide](TUNING_GUIDE.md) — HNSW parameter tuning for
  recall/performance tradeoffs
- [Server Security](SERVER_SECURITY.md) — TLS, authentication, and
  rate limiting for `velesdb-server`
