# Core: streaming inserts, backpressure and the delta buffer

For continuously arriving data — IoT sensors, live embeddings, log streams —
`velesdb-core` ships a bounded-channel ingestion pipeline with automatic
micro-batch flushing and explicit backpressure signalling.

Moved out of `crates/velesdb-core/README.md` to keep that file under the
400-line documentation budget.

> The snippets below are function bodies that continue from a `collection`
> handle obtained with
> `db.get_vector_collection(name).expect("collection exists")`.

---

## Basic usage

```rust,no_run
use velesdb_core::collection::streaming::StreamingConfig;
use velesdb_core::Point;

// Configure the pipeline
let config = StreamingConfig {
    buffer_size: 10_000,     // channel capacity (backpressure threshold)
    batch_size: 128,         // flush every 128 points
    flush_interval_ms: 50,   // ...or every 50 ms, whichever comes first
};

// `collection` is a `VectorCollection` obtained from
// `db.get_vector_collection(name).expect("collection exists")` — the handle is
// cheap to clone (Arc-backed inside). Activate the streaming pipeline:
collection.enable_streaming(config);

// Send points — returns immediately.
let point = Point::new(1, vec![0.1; 384], None);
match collection.stream_insert(point) {
    Ok(()) => { /* accepted */ }
    Err(e) => eprintln!("Backpressure: {e}"),
}
```

## Backpressure

The send path is non-blocking (`try_send`). When the bounded channel is at
capacity, `stream_insert` returns `BackpressureError::BufferFull` — the caller
must retry after a short delay or drop the point. If the background drain task
exits unexpectedly, `BackpressureError::DrainTaskDead` is returned;
`NotConfigured` means `enable_streaming` was never called.

Dropping points silently is never done for you: backpressure is always
surfaced as an error value.

## Delta buffer (insert-and-search)

During an HNSW rebuild, freshly inserted vectors are not yet reachable through
the graph. The delta buffer accumulates them and merges them into search
results with a brute-force scan, so new data is searchable immediately instead
of waiting for the rebuild to finish.

```rust,ignore
// The delta buffer is managed automatically by the streaming pipeline.
// When it is active, search results transparently include buffered vectors.
let results = collection.search(&query, 10)?;
// ^ covers both HNSW-indexed and delta-buffered vectors
```

The cost is a linear scan over the buffered slice, so the buffer is bounded and
drained by the rebuild; it is a freshness mechanism, not a second index.

## Types

| Type | Path | Description |
|------|------|-------------|
| `StreamIngester` | `velesdb_core::collection::streaming` | Bounded-channel ingestion pipeline |
| `StreamingConfig` | `velesdb_core::collection::streaming` | `buffer_size`, `batch_size`, `flush_interval_ms` |
| `BackpressureError` | `velesdb_core::collection::streaming` | `BufferFull`, `NotConfigured`, `DrainTaskDead` |

Full signatures live on [docs.rs](https://docs.rs/velesdb-core).

## See also

- [velesdb-core README](../../crates/velesdb-core/README.md)
- [Write concurrency](./WRITE_CONCURRENCY.md) — the single-writer-per-collection model
- [Concurrency and locking](./CONCURRENCY_LOCKING.md)
- [Core collections and metrics](./CORE_COLLECTIONS_AND_METRICS.md) — bulk ingestion and durability

---

Last updated: 2026-07-25 · Applies to: velesdb-core 6.0.0
