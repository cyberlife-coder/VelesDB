# VelesDB Architecture — at a glance

This document is the **15-minute read** an engineer or a technical due-diligence reviewer should start with. It tells you what VelesDB is, how it is shaped, and where to dig deeper.

> **Last updated:** 2026-04-27 — applies to v1.13.x and beyond.

---

## TL;DR — three sentences

VelesDB is a **single-binary, embeddable database** that fuses a **vector index (HNSW)**, a **graph engine** (nodes + edges + traversal), and a **typed columnstore** behind a **shared SQL-like query language called VelesQL**. The target use case is local-first AI agents and RAG pipelines that today have to stitch together pgvector + Neo4j + PostgreSQL by hand. The whole engine fits in a 6 MB binary, runs on Linux/macOS/Windows/iOS/Android/WASM, and persists to a directory on disk that the user controls.

---

## The three engines and why they share an address space

| Engine | What it does | Backed by |
|--------|-------------|-----------|
| **Vector** | k-nearest-neighbor search over high-dimensional embeddings | Native HNSW + SIMD-accelerated distance kernels (AVX-512, AVX2, NEON, WASM SIMD128) |
| **Graph** | Node/edge storage with BFS/DFS traversal, relationship properties | CSR-snapshot zero-copy traversal, FxHashSet visited sets |
| **ColumnStore** | Typed metadata filtering (eq, range, IN, BETWEEN, LIKE) at scale | Per-column secondary indexes, RoaringBitmap for set operations |

The reason these three live together is not because the developer was bored. It is because **a real RAG query is a join of all three**. *"Show me documents semantically close to my question, written by engineers in my team, after 2024-Q1, where the source repository is in our allowlist"* needs the vector engine for similarity, the graph engine for `:AUTHORED_BY` and `:WORKS_IN`, and the columnstore for the date and the allowlist. Doing this across three external systems means three network hops, three caches, three failure modes, and a planner that no one owns.

VelesDB pushes all three down to one query plan, on one cache, on one storage layer.

---

## Layered diagram

```
                ┌─────────────────────────────────────────┐
                │               CLIENTS                    │
                │  Python │ TypeScript │ REST │ CLI │ Mobile│
                └─────┬─────┬───────────┬─────┬─────┬──────┘
                      │     │           │     │     │
                      ▼     ▼           ▼     ▼     ▼
                ┌─────────────────────────────────────────┐
                │              API LAYER                   │
                │ velesdb-python │ velesdb-server │ ...   │
                │   (PyO3)       │    (Axum)      │ wasm  │
                └────────────────┬────────────────────────┘
                                 │
                                 ▼
                ┌─────────────────────────────────────────┐
                │         CORE ENGINE (velesdb-core)       │
                │                                          │
                │   VelesQL parser + planner + cost-based  │
                │   ──────────────────────────────────────  │
                │   Vector | Graph | ColumnStore | Sparse  │
                │   ──────────────────────────────────────  │
                │   HNSW | BM25 | Property indexes          │
                │   ──────────────────────────────────────  │
                │   SIMD distance kernels                   │
                └────────────────┬────────────────────────┘
                                 │
                                 ▼
                ┌─────────────────────────────────────────┐
                │           STORAGE LAYER                  │
                │   mmap | WAL | Snapshots | RoaringBitmap │
                └─────────────────────────────────────────┘
```

For component-level box diagrams, see [`docs/reference/ARCHITECTURE.md`](docs/reference/ARCHITECTURE.md). For the data-flow walkthrough of a search, see the *Anatomy of a query* section below.

---

## The crates

The workspace is laid out as eight crates with one-way dependencies (no cycles).

| Crate | Role | Public surface | Notes |
|-------|------|---------------|-------|
| **`velesdb-core`** | The engine. HNSW, SIMD, VelesQL, collections, storage, recovery, GPU pipeline. Everything else depends on it. | `Database`, `VectorCollection`, `GraphCollection`, `MetadataCollection`, VelesQL parser, error types | The only crate where `unsafe` lives in non-trivial volume (SIMD intrinsics, mmap). All `unsafe` is documented in [`docs/SOUNDNESS.md`](docs/SOUNDNESS.md). |
| **`velesdb-server`** | Axum REST + OpenAPI server. 47 endpoints. | HTTP API | Optional; `feature = "openapi"` exposes the schema. |
| **`velesdb-python`** | PyO3 bindings + NumPy interop. | `velesdb` package on PyPI | Released as wheels via maturin (abi3-py39, manylinux + macOS arm64/x64 + Windows x64). |
| **`velesdb-wasm`** | Browser-side vector search. | `@wiscale/velesdb-sdk` partial | No `persistence` feature; transient in-memory. |
| **`velesdb-mobile`** | iOS / Android bindings via UniFFI. | Swift + Kotlin | One binding crate, two outputs. |
| **`velesdb-cli`** | Interactive REPL for VelesQL. | Single binary | Wraps `velesdb-core`. |
| **`velesdb-migrate`** | Migration tooling: import from Pinecone / Qdrant / Milvus / Weaviate / Chroma / Elasticsearch / Redis. | CLI tool | Strategic candidate to extract to a separate repo (see ROADMAP.md Horizon 3). |
| **`tauri-plugin-velesdb`** | Tauri desktop integration. | Plugin | Used by `demos/tauri-rag-app`. |

The dependency graph is strictly downward: `server`, `python`, `wasm`, `mobile`, `cli`, `migrate`, `tauri-plugin` all depend on `velesdb-core` and never on each other.

---

## Anatomy of a query: walking through `SELECT * FROM docs NEAR $vec WHERE date > '2024-01-01' LIMIT 10`

Here is what happens, end-to-end, in roughly the order it happens. This is the path that the canonical 450 µs p50 latency claim measures.

1. **Parse.** The query enters the VelesQL parser (PEG grammar in `crates/velesdb-core/src/velesql/parser/`). Output: a typed AST (`Query`).
2. **Validate.** Names, types, dimension, distance metric, and feature gates are checked against the live catalog (`Database` registry).
3. **Plan.** The cost-based optimizer (CBO) in `crates/velesdb-core/src/velesql/cost_estimator/` picks an execution strategy. For a vector + filter query, the choice is typically between *pre-filter* (apply the WHERE first then NEAR on the remaining points) and *post-filter* (NEAR first then WHERE on the result). Selectivity statistics drive the choice.
4. **Cache.** A two-tier LRU plan cache (write-generation invalidated) hits or misses. A hit short-circuits steps 1–3 for repeat queries (~1 µs cache hit).
5. **Execute.** The plan calls into the index layer:
   - HNSW search descends the graph, calling SIMD distance kernels (one of AVX-512 / AVX2 / NEON / WASM SIMD128 / scalar) chosen at runtime by `simd_dispatch.rs`.
   - The filter engine applies `date > '2024-01-01'` against the secondary index (RoaringBitmap intersection if multiple predicates).
6. **Hydrate.** Top-k IDs are resolved to full `Point` records (vector + payload).
7. **Serialize and return.** Results travel back up through the API layer.

If the query mutates state (`INSERT`, `UPDATE`, `DELETE`), the WAL (Write-Ahead Log) is updated **before** the in-memory state, with `fsync` controlled by the durability policy.

For the deep walkthrough with code references at every step, see [`docs/reference/ARCHITECTURE.md`](docs/reference/ARCHITECTURE.md). For the data-flow diagram, see [`docs/reference/ARCHITECTURE_DIAGRAMS.md`](docs/reference/ARCHITECTURE_DIAGRAMS.md).

---

## Concurrency, locking, and the happy path of a write

Concurrency is the area where embedded databases earn their reputation. VelesDB uses a few principles:

1. **No `std::sync` lock primitives.** All locks are `parking_lot::RwLock` or `Mutex`. Reasons: no poisoning (so no `.unwrap()` on locks), faster uncontended path, and they release deterministically on `drop`.
2. **Lock ordering is documented and enforced by code review.** See [`docs/CONCURRENCY_MODEL.md`](docs/CONCURRENCY_MODEL.md) (697 lines) for the full lock graph and deadlock-prevention rules. Every code path that takes more than one lock is annotated with a `// LOCK ORDER:` comment naming the order.
3. **Single-writer per collection by default.** This is the v1.x trade-off: simple correctness and no write-write contention. Concurrent WAL writer is a v1.16 (`velesdb-premium`) feature, see ROADMAP.md.
4. **Tests run single-threaded.** `--test-threads=1` is mandatory because tests share temp directories. CI enforces it.

The write path for a single point looks like:

```
    upsert(point)
       │
       ▼
   ┌──────────────────────────────────┐
   │ collection write-lock acquired   │   <- single writer
   ├──────────────────────────────────┤
   │ WAL.append(record)               │
   │ WAL.fsync() if durable           │
   ├──────────────────────────────────┤
   │ in-memory vector layer updated   │
   │ HNSW index updated               │
   │ secondary indexes updated        │
   ├──────────────────────────────────┤
   │ collection write-lock released   │
   └──────────────────────────────────┘
```

Reads take a read-lock and never block each other. Read-mostly workloads scale linearly with cores until the network or the memory bandwidth becomes the limit.

---

## Storage on disk

A VelesDB database is **a directory**. Inside that directory:

- A small `manifest.json` with version + collection list.
- One subdirectory per collection. Each contains:
  - `vectors.mmap` — the raw vector data, memory-mapped
  - `payload.db` — point payloads (JSON)
  - `wal.log` — append-only Write-Ahead Log for durability
  - `index.hnsw` — serialized HNSW graph
  - `bm25/` — full-text inverted index (if enabled)
  - `secondary/` — typed column indexes
  - `snapshot.<gen>` — periodic snapshots for fast cold-start

Recovery on restart: replay the WAL from the latest snapshot.

For the byte-level layout and serialization format, see [`docs/STORAGE_FORMAT.md`](docs/STORAGE_FORMAT.md).

---

## What's NOT in VelesDB (deliberately)

These are intentional non-features. Each one keeps the engine simple at the cost of a use case we have ruled out for now:

- **No Raft / multi-node replication.** Single-node, local-first. Tracked for `velesdb-premium` in ROADMAP.md Horizon 3.
- **No built-in embedding model.** Embedding is a model concern; you bring your own (sentence-transformers, OpenAI, Cohere, BGE, CLIP).
- **No K8s operator / cloud-managed mode.** Conflicts with local-first. Run it on a server, on your laptop, in a container, in WASM, in a mobile app — all the same code.
- **No reranker / LLM glue.** That's user space.
- **No SQL OLTP/OLAP generalist queries.** We do vector + graph + columnstore. A `JOIN` between two arbitrary collections is on the roadmap (#513) but is not the focus.

The full out-of-scope list is in [`ROADMAP.md`](ROADMAP.md).

---

## Soundness, safety, and what we audit

The areas where VelesDB uses `unsafe` are:

| Area | Why `unsafe` | Documented at |
|------|-------------|--------------|
| SIMD intrinsics (AVX2, AVX-512, NEON, WASM SIMD128) | The intrinsics themselves are `unsafe fn` | `crates/velesdb-core/src/simd_native/` (every block annotated `// SAFETY:`) |
| `mmap` of vector data | Reading `&[f32]` from a memory-mapped file | `crates/velesdb-core/src/storage/mmap.rs` |
| FFI for UniFFI mobile bindings | C ABI boundary | `crates/velesdb-mobile/` |
| Compute shader buffer mapping (GPU) | wgpu buffer lifetime | `crates/velesdb-core/src/gpu/` |

Every `unsafe` block has a `// SAFETY: <reason>` comment, enforced by `scripts/verify_unsafe_safety_template.py` in CI. As of v1.13.2: 129 unsafe blocks, 431 SAFETY comments. The full audit narrative is in [`docs/SOUNDNESS.md`](docs/SOUNDNESS.md) (964 lines).

External soundness audit (Cure53 / independent Rust safety expert) is in the v1.15 horizon, conditional on funding. See ROADMAP.md.

---

## Performance — what is the canonical number

> **The canonical, full-path number is 450 µs p50 end-to-end** (10K vectors, 384D, WAL ON, recall ≥ 96%), measured on the i9-14900KF reference machine, reproducible via `python benchmarks/velesdb_benchmark.py --recall`.

Index-only micro-benchmarks (HNSW search isolated, no WAL, hot cache) measure ~55 µs in the same conditions but at 5K/768D. They are useful to understand where the time goes but they are **not the same number**. The README and the crate README disambiguate explicitly since v1.13.3.

The full performance budget gates are in [`QUALITY_BAR.md`](QUALITY_BAR.md). The benchmark methodology is in [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md). A reproducible head-to-head benchmark vs Qdrant + Chroma + pgvector under Docker Compose is on the v1.15 roadmap.

---

## Where to go next, by question

| Your question | Read this |
|--------------|-----------|
| What does the workspace look like crate by crate? | [`docs/contributing/PROJECT_STRUCTURE.md`](docs/contributing/PROJECT_STRUCTURE.md) |
| What is the deep architecture, with all box diagrams? | [`docs/reference/ARCHITECTURE.md`](docs/reference/ARCHITECTURE.md) (518 lines) and [`docs/reference/ARCHITECTURE_DIAGRAMS.md`](docs/reference/ARCHITECTURE_DIAGRAMS.md) |
| How does concurrency / locking work? | [`docs/CONCURRENCY_MODEL.md`](docs/CONCURRENCY_MODEL.md) |
| What is the on-disk format? | [`docs/STORAGE_FORMAT.md`](docs/STORAGE_FORMAT.md) |
| Where is `unsafe` used and why is it sound? | [`docs/SOUNDNESS.md`](docs/SOUNDNESS.md) |
| What's the query language? | [`docs/VELESQL_SPEC.md`](docs/VELESQL_SPEC.md) |
| How do I tune HNSW parameters? | [`docs/guides/TUNING_GUIDE.md`](docs/guides/TUNING_GUIDE.md) |
| What are the current technical limitations? | [`docs/reference/KNOWN_LIMITATIONS.md`](docs/reference/KNOWN_LIMITATIONS.md) |
| What architectural debt is tracked? | [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) (tech-debt registry, despite the name) |
| What's coming next? | [`ROADMAP.md`](ROADMAP.md) |
| What is the quality bar for shipping? | [`QUALITY_BAR.md`](QUALITY_BAR.md) |
| How does a query become results, line by line? | The *Anatomy of a query* section above; deep version in [`docs/reference/ARCHITECTURE.md`](docs/reference/ARCHITECTURE.md) §3 |

---

## A note on naming

`docs/ARCHITECTURE.md` exists in this repo, but despite its name it is **a tracker for architectural tech debt** (e.g. the `Collection` god-object split planned for post-seed), not an architecture overview. The actual architecture documents are this file (high-level, narrative) and [`docs/reference/ARCHITECTURE.md`](docs/reference/ARCHITECTURE.md) (deep, comprehensive). The tech-debt registry is kept under its current name because eight in-code references depend on the path; renaming it is on the cleanup backlog.
