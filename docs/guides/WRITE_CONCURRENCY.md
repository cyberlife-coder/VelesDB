# Write Concurrency Model

This guide explains how VelesDB handles concurrent writes to collections,
what workloads scale well with the default configuration, and when you
may need the Enterprise tier.

> **Related guides**:
> - [Concurrency & Locking](CONCURRENCY_LOCKING.md) — file locks, thread safety, multi-process rules
> - [Tuning Guide](TUNING_GUIDE.md) — HNSW parameters, quantization modes
> - [Configuration](CONFIGURATION.md) — `VelesConfig` fields and defaults

---

## TL;DR

| Workload | Scaling |
|---|---|
| Multiple threads writing to **different** collections | Linear with CPU cores |
| A single thread (or client) writing to **one** collection, using batched upserts | Hardware-limited (25-30 Kvec/s on 768-dim vectors) |
| Multiple threads writing to the **same** collection | Serialized behind a per-collection writer lock |
| Read-heavy workloads (search, scroll, match) | Fully parallel; multiple readers scale independently of writers |

VelesDB Community Edition uses a **single-writer-per-collection** model.
This is the same model used by every major open-source vector database
(Qdrant OSS, Weaviate Community, Chroma, Milvus OSS). It keeps crash
recovery simple, keeps the WAL format straightforward, and is more than
sufficient for the workloads most teams run.

If your workload needs **concurrent writers on the same collection**
(multi-tenant SaaS with per-tenant concurrent writes, high-ingestion
pipelines with more than 8 simultaneous clients), see the
[Enterprise Tier](#enterprise-tier) section below.

---

## How it works today

### Per-collection writer lock

Each collection owns a `payload_storage` field protected by an exclusive
writer lock. Every mutation (`upsert`, `upsert_bulk`, `delete`, `flush`)
acquires this lock before touching the WAL. The lock serializes writers
within a collection but does not cross collection boundaries — two
collections can be written to in parallel without contention.

```
Client A ──upsert("docs", ...)──┐
                                 ├──> docs.payload_storage.write() ──> WAL
Client B ──upsert("docs", ...)──┘    (one at a time)

Client C ──upsert("kg", ...)─────────> kg.payload_storage.write() ──> WAL
                                       (parallel with docs, different collection)
```

### Batch writes are the fast path

When you upsert a batch (say, 1000 points via `upsert_bulk` or via the
Python `collection.upsert(points)` call with a list), VelesDB:

1. Takes the collection writer lock **once**.
2. Writes all 1000 records sequentially to the WAL buffer.
3. Performs **one** `fsync` at the end, instead of 1000.
4. Releases the lock.

This amortizes the fsync cost across the whole batch. On an NVMe SSD
with 768-dim vectors, this path sustains **25-30 Kvec/s** single-client.

### Where the ceiling is

The ceiling you hit with the Community model is:

- **Single client, large batches**: hardware-limited (CPU + fsync
  throughput). No software-side improvement available in Community.
- **Multiple concurrent clients on the same collection**: each client
  waits its turn on the collection writer lock. Effective throughput
  depends on batch size and contention pattern — if each client sends
  large batches, the aggregate throughput still approaches the
  hardware ceiling. If each client sends small batches, throughput
  suffers from per-batch overhead.
- **Multiple collections**: parallelism is linear. If you split
  `docs_tenant_1`, `docs_tenant_2`, ... into separate collections, each
  gets its own writer lock and they run in parallel.

---

## Best practices for Community scale

### Pattern 1 — Batch client-side

Instead of:

```python
for point in stream:
    collection.upsert([point])  # one-by-one, 1 lock + 1 fsync each
```

Do:

```python
buffer = []
for point in stream:
    buffer.append(point)
    if len(buffer) >= 1000:
        collection.upsert(buffer)  # one lock, one fsync
        buffer.clear()
if buffer:
    collection.upsert(buffer)
```

This moves the batching into your application layer and gets you the
full hardware throughput with a single-client model.

### Pattern 2 — Shard by collection

If you have naturally partitioned data (by tenant, by time bucket, by
language), use one collection per partition:

```python
db.create_collection(f"docs_{tenant_id}", dimension=768, metric="cosine")
```

Queries can be fan-out to multiple collections and merged client-side
(or via `match_batch` for parallel search). Writes scale linearly with
the number of collections and CPU cores.

### Pattern 3 — Use the async ingestion queue (when available)

If you enable `DeferredIndexer` or `AsyncIndexBuilder` (currently
internal APIs, see [CORE_WIRING_DEBT.md](../CORE_WIRING_DEBT.md)), the
upsert path buffers in memory and flushes in larger batches behind the
scenes. This reduces lock acquisition frequency for streaming workloads.

### Anti-pattern — Many small writers on one collection

If your architecture has 16 Python workers all writing individual
points to the same collection, you will see contention on the writer
lock. Options:

- **Application-side buffering**: have each worker batch 1000 points
  before calling `upsert`.
- **Sharding**: give each worker its own collection, merge at search
  time.
- **Enterprise tier**: use VelesDB Enterprise, which lifts the
  single-writer limit (see below).

---

## Enterprise tier

VelesDB Enterprise (via `velesdb-premium`) includes a lock-free WAL
with leader-follower flush that enables **N writers per collection**
without serialization. The feature is designed for:

- **Multi-tenant SaaS** with concurrent write paths from many tenants
  sharing a single collection.
- **High-ingestion pipelines** with 8+ simultaneous clients that
  cannot batch client-side (typically event-driven workloads).
- **Agent swarms** with many parallel agents writing to a shared
  memory collection (semantic + episodic patterns from the Agent
  Memory SDK).

**Expected benefit**: 2-4x aggregate write throughput on 8+ concurrent
writers per collection. Single-client bulk imports are unchanged
(already at the hardware ceiling with the Community batched path).

**Other Enterprise features** (see velesdb.com/enterprise for the
full list):

- Advanced RBAC with per-role audit logging
- Multi-tenant isolation with per-tenant resource quotas
- Priority support and SLA-backed response times
- Early access to experimental optimizations

**When to consider Enterprise**: if you have benchmarked your workload
and single-writer-per-collection is a genuine bottleneck (most teams
discover they can solve it with batching or sharding first).

**Where the limit is documented internally**: the architectural
analysis lives in `docs/CORE_WIRING_DEBT.md` (engineering debt
catalogue). Community stays stable and predictable; Enterprise delivers
the specialized concurrency model for the workloads that need it.

---

## FAQ

### Is the single-writer limit a bug?

No. It is the standard model for open-source vector databases and for
most embedded or local-first databases (SQLite uses single-writer,
many readers). It is intentional because it keeps WAL recovery simple
and avoids a whole class of concurrency bugs.

### Can I hit 25-30 Kvec/s with my Python code?

Yes, with batched upserts on 768-dim vectors on an NVMe SSD. Run the
benchmarks in `benches/` for your specific hardware. Reported numbers
in `docs/BENCHMARKS.md` use `cargo bench` on a developer-class machine
and include the full production path (WAL + recall >= 95%).

### Can I use multi-process writers?

No. VelesDB uses an OS-level file lock (advisory on Linux, mandatory
on Windows) to ensure a single process owns a data directory. See
[Concurrency & Locking](CONCURRENCY_LOCKING.md) for details.

### What if I have a read-heavy workload with occasional writes?

You are in the sweet spot for Community Edition. Reads scale fully
across threads and do not contend with the writer lock. The writer
lock only blocks mutations, not searches.

### Does the writer lock affect queries?

No. Queries (vector search, text search, hybrid search, graph match,
scroll) take **read** locks and run fully in parallel with other
readers. A single writer does not block readers.

### Why not just use more collections?

You can, and for many workloads that is the right answer. But some
cases — for example, a single knowledge graph with agents updating
shared nodes — are more natural as a single collection. Enterprise
tier is for those cases.

---

## References

- [Concurrency & Locking guide](CONCURRENCY_LOCKING.md) — file/thread rules
- [Tuning Guide](TUNING_GUIDE.md) — HNSW + quantization parameters
- [Configuration](CONFIGURATION.md) — `VelesConfig` reference
- [Core Wiring Debt](../CORE_WIRING_DEBT.md) — internal engineering debt catalogue
- [Architecture](../ARCHITECTURE.md) — overall system design
