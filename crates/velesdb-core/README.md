# velesdb-core

> The embedded tri-engine of VelesDB: vector, graph and columnar metadata in one Rust database.

[![crates.io](https://img.shields.io/crates/v/velesdb-core.svg)](https://crates.io/crates/velesdb-core)
[![docs.rs](https://docs.rs/velesdb-core/badge.svg)](https://docs.rs/velesdb-core)
[![License](https://img.shields.io/badge/license-VelesDB_Core_1.0-blue.svg)](./LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/cyberlife-coder/VelesDB/ci.yml?branch=main)](https://github.com/cyberlife-coder/VelesDB/actions)

## Objective

Semantic retrieval usually means running a vector store, a graph database and a
relational store side by side, then stitching their results together in
application code — three deployments, three consistency stories, three query
languages.

`velesdb-core` is the embedded engine that collapses those three into one
process and one language. Vectors (HNSW + SIMD), typed graph edges and typed
columnar metadata live in the same collection and are queried together with
**VelesQL**. No server, no network hop, no external dependency: it is a Rust
library that reads and writes a directory on your disk.

If you do not need retrieval over your own data, you do not need this crate.

## Use cases

- A desktop or CLI application that must search its own documents offline, with
  no service to install and no data leaving the machine.
- A RAG pipeline that filters candidates on structured metadata (`tenant`,
  `date`, `status`) in the *same* query as the vector search, instead of
  post-filtering results and losing recall.
- A recommendation feature where "similar to this item" must be combined with
  "and connected to the user by at most 2 hops" — vector plus graph traversal
  in one statement.
- An AI agent that needs durable memory (facts, events, learned procedures)
  with TTL and snapshots, embedded in the agent process itself.
- An embedded/edge deployment where a 32x-compressed index must fit in RAM on
  constrained hardware.

## Prerequisites

| Requirement | Minimum version | Note |
|---|---|---|
| Rust | 1.90 | Workspace MSRV, pinned in `rust-toolchain.toml` |
| Cargo | shipped with Rust | No other build tool required |
| Disk | writable directory | The `persistence` feature (on by default) memory-maps files there |
| Embeddings | any source | This crate does **not** compute embeddings — you supply the vectors |
| GPU | optional | Only for the `gpu` feature; falls back to SIMD when absent |

## Installation

```bash
cargo add velesdb-core
```

For WASM or any target without a filesystem, disable the default feature:

```bash
cargo add velesdb-core --no-default-features
```

## First success in 60 seconds

Create a project, add the two dependencies, paste this into `src/main.rs`, run
it.

```bash
cargo new veles-hello && cd veles-hello
cargo add velesdb-core serde_json
```

```rust,no_run
use serde_json::json;
use velesdb_core::{Database, DistanceMetric, Point};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Open (or create) a local database directory.
    let db = Database::open("./veles-quickstart")?;

    // 2. One collection = one vector dimension + one distance metric (both immutable).
    db.create_collection("documents", 4, DistanceMetric::Cosine)?;
    let documents = db
        .get_vector_collection("documents")
        .ok_or("collection not found")?;

    // 3. Insert points: id, vector, optional JSON payload.
    documents.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"title": "rust"}))),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"title": "python"}))),
        Point::new(3, vec![0.9, 0.1, 0.0, 0.0], Some(json!({"title": "cargo"}))),
    ])?;

    // 4. flush() is the explicit durability barrier.
    documents.flush()?;

    // 5. Search: top-2 nearest neighbours of [1, 0, 0, 0].
    for hit in documents.search(&[1.0, 0.0, 0.0, 0.0], 2)? {
        let title = hit
            .point
            .payload
            .as_ref()
            .and_then(|payload| payload.get("title"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("<none>");
        println!("id={} title={title} score={:.4}", hit.point.id, hit.score);
    }

    Ok(())
}
```

```console
$ cargo run
id=1 title=rust score=1.0000
id=3 title=cargo score=0.9939
```

Success looks exactly like that: two lines, `id=1` first with a cosine score of
`1.0000` (the query vector is identical to point 1), `id=3` second. Point 2 is
orthogonal to the query and is correctly excluded from the top 2.

Anything else is a failure — in particular, a second `cargo run` prints
`Error: CollectionExists("documents")` because the collection is already
persisted in `./veles-quickstart`. Delete that directory to start over, or skip
`create_collection` when the collection already exists.

## Configuration

Compile-time features (`Cargo.toml`):

| Feature | Default | Effect |
|---|---|---|
| `persistence` | **on** | mmap storage, WAL, rayon parallelism, tokio. Turn off for WASM. |
| `gpu` | off | wgpu compute pipeline for batch distance kernels; falls back to SIMD |
| `openapi` | off | `utoipa::ToSchema` derives on the `api_types` DTOs |
| `update-check` | off | HTTP client for automatic version checking |
| `internal-bench` | off | Exposes internal hooks used by some benches |
| `bench-sift1m` | off | SIFT1M benchmark. Links `ureq`/TLS as a **regular** dependency — never enable in a shipping build |
| `loom` | off | Loom concurrency testing (nightly only) |
| `test-fault-injection` | off | RAII guards forcing internal failures in tests. Never enable in production |

Runtime settings (HNSW parameters, limits, storage, logging) are read from
`velesdb.toml` — see the [configuration guide](../../docs/guides/CONFIGURATION.md).

## Examples

- [`examples/`](./examples) — `crash_driver` (crash-recovery test driver),
  `profile_batch_insert` (flamegraph target for HNSW batch insert),
  `simd_precision_check` (SIMD vs scalar validation). These are engine tooling,
  not tutorials.
- [`examples/rust/`](../../examples/rust) — `multimodel_search.rs`, a runnable
  vector + graph + metadata query.
- [`examples/mini_recommender/`](../../examples/mini_recommender) and
  [`examples/ecommerce_recommendation/`](../../examples/ecommerce_recommendation)
  — complete standalone applications.

## API / commands

Generated reference: [docs.rs/velesdb-core](https://docs.rs/velesdb-core).
Import map (where each type lives): [Core API map](../../docs/guides/CORE_API_MAP.md).

Task guides, all moved out of this README so it stays readable:

| Guide | What it covers |
|---|---|
| [Collections, metrics, storage](../../docs/guides/CORE_COLLECTIONS_AND_METRICS.md) | Collection model, the 5 distance metrics, embedding dimensions, payload format, quantized storage modes, bulk ingestion, durability |
| [`VelesQL` reference](../../docs/guides/CORE_VELESQL_REFERENCE.md) | Vector/text/hybrid queries, metadata filters, `WITH` options, operator table, `JOIN` limit, `EXPLAIN` |
| [Sparse vectors and fusion](../../docs/guides/CORE_SPARSE_AND_FUSION.md) | Named sparse indexes, DAAT `MaxScore`, RRF and Relative Score fusion |
| [Streaming inserts](../../docs/guides/CORE_STREAMING_INSERTS.md) | `StreamIngester`, backpressure, delta buffer (insert-and-search) |
| [Query plan cache](../../docs/guides/CORE_QUERY_PLAN_CACHE.md) | Two-tier LRU cache, write-generation invalidation, `EXPLAIN` cache fields, metrics |
| [Agent Memory SDK (Rust)](../../docs/guides/CORE_AGENT_MEMORY_RUST.md) | Semantic, episodic and procedural memory, TTL, eviction, snapshots |
| [Core performance](../../docs/guides/CORE_PERFORMANCE.md) | Every published number, its measurement context, and how to reproduce it |
| [Graph patterns](../../docs/guides/GRAPH_PATTERNS.md) · [Multi-model queries](../../docs/guides/MULTIMODEL_QUERIES.md) | Graph modelling and cross-engine statements |
| [Search modes](../../docs/guides/SEARCH_MODES.md) · [Tuning guide](../../docs/guides/TUNING_GUIDE.md) · [Quantization](../../docs/guides/QUANTIZATION.md) | Recall/latency trade-offs |
| [Write concurrency](../../docs/guides/WRITE_CONCURRENCY.md) · [Concurrency and locking](../../docs/guides/CONCURRENCY_LOCKING.md) | The write model and file locking |

## Performance

Two headline numbers, both measured rather than estimated. Every figure, its
hardware and its reproduction command live in
[Core performance](../../docs/guides/CORE_PERFORMANCE.md).

| Claim | Measured | Context |
|-------|----------|---------|
| Native HNSW search with AVX-512/AVX2/NEON SIMD | **450µs p50** end-to-end | 10K points, 384D, WAL on, recall ≥ 96% |
| `ColumnStore` filtering vs. scanning JSON payloads | up to **130x** faster | integer equality, 100K rows |

Reproduce with `cargo bench -p velesdb-core --bench hnsw_benchmark` and
`cargo bench -p velesdb-core --bench column_filter_benchmark`.

Numbers move with hardware and dataset. Treat them as the shape of the
engine's cost, not a guarantee for your workload — measure on yours.

## Known limits

- **No embedding generation.** You bring the vectors; the crate never calls a
  model or the network to produce them.
- **No clustering, sharding or replication.** `velesdb-core` is a single-process
  embedded engine. One process at a time may open a database directory: a second
  one fails with `DatabaseLocked`.
- **One writer per collection.** Concurrent readers are fine; concurrent writers
  to the same collection serialize — see
  [Write concurrency](../../docs/guides/WRITE_CONCURRENCY.md).
- **Metric and dimension are immutable.** Changing either means creating a new
  collection and reindexing.
- **`JOIN ... USING (...)` supports one column only.** Multi-column `USING`
  parses but does not execute; use `JOIN ... ON left = right` instead.
- **No agent-memory service layer here.** The explainable `MemoryService`,
  `why()` and the deterministic context compiler (`compile_context`) live one
  level up in [`velesdb-memory`](../velesdb-memory/README.md), which depends on
  this crate — never the reverse.
- **WASM builds are read/compute only.** `--no-default-features` removes mmap
  storage, WAL, rayon and tokio along with the `persistence` feature.

## Compatibility

`velesdb-core` is a library, not an agent or MCP surface, so this table lists
the platforms and toolchains the project builds and tests on.

| Environment | Status | Note |
|---|---|---|
| Rust 1.90 (pinned) | Supported | `rust-toolchain.toml`; CI uses the same version |
| Linux `x86_64` | Supported | CI: `cargo check --workspace --all-targets --all-features` |
| Linux aarch64 | Supported | CI: dedicated ARM64 benchmark runner (`ubuntu-24.04-arm`) |
| Windows `x86_64` (MSVC) | Supported | CI: `--all-features` check on `windows-latest` |
| macOS aarch64 / `x86_64` | Supported | Release pipeline builds both Darwin targets |
| `wasm32-unknown-unknown` | Supported, restricted | CI checks `--no-default-features` only; no filesystem persistence |
| Rust nightly | Build-checked | Only for the `loom` concurrency feature |

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `Error: CollectionExists("documents")` | `create_collection` re-run against an existing on-disk collection | Delete the database directory, or call `get_vector_collection` first and only create when it returns `None` |
| `[VELES-004] Vector dimension mismatch: expected 4, got 3` | The vector (on insert or query) does not match the dimension fixed at creation | Use your embedding model's exact output size; the dimension cannot be changed after creation |
| `[VELES-031] Database is already opened by another process: <path>` | A second process tried to open the same directory | Close the first process, or point the second one at another directory — one writer process per database |
| `get_vector_collection` returns `None` for a name you created | The name belongs to a graph or metadata-only collection | Use `get_graph_collection` / `get_metadata_collection`, or `get_any_collection` for the type-erased handle |
| Data missing after a crash or `kill -9` | `upsert` updates in-memory/WAL state; destructors are best-effort | Call `flush()` as your explicit commit boundary |

## License

`VelesDB` Core License 1.0 — see [LICENSE](./LICENSE).

---

Last updated: 2026-07-25 · Applies to: velesdb-core 4.3.0 · [Report a docs error](https://github.com/cyberlife-coder/velesdb/issues)
