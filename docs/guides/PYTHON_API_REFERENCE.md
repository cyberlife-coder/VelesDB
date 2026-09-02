# Python API reference — Database, Collection, search and storage

Moved out of [`crates/velesdb-python/README.md`](../../crates/velesdb-python/README.md)
to keep that file under the documentation line budget. Authoritative signatures
and docstrings live in the shipped type stub
[`crates/velesdb-python/python/velesdb/__init__.pyi`](../../crates/velesdb-python/python/velesdb/__init__.pyi),
which your IDE reads directly (`py.typed` is included in the wheel).

## Feature map

| Capability | Where |
|---|---|
| Dense + sparse vector search, hybrid fusion | this page |
| Multi-query fusion (RRF, Weighted, Relative Score) | this page |
| Storage modes / quantization, bulk loading, distance metrics | this page |
| Metadata-only collections, secondary indexes, `explain()` | this page |
| Agent Memory SDK and the `why()` wedge | [PYTHON_AGENT_MEMORY.md](PYTHON_AGENT_MEMORY.md) |
| Context compiler | [PYTHON_CONTEXT_COMPILER.md](PYTHON_CONTEXT_COMPILER.md) |
| Graph collections and in-memory `GraphStore` | [PYTHON_GRAPH.md](PYTHON_GRAPH.md) |
| VelesQL parser API | [PYTHON_VELESQL.md](PYTHON_VELESQL.md) |
| Text → vectors → results (RAG) | [PYTHON_RAG_PIPELINE.md](PYTHON_RAG_PIPELINE.md) |
| Talking to a remote `velesdb-server` | [PYTHON_REMOTE_SERVER.md](PYTHON_REMOTE_SERVER.md) |
| Throughput tuning | [PYTHON_PERFORMANCE.md](PYTHON_PERFORMANCE.md) |

## Search API note

`search_request(SearchOptions(...))` is the canonical search entry point and is
used throughout the VelesDB Python documentation. The older multi-keyword
`collection.search(vector=..., top_k=..., filter=...)` still works but is
**deprecated since v1.15** — it emits a `DeprecationWarning`.
`SearchOptions` accepts canonical `k`, plus `vector`, `sparse_vector`, `filter`,
`sparse_index_name` and `include_vectors`, plus a fluent builder
(`SearchOptions().with_vector(v).with_k(10)`). The deprecated `top_k` spelling
remains accepted as an alias for one compatibility version. The `vector` argument
accepts numpy arrays (`dtype=np.float32`) as well as Python lists.

## Database

```python
# Create/open database
db = velesdb.Database("./path/to/data")

# List collections
names = db.list_collections()
names = db.get_collections()  # compatibility alias

# Create collection (with optional HNSW tuning via typed options)
collection = db.create_collection("name", dimension=768, metric="cosine")
from velesdb import HnswOptions
collection = db.create_collection(
    "tuned",
    dimension=768,
    hnsw=HnswOptions(m=48, ef_construction=600),
)
# Auto-tuned HNSW for an expected dataset size
collection = db.create_collection(
    "big",
    dimension=128,
    hnsw=HnswOptions.for_dataset_size(128, 1_000_000),
)

# Get existing collection
collection = db.get_collection("name")

# Delete collection
db.delete_collection("name")

# Create a metadata-only collection (no vectors, payload-only CRUD)
products = db.create_metadata_collection("products")

# Create a graph collection (see PYTHON_GRAPH.md)
graph = db.create_graph_collection("knowledge", dimension=768)

# Agent memory for AI workflows (see PYTHON_AGENT_MEMORY.md)
memory = db.agent_memory(dimension=384)

# Train Product Quantization for compressed search
db.train_pq("name", m=8, k=256)
db.train_pq("name", m=16, k=128, opq=True)  # Optimized PQ

# Analyze collection statistics
stats = db.analyze_collection("name")
print(stats["total_points"], stats["total_size_bytes"])

# Query plan cache management
cache_stats = db.plan_cache_stats()
print(f"Hit rate: {cache_stats['hit_rate']:.2%}")
db.clear_plan_cache()
```

`create_collection` is the recommended Python entry point. It accepts typed
dataclasses (`hnsw=HnswOptions(...)`, `auto_reindex=AutoReindexOptions(...)`,
`limits=LimitsOptions(...)`) for tuning; `dimension=None` auto-detects the
dimension on the first `upsert()`. `db.get_or_create_collection(...)` is the
idempotent variant. The Rust core also exposes `create_vector_collection` for
callers who want the explicit typed API.

## Collection

```python
# Get collection info
info = collection.info()
# {"name": "documents", "dimension": 768, "metric": "cosine", "storage_mode": "full", "point_count": 100, "metadata_only": False}

# Insert/update vectors (with immediate flush)
collection.upsert([
    {"id": 1, "vector": [0.1, 0.2, 0.3, 0.4], "payload": {"key": "value"}}
])

# Bulk insert (optimized for high-throughput - 3-7x faster)
# Uses parallel HNSW insertion + single flush at the end
collection.upsert_bulk([
    {"id": i, "vector": vectors[i].tolist()} for i in range(10000)
])

# Vector search
results = collection.search_request(velesdb.SearchOptions(vector=query_vector, k=10))

# Search with custom HNSW ef_search (trade speed for recall)
results = collection.search_with_ef(vector=query_vector, top_k=10, ef_search=256)

# Search returning only IDs and scores (faster, no payload transfer)
results = collection.search_ids(vector=query_vector, top_k=10)
# [{"id": 1, "score": 0.98}, {"id": 2, "score": 0.95}]

# Batch search (multiple queries in parallel)
batch_results = collection.batch_search([
    {"vector": query_vector, "top_k": 5},
    {"vector": other_vector, "top_k": 10},
])

# Multi-query fusion search (MQG pipelines)
from velesdb import FusionStrategy

results = collection.multi_query_search(
    vectors=[query1, query2, query3],  # Multiple reformulations
    k=10,
    fusion=FusionStrategy.rrf(k=60)  # RRF, average, maximum, or weighted
)

# Weighted fusion (like SearchXP scoring)
results = collection.multi_query_search(
    vectors=[v1, v2, v3],
    k=10,
    fusion=FusionStrategy.weighted(
        avg_weight=0.6,
        max_weight=0.3,
        hit_weight=0.1
    )
)

# Relative Score Fusion (linear combination of dense + sparse scores)
results = collection.multi_query_search(
    vectors=[v1, v2],
    top_k=10,
    fusion=FusionStrategy.relative_score(dense_weight=0.7, sparse_weight=0.3)
)

# Maximum fusion (take the highest score across queries)
results = collection.multi_query_search(
    vectors=[v1, v2],
    top_k=10,
    fusion=FusionStrategy.maximum()
)

# Multi-query search returning only IDs and fused scores
results = collection.multi_query_search_ids(
    vectors=[v1, v2, v3],
    top_k=10,
    fusion=FusionStrategy.rrf()
)
# [{"id": 1, "score": 0.85}]

# Hybrid dense + sparse search (fused with RRF k=60 by default)
results = collection.search_request(velesdb.SearchOptions(
    vector=query_vector,
    sparse_vector={0: 1.0, 42: 2.0},
    top_k=10,
))

# Text search (BM25)
results = collection.text_search(query="machine learning", top_k=10)

# Hybrid search (vector + text with RRF fusion)
results = collection.hybrid_search(
    vector=query_vector,
    query="machine learning",
    top_k=10,
    vector_weight=0.7  # 0.0 = text only, 1.0 = vector only
)

# Get specific points
points = collection.get([1, 2, 3])

# Delete points
collection.delete([1, 2, 3])

# Check if empty
is_empty = collection.is_empty()

# Flush to disk
collection.flush()

# VelesQL query
results = collection.query(
    "SELECT * FROM vectors WHERE category = 'tech' LIMIT 10"
)

# VelesQL with parameters
results = collection.query(
    "SELECT * FROM vectors WHERE VECTOR NEAR $query LIMIT 5",
    params={"query": query_vector}
)

# Search with metadata filter
results = collection.search_with_filter(
    vector=query_vector,
    top_k=10,
    filter={"condition": {"type": "eq", "field": "category", "value": "tech"}}
)

# Streaming insert (high-throughput, eventual consistency).
# Requires collection.enable_streaming(...) first — see below.
count = collection.stream_insert([
    {"id": 100, "vector": query_vector, "payload": {"key": "value"}}
])

# Scroll through all points in stable batches (no vector required)
# Useful for export, reindexing, or full-collection inspection.
for batch in collection.scroll(batch_size=100, filter=None):
    for point in batch:
        print(point["id"], point["payload"])

# MATCH graph traversal query (VelesQL)
results = collection.match_query(
    "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name",
    vector=query_embedding,   # optional: add similarity scoring
    threshold=0.5             # minimum similarity threshold
)

# Query returning only IDs and scores (faster, no payload)
ids = collection.query_ids("SELECT * FROM docs WHERE price > 100 LIMIT 5")

# Explain query execution plan
plan = collection.explain("SELECT * FROM docs WHERE category = 'tech' LIMIT 10")
print(plan["tree"])            # execution plan tree
print(plan["estimated_cost_ms"])
print(plan["filter_strategy"]) # "seq_scan", "index_scan", etc.

# Index management
collection.create_property_index("Document", "category")  # O(1) equality lookup
collection.create_range_index("Document", "price")         # O(log n) range queries
indexes = collection.list_indexes()
collection.drop_index("Document", "category")

# Secondary indexes on a payload field (and their memory footprint)
collection.has_secondary_index("category")                 # -> bool
collection.drop_secondary_index("category")                # -> bool (True if dropped)
collection.indexes_memory_usage()                          # -> int (total bytes)
```

### Streaming ingestion

`stream_insert()` requires streaming to be enabled first, otherwise it raises:

```python
from velesdb import StreamingIngestConfig

collection.enable_streaming(StreamingIngestConfig(batch_size=2, flush_interval_ms=10))
collection.enable_streaming()  # or: engine defaults
```

The drain task flushes asynchronously, so a point is not searchable the instant
`stream_insert()` returns — poll `collection.is_empty()` or `count()` if you
need to wait for it.

## Sparse vector search

VelesDB supports sparse vectors alongside dense vectors. Sparse vectors are useful
for learned sparse models (SPLADE, BGE-M3 sparse) and keyword-weighted representations.

```python
# Upsert points with both dense and sparse vectors
collection.upsert([
    {
        "id": 1,
        "vector": [0.1, 0.2, 0.3, 0.4],       # dense embedding
        "sparse_vector": {0: 1.5, 3: 0.8, 42: 2.1},  # {dimension_index: weight}
        "payload": {"title": "Sparse retrieval paper"}
    },
    {
        "id": 2,
        "vector": [0.5, 0.6, 0.7, 0.8],
        "sparse_vector": {3: 1.2, 7: 0.5, 42: 0.9},
        "payload": {"title": "Dense retrieval survey"}
    }
])

# Sparse-only search (no dense vector needed)
results = collection.search_request(velesdb.SearchOptions(
    sparse_vector={0: 1.0, 42: 2.0},
    k=5,
))

# Hybrid dense + sparse search (fused with RRF k=60 by default)
results = collection.search_request(velesdb.SearchOptions(
    vector=[0.15, 0.25, 0.35, 0.45],
    sparse_vector={0: 1.0, 42: 2.0},
    k=10,
))

# Named sparse indexes (e.g., separate SPLADE and BM25 sparse models)
collection.upsert([
    {
        "id": 3,
        "vector": [0.2, 0.3, 0.4, 0.5],
        "sparse_vector": {
            "splade": {10: 1.5, 20: 0.8},
            "bm25":   {5: 2.0, 15: 1.1}
        },
        "payload": {"title": "Multi-model embeddings"}
    }
])

results = collection.search_request(velesdb.SearchOptions(
    vector=[0.2, 0.3, 0.4, 0.5],
    sparse_vector={10: 1.5, 20: 0.8},
    k=10,
    sparse_index_name="splade",  # query a specific named sparse index
))
```

Sparse vectors also work with scipy sparse objects:

```python
from scipy.sparse import csr_matrix
import numpy as np

sparse_query = csr_matrix(np.array([[0.0, 1.5, 0.0, 0.8]]))
results = collection.search_request(velesdb.SearchOptions(sparse_vector=sparse_query, k=5))
```

## Fusion strategies

All multi-query and hybrid search methods accept a `FusionStrategy` to control
how scores from multiple result sets are combined.

```python
from velesdb import FusionStrategy

# Reciprocal Rank Fusion (default) -- robust to score scale differences
strategy = FusionStrategy.rrf(k=60)       # lower k = more weight to top ranks

# Average -- mean score across all queries
strategy = FusionStrategy.average()

# Maximum -- take the highest score per document
strategy = FusionStrategy.maximum()

# Weighted -- custom combination of avg, max, and hit ratio
strategy = FusionStrategy.weighted(avg_weight=0.6, max_weight=0.3, hit_weight=0.1)
strategy = FusionStrategy.weighted({"avg_weight": 0.6, "max_weight": 0.3, "hit_weight": 0.1})
strategy = FusionStrategy.weighted(0.3, 0.1)  # legacy: max_weight, hit_weight
strategy = FusionStrategy.weighted(max_weight=0.3, hit_weight=0.1)  # legacy kwargs

# Relative Score Fusion -- linear blend of dense and sparse scores
strategy = FusionStrategy.relative_score(dense_weight=0.7, sparse_weight=0.3)
strategy = FusionStrategy.rsf(dense_weight=0.7, sparse_weight=0.3)  # alias
```

| Strategy | Formula | Best For |
|----------|---------|----------|
| `rrf(k)` | sum 1/(k + rank) | Multi-query fusion, different score scales |
| `average()` | mean(scores) | Uniform query importance |
| `maximum()` | max(scores) | When any single match is sufficient |
| `weighted(a, m, h)` / `weighted(dict)` | a*avg + m*max + h*hit_ratio | Fine-grained scoring control |
| `relative_score(d, s)` / `rsf(d, s)` | d*dense + s*sparse | Dense+sparse hybrid pipelines |

## Distance metrics

| Metric | Description | Use Case |
|--------|-------------|----------|
| `cosine` | Cosine similarity (default) | Text embeddings, normalized vectors |
| `euclidean` | Euclidean (L2) distance | Image features, spatial data |
| `dot` | Dot product | When vectors are pre-normalized |
| `hamming` | Hamming distance | Binary vectors, fingerprints, hashes |
| `jaccard` | Jaccard similarity | Set similarity, tags, recommendations |

Aliases accepted for `dot`: `dotproduct`, `dot_product`, `inner`, `ip`; `l2`
maps to `euclidean` and `cos` to `cosine`. The canonical list is exported at
runtime as `velesdb.DISTANCE_METRICS` (also `velesdb.STORAGE_MODES` and
`velesdb.CONDITION_TYPES`).

## Storage modes (quantization)

Reduce memory usage with vector quantization:

```python
# Full precision (default) - 4 bytes per dimension
collection = db.create_collection("full", dimension=768, storage_mode="full")

# SQ8 quantization - 1 byte per dimension (4x compression)
collection = db.create_collection("sq8", dimension=768, storage_mode="sq8")

# Binary quantization - 1 bit per dimension (32x compression)
collection = db.create_collection("binary", dimension=768, storage_mode="binary")

# Product quantization - 8-32x compression, best for large-scale datasets
collection = db.create_collection("pq", dimension=768, storage_mode="pq")

# RaBitQ - 32x compression with scalar correction, best for high-compression with good recall
collection = db.create_collection("rabitq", dimension=768, storage_mode="rabitq")
```

| Mode | Alias | Memory per Vector (768D) | Compression | Best For |
|------|-------|-------------------------|-------------|----------|
| `full` | `f32` | 3,072 bytes | 1x | Maximum accuracy |
| `sq8` | `int8` | 768 bytes | 4x | Good accuracy/memory balance |
| `binary` | `bit` | 96 bytes | 32x | Edge/IoT, massive scale |
| `pq` | `product_quantization`, `product-quantization` | 96-384 bytes | 8-32x | Large-scale datasets, lossy |
| `rabitq` | — | 96 bytes | 32x | High-compression with good recall |

Canonical names and aliases are interchangeable: `storage_mode="f32"` is
equivalent to `storage_mode="full"`.

## Bulk loading performance

For large-scale data import, use `upsert_bulk()` instead of `upsert()`:

| Method | 10k vectors (768D) | Notes |
|--------|-------------------|-------|
| `upsert()` | ~47s | Flushes after each batch |
| `upsert_bulk()` | **~3s** | Single flush + parallel HNSW |

```python
# Recommended for bulk import
import numpy as np

vectors = np.random.rand(10000, 768).astype('float32')
points = [{"id": i, "vector": v.tolist()} for i, v in enumerate(vectors)]

collection.upsert_bulk(points)  # Batch-optimized: parallel HNSW + single flush
```

`upsert_bulk_numpy()` skips the per-element Python-float conversion entirely —
see [PYTHON_PERFORMANCE.md](PYTHON_PERFORMANCE.md).

---

Last updated: 2026-08-08 · Applies to: velesdb-core 6.0.0
