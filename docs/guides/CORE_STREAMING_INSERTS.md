# Core: streaming inserts and backpressure

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
// cheap to clone (Arc-backed inside). Activate the streaming pipeline; a
// `buffer_size` of 0 or a `flush_interval_ms` of 0 is refused (VELES-009):
collection.enable_streaming(config)?;

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

## When a streamed point is searchable

A point is searchable once its micro-batch is flushed — after `batch_size`
points or `flush_interval_ms`, whichever comes first: the drain upserts the
batch through the collection's ordinary write path, and search finds each
point from then on. A point the HNSW graph has not indexed yet (the deferred
indexer or the async index builder still holds it) is scored exactly on
every search until a merge indexes it.

```rust,ignore
collection.stream_insert(point)?;
// ... at most `flush_interval_ms` later:
let results = collection.search(&query, 10)?;
```

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

Last updated: 2026-07-25 · Applies to: velesdb-core 8.0.0
