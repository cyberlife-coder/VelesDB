# VelesDB Frequently Asked Questions

**Last Updated**: 2026-03-15

---

## Table of Contents

- [API Stability and Versioning](#api-stability-and-versioning)
- [Backward Compatibility and Migration](#backward-compatibility-and-migration)
- [Performance Tips](#performance-tips)
- [Known Limitations](#known-limitations)
- [VelesQL vs SQL](#velesql-vs-sql)
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

For example, the legacy `Collection` type was deprecated in v1.4 and will be removed in v1.6. Code using it should migrate to the typed APIs (`VectorCollection`, `GraphCollection`, `MetadataCollection`).

### Are on-disk formats stable?

On-disk format stability is guaranteed within a major version. If a format change is required in a minor release (as happened in v1.5 with the bincode-to-postcard migration), a migration path is provided. See `docs/MIGRATION_v1.4_to_v1.5.md` for details.

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
// Before (legacy)
#[allow(deprecated)]
let coll = db.get_collection("docs").unwrap();
coll.search(&query, 10)?;

// After (typed)
let coll = db.get_vector_collection("docs").unwrap();
coll.search(&query, 10)?;
```

#### Python migration

```python
# Before (still works, emits deprecation warning)
coll = db.get_collection("docs")
results = coll.search(vector=query_vec, top_k=10)

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
# Higher m = more connections = better recall, more memory
# Higher ef_construction = better index quality, slower build
coll = db.create_collection("docs", dimension=768, m=48, ef_construction=600)

# At query time, increase ef_search for better recall
results = coll.search_with_ef(vector=query, top_k=10, ef_search=256)
```

Default values (m=16, ef_construction=200) work well for most workloads. Increase `m` and `ef_construction` for datasets over 1M vectors or when recall above 0.99 is required.

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

- Vector dimension: up to 65,535 (practical limit ~4096 for performance).
- Collection count: no hard limit, but each collection consumes file descriptors for mmap.
- Single-node memory: vector data is memory-mapped, so the practical limit is available RAM + swap.

### Query limitations

- VelesQL does not support subqueries or CTEs.
- `UPDATE` and `DELETE ... WHERE` statements are not supported in VelesQL (use the programmatic API).
- Graph traversal in VelesQL is limited to `MATCH` patterns; recursive CTEs are not available.

---

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
| `TRAIN QUANTIZER ON ...` | Yes | No |
| Subqueries / CTEs | No | Yes |
| `INSERT` / `UPDATE` / `DELETE` | No | Yes |
| `CREATE TABLE` / DDL | No | Yes |
| Window functions | No | Yes |
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

- In-memory vector collections (HNSW search, quantization).
- VelesQL parsing and execution.
- All distance metrics (cosine, euclidean, dot product).

### What features are NOT available in WASM?

- Disk persistence (mmap, WAL).
- Multi-threaded indexing (rayon).
- Graph collections (require persistence).
- Streaming ingestion (requires tokio).

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

Python 3.8 or later. NumPy is supported for vector input but not required.

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
results = coll.search(vector=vec, top_k=10)
```

### Where are the Python examples?

See `examples/python/` for runnable examples covering:

- Basic CRUD and search (`fusion_strategies.py`)
- Graph traversal (`graph_traversal.py`)
- Hybrid queries (`hybrid_queries.py`)
- Multi-model notebook (`multimodel_notebook.py`)
- GraphRAG with LangChain (`graphrag_langchain.py`)
- GraphRAG with LlamaIndex (`graphrag_llamaindex.py`)
