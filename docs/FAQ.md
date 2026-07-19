# VelesDB Frequently Asked Questions

**Last Updated**: 2026-06-12

---

## Table of Contents

- [API Stability and Versioning](#api-stability-and-versioning)
- [Backward Compatibility and Migration](#backward-compatibility-and-migration)
- [Performance Tips](#performance-tips)
- [Known Limitations](#known-limitations)
- [VelesQL vs SQL](#velesql-vs-sql)
- [Context Compiler](#context-compiler)
- [WASM Support](#wasm-support)
- [Python Bindings](#python-bindings)

---

## API Stability and Versioning

### What versioning scheme does VelesDB follow?

VelesDB follows [Semantic Versioning 2.0](https://semver.org/):

- **MAJOR** (X.0.0): Breaking changes to public API or on-disk format.
- **MINOR** (0.X.0): New features, backward-compatible additions.
- **PATCH** (0.0.X): Bug fixes, performance improvements, no API changes.

### What is the deprecation policy?

VelesDB uses a **two minor-version deprecation window**:

1. **Deprecated in version X**: The API is marked `#[deprecated]` with a compiler warning. Documentation is updated to point to the replacement.
2. **Supported in version X+1**: The deprecated API still works but emits warnings.
3. **Removed in version X+2**: The deprecated API is removed entirely.

For example, the legacy `Collection` type was deprecated in v1.4 and is marked `#[deprecated]` since v1.6.0. It still compiles and works but emits warnings. Migrate to the typed APIs (`VectorCollection`, `GraphCollection`, `MetadataCollection`) at your convenience — removal is planned for v2.0.

### Are on-disk formats stable?

On-disk format stability is guaranteed within a major version. If a format change is required in a minor release (as happened in v1.5 with the bincode-to-postcard migration), a migration path is provided. See `docs/guides/MIGRATION_v1.6.md` and `docs/guides/MIGRATION_v1.7.md` for recent migration guides.

---

## Backward Compatibility and Migration

### How do I migrate from the legacy `Collection` to typed APIs?

The legacy `Collection` god-object is being replaced by three focused types. Here is the mapping:

| Legacy API | New Typed API | Purpose |
|---|---|---|
| `Collection` (with vectors) | `VectorCollection` | Dense/sparse vector search, hybrid queries |
| `Collection` (with graph ops) | `GraphCollection` | Nodes, edges, BFS/DFS traversal |
| `Collection` (metadata-only) | `MetadataCollection` | Payload-only storage, no vectors |

#### Rust migration

```rust
// Before (v1.12 and earlier — `Database::get_collection()` was removed in v1.13)
// let coll = db.get_collection("docs").unwrap();
// coll.search(&query, 10)?;

// After (typed — v1.13+)
let coll = db.get_vector_collection("docs").unwrap();
coll.search(&query, 10)?;
```

#### Python migration

```python
# Before (legacy untyped collection)
coll = db.get_collection("docs")
results = coll.search_request(velesdb.SearchOptions(vector=query_vec, top_k=10))

# After (preferred for graph collections)
graph = db.create_graph_collection("knowledge", dimension=768)
graph.add_edge({"id": 1, "source": 10, "target": 20, "label": "KNOWS"})
results = graph.traverse_bfs(source_id=10, max_depth=3)
```

The legacy `Collection` type continues to work for vector search in the Python SDK. Use `create_graph_collection()` / `get_graph_collection()` for graph-specific operations, and `create_metadata_collection()` for metadata-only collections.

### Can I use both old and new APIs simultaneously?

Yes. The legacy `Collection` and the new typed collections coexist in the same database. They share the same on-disk storage format. You can gradually migrate collection by collection.

---

## Performance Tips

### How do I get maximum SIMD performance on local dev?

Uncomment the `target-cpu=native` line in `.cargo/config.toml`:

```toml
[build]
# Uncomment for local development only (do NOT commit uncommented):
# rustflags = ["-C", "target-cpu=native"]
```

This enables AVX-512 or AVX2 intrinsics specific to your CPU. **Do not commit this change** -- it breaks CI runners that may have different CPU capabilities.

At runtime, VelesDB automatically detects and uses the best available SIMD path (AVX-512 > AVX2 > NEON > scalar) via `simd_dispatch.rs`.

### What quantization options are available?

| Mode | Memory Reduction | Recall Impact | Best For |
|---|---|---|---|
| `full` (f32) | 1x (baseline) | Perfect | Small datasets, highest accuracy |
| `sq8` (8-bit scalar) | 4x | Minimal (~1-2%) | Production workloads, good balance |
| `binary` (1-bit) | 32x | Moderate (~5-10%) | Very large datasets, rough filtering |
| `pq` (product quantization) | Configurable | Tunable | Large-scale ANN with ADC search |

```python
# SQ8 quantization (4x memory savings)
coll = db.create_collection("docs", dimension=768, storage_mode="sq8")

# Product Quantization (train after inserting data)
db.train_pq("docs", m=8, k=256)
db.train_pq("docs", m=16, k=128, opq=True)  # OPQ variant
```

### How can I tune HNSW parameters?

```python
from velesdb import HnswOptions

# Higher m = more connections = better recall, more memory
# Higher ef_construction = better index quality, slower build
# v1.13: typed HnswOptions replaces the old flat kwargs (`m=`, `ef_construction=`).
coll = db.create_collection(
    "docs",
    dimension=768,
    hnsw=HnswOptions(m=48, ef_construction=600),
)

# At query time, increase ef_search for better recall
results = coll.search_with_ef(vector=query, top_k=10, ef_search=256)
```

These defaults apply when you omit `hnsw=HnswOptions(...)` on `create_collection`: `m=24, ef_construction=300` for dim <= 256, and `m=32, ef_construction=400` for dim >= 257. They work well for most workloads up to 100K vectors. Use `HnswParams::for_dataset_size()` for larger datasets.

### How do I use batch and streaming ingestion?

```python
# Batch upsert (synchronous, immediate consistency)
coll.upsert([
    {"id": 1, "vector": vec1, "payload": {"title": "Doc 1"}},
    {"id": 2, "vector": vec2, "payload": {"title": "Doc 2"}},
])

# Streaming insert (async buffer, eventual consistency, higher throughput)
coll.stream_insert([
    {"id": 3, "vector": vec3, "payload": {"title": "Doc 3"}},
])
```

---

## Known Limitations

### Architecture limitations

- **Single-node only**: VelesDB runs on a single machine. There is no distributed mode, no sharding across nodes, and no replication.
- **No high availability (HA)**: A single VelesDB instance is a single point of failure. Use application-level redundancy if needed.
- **No ACID transactions**: Operations are durable (WAL-backed) but there is no multi-operation transaction support. Each upsert/delete is atomic individually.
- **No distributed transactions**: Cross-collection operations are not transactional.

### Data size limits

- Vector dimension: up to 65,536 (`MAX_DIMENSION` in `validation.rs`; practical limit ~4096 for performance).
- Collection count: no hard limit, but each collection consumes file descriptors for mmap.
- Single-node memory: vector data is memory-mapped, so the practical limit is available RAM + swap.

### Query limitations

- VelesQL parses subqueries but does not execute them yet. CTEs are not supported.
- `INSERT` and `UPDATE` are parsed by VelesQL but runtime execution is not yet implemented (use the programmatic API). `DELETE` is planned.
- Graph traversal in VelesQL is limited to `MATCH` patterns; recursive CTEs are not available.

---

## Context Compiler

### How do I reduce my agent's token costs?

Compile the context before sending it: the `compile_context` MCP tool (also
`compileContext` in Node, `ContextCompiler` in Rust) deduplicates fragments,
collapses repeated log lines, and packs what matters under your token budget
— locally, deterministically, with an auditable decision per fragment. On the
committed benchmark corpus this measures 75–82 % estimated savings in ~2 ms
(`cargo run -p velesdb-memory --example context_savings --no-default-features
--features context`).

### Is the compression an LLM summary?

No. The compiler is **strictly deterministic** — no model, no network, no
clock: the same request always compiles to the same bytes. "Abstraction" is a
structured, reversible reduction (e.g. `ERROR timeout (x50)` for a repeated
log line), never a generative summary. Anything critical — code fences, URLs,
numbers/dates/ids, negative constraints, fragments marked
`{"verbatim": true}` — survives byte for byte.

### Can I get back what was compressed away?

Yes. Nothing is silently lost: over-budget content becomes a
`ctx://source/<hash>` handle listed in `retrieval_handles`, and
`retrieve_context_source` returns the exact original bytes. If critical
content could not fit, the compilation's `risk` comes back `"high"` so you
can raise the budget or send uncompressed. `explain_compilation` answers
"why was this fragment dropped/shortened?" with the deciding rule and reason.

### Are the reported savings my billed savings?

No — and the docs never claim so. `insights.tokens_saved` is a **local
estimate** from a char-class estimator calibrated against a real BPE
(cl100k): it deliberately over-counts every measured content class
(+13 % on JSON up to +55 % on English prose), so the budget guarantee errs
on the safe side. Billed savings depend on your provider's tokenizer and
pricing — pass your `PricingTable` in `policy.pricing` (works over MCP,
Node, and Rust; set `target_model` to pick the row) and inject your own
`TokenEstimator` (Rust) for exact figures; *validated* savings — proving
answer quality did not degrade — require a task-level evaluation harness.

## VelesQL vs SQL

### What SQL features does VelesQL support?

VelesQL is a SQL-like query language with vector and graph extensions. It supports a subset of SQL plus vector-specific operations.

| Feature | VelesQL | Standard SQL |
|---|---|---|
| `SELECT ... FROM ... WHERE` | Yes | Yes |
| `ORDER BY` (columns, expressions) | Yes | Yes |
| `LIMIT` / `OFFSET` | Yes | Yes |
| `GROUP BY` / `HAVING` | Yes | Yes |
| `DISTINCT` / `DISTINCT ON` | Yes | Yes |
| `JOIN` (INNER, LEFT, CROSS) | Yes | Yes |
| `UNION` / `INTERSECT` / `EXCEPT` | Yes | Yes |
| Aggregations (`COUNT`, `SUM`, `AVG`, `MIN`, `MAX`) | Yes | Yes |
| `vector NEAR $v` (similarity search) | Yes | No |
| `MATCH` (graph traversal) | Yes | No |
| `USING FUSION` (hybrid search) | Yes | No |
| `NEAR_FUSED` (multi-vector fusion) | Yes | No |
| `SPARSE_NEAR` (sparse vector search) | Yes | No |
| `TRAIN QUANTIZER ON ...` | Yes | No |
| Subqueries / CTEs | No | Yes |
| `INSERT` / `UPDATE` | Parsed (no runtime execution) | Yes |
| `DELETE` | Planned | Yes |
| `CREATE TABLE` / DDL | No | Yes |
| Window functions (`ROW_NUMBER`, `RANK`, `DENSE_RANK` with `OVER`, `PARTITION BY`, `ORDER BY`) | Yes (v1.13.0) | Yes |
| Stored procedures | No | Yes |

### Where is the full VelesQL specification?

See `docs/VELESQL_SPEC.md` for the complete grammar and examples.

---

## WASM Support

### Can VelesDB run in the browser?

Yes. The `velesdb-wasm` crate provides browser-side vector search. You must disable the `persistence` feature (which depends on mmap, rayon, and tokio, none of which work in WASM):

```bash
cargo build -p velesdb-wasm --no-default-features --target wasm32-unknown-unknown
```

### What features are available in WASM?

- In-memory vector collections with exact (brute-force) search — there is no
  HNSW graph in WASM, so search is O(n) but recall is perfect.
- **Local VelesQL execution** via `db.executeQuery(sql, params)`: SELECT
  (WHERE, GROUP BY/HAVING, ORDER BY, JOIN, UNION/INTERSECT/EXCEPT, fusion),
  DML (INSERT/UPSERT/UPDATE/DELETE), DDL (CREATE/DROP/TRUNCATE COLLECTION),
  introspection (SHOW COLLECTIONS, DESCRIBE, EXPLAIN), and graph statements
  (INSERT NODE/EDGE, SELECT EDGES). No REST server required.
- **Graph queries** via the in-memory `GraphStore` (nodes, edges, traversal):
  graph statements (`INSERT NODE/EDGE`, `SELECT EDGES`) and `MATCH` patterns up
  to **2 hops** run against a per-collection in-memory graph created lazily.
  (`CREATE GRAPH COLLECTION` DDL is rejected — use `GraphStore` directly.)
- All distance metrics (cosine, euclidean, dot product, hamming, jaccard).
- f16/bf16 half-precision vector storage (50% memory reduction).
- Explicit snapshot persistence to IndexedDB (`save()` / `load()` /
  `export_to_bytes()` / `import_from_bytes()`).

### What features are NOT available in WASM?

- Durable disk persistence (mmap, WAL): the `persistence` feature does not
  compile on `wasm32`. IndexedDB `save()`/`load()` is explicit snapshotting,
  not write-ahead durability.
- HNSW indexing — `CREATE INDEX` is accepted as a no-op for API parity;
  search stays brute-force.
- `MATCH` patterns beyond 2 hops (rejected with a descriptive error).
- `TRAIN QUANTIZER` / PQ training (requires the `persistence`/`rayon` stack).
- Multi-threaded indexing (rayon) and streaming ingestion (tokio).

---

## Python Bindings

### How do I install the Python SDK?

The Python SDK is built with PyO3 and maturin:

```bash
# Development build (editable install)
cd crates/velesdb-python
pip install maturin
maturin develop

# Release wheel
maturin build --release
pip install target/wheels/velesdb-*.whl
```

### What Python version is required?

Python 3.9 or later. NumPy is supported for vector input but not required.

### What classes are available?

| Class | Purpose |
|---|---|
| `Database` | Open/create database, manage collections |
| `Collection` | Vector search, upsert, delete, VelesQL queries |
| `GraphCollection` | Persistent graph with edges, traversal, node embeddings |
| `GraphSchema` | Schema configuration for graph collections |
| `FusionStrategy` | Multi-query fusion (RRF, Average, Maximum, Weighted, RSF) |
| `VelesQL` | Query parser and validator |
| `ParsedStatement` | Introspect parsed VelesQL queries |
| `GraphStore` | In-memory graph operations |
| `AgentMemory` | AI agent memory (semantic, episodic, procedural) |

### Can I use NumPy arrays as vectors?

Yes. All methods that accept vectors (`search`, `upsert`, etc.) accept both Python lists and NumPy arrays:

```python
import numpy as np

vec = np.random.randn(384).astype(np.float32)
results = coll.search_request(velesdb.SearchOptions(vector=vec, top_k=10))
```

### Where are the Python examples?

See `examples/python/` for runnable examples covering:

- Basic CRUD and search (`fusion_strategies.py`)
- Graph traversal (`graph_traversal.py`)
- Hybrid queries (`hybrid_queries.py`)
- Multi-model notebook (`multimodel_notebook.py`)
- GraphRAG with LangChain (`graphrag_langchain.py`)
- GraphRAG with LlamaIndex (`graphrag_llamaindex.py`)
