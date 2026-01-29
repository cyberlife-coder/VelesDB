# VelesDB Python

[![PyPI](https://img.shields.io/pypi/v/velesdb)](https://pypi.org/project/velesdb/)
[![Python](https://img.shields.io/pypi/pyversions/velesdb)](https://pypi.org/project/velesdb/)
[![License](https://img.shields.io/pypi/l/velesdb)](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE)

Python bindings for [VelesDB](https://github.com/cyberlife-coder/VelesDB) - a high-performance vector database for AI applications.

## Features

- **Vector Similarity Search**: HNSW index with SIMD-optimized distance calculations
- **Multi-Query Fusion**: Native MQG support with RRF/Weighted fusion strategies
- **Multiple Distance Metrics**: Cosine, Euclidean, Dot Product, Hamming, Jaccard
- **Persistent Storage**: Memory-mapped files for efficient disk I/O
- **Metadata Support**: Store and retrieve JSON payloads with vectors
- **NumPy Integration**: Native support for NumPy arrays
- **Type Hints**: Full `.pyi` stub file for IDE autocompletion

## Installation

```bash
pip install velesdb
```

## Quick Start

```python
import velesdb

# Open or create a database
db = velesdb.Database("./my_vectors")

# Create a collection for 768-dimensional vectors (e.g., BERT embeddings)
collection = db.create_collection(
    name="documents",
    dimension=768,
    metric="cosine"  # Options: "cosine", "euclidean", "dot", "hamming", "jaccard"
)

# Insert vectors with metadata
collection.upsert([
    {
        "id": 1,
        "vector": [0.1, 0.2, ...],  # 768-dim vector
        "payload": {"title": "Introduction to AI", "category": "tech"}
    },
    {
        "id": 2,
        "vector": [0.3, 0.4, ...],
        "payload": {"title": "Machine Learning Basics", "category": "tech"}
    }
])

# Search for similar vectors
results = collection.search(
    vector=[0.15, 0.25, ...],  # Query vector
    top_k=5
)

for result in results:
    print(f"ID: {result['id']}, Score: {result['score']:.4f}")
    print(f"Payload: {result['payload']}")
```

## API Reference

### Database

```python
# Create/open database
db = velesdb.Database("./path/to/data")

# List collections
names = db.list_collections()

# Create collection
collection = db.create_collection("name", dimension=768, metric="cosine")

# Get existing collection
collection = db.get_collection("name")

# Delete collection
db.delete_collection("name")
```

### Collection

```python
# Get collection info
info = collection.info()
# {"name": "documents", "dimension": 768, "metric": "cosine", "storage_mode": "full", "point_count": 100}

# Insert/update vectors (with immediate flush)
collection.upsert([
    {"id": 1, "vector": [...], "payload": {"key": "value"}}
])

# Bulk insert (optimized for high-throughput - 3-7x faster)
# Uses parallel HNSW insertion + single flush at the end
collection.upsert_bulk([
    {"id": i, "vector": vectors[i].tolist()} for i in range(10000)
])

# Vector search
results = collection.search(vector=[...], top_k=10)

# Batch search (multiple queries in parallel)
batch_results = collection.batch_search(
    vectors=[[0.1, 0.2, ...], [0.3, 0.4, ...]],
    top_k=5
)

# Multi-query fusion search (MQG pipelines) ⭐ NEW
from velesdb import FusionStrategy

results = collection.multi_query_search(
    vectors=[query1, query2, query3],  # Multiple reformulations
    top_k=10,
    fusion=FusionStrategy.rrf(k=60)  # RRF, average, maximum, or weighted
)

# Weighted fusion (like SearchXP scoring)
results = collection.multi_query_search(
    vectors=[v1, v2, v3],
    top_k=10,
    fusion=FusionStrategy.weighted(
        avg_weight=0.6,
        max_weight=0.3,
        hit_weight=0.1
    )
)

# Text search (BM25)
results = collection.text_search(query="machine learning", top_k=10)

# Hybrid search (vector + text with RRF fusion)
results = collection.hybrid_search(
    vector=[0.1, 0.2, ...],
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

# VelesQL query (v0.8.5+)
results = collection.query(
    "SELECT * FROM vectors WHERE category = 'tech' LIMIT 10"
)

# VelesQL with parameters
results = collection.query(
    "SELECT * FROM vectors WHERE VECTOR NEAR $query LIMIT 5",
    params={"query": [0.1, 0.2, ...]}
)

# Search with metadata filter
results = collection.search_with_filter(
    vector=[0.1, 0.2, ...],
    top_k=10,
    filter={"condition": {"type": "eq", "field": "category", "value": "tech"}}
)
```

### Storage Modes (Quantization)

Reduce memory usage with vector quantization:

```python
# Full precision (default) - 4 bytes per dimension
collection = db.create_collection("full", dimension=768, storage_mode="full")

# SQ8 quantization - 1 byte per dimension (4x compression)
collection = db.create_collection("sq8", dimension=768, storage_mode="sq8")

# Binary quantization - 1 bit per dimension (32x compression)
collection = db.create_collection("binary", dimension=768, storage_mode="binary")
```

| Mode | Memory per Vector (768D) | Compression | Best For |
|------|-------------------------|-------------|----------|
| `full` | 3,072 bytes | 1x | Maximum accuracy |
| `sq8` | 768 bytes | 4x | Good accuracy/memory balance |
| `binary` | 96 bytes | 32x | Edge/IoT, massive scale |

### Bulk Loading Performance

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

collection.upsert_bulk(points)  # 7x faster than upsert()
```

## Distance Metrics

| Metric | Description | Use Case |
|--------|-------------|----------|
| `cosine` | Cosine similarity (default) | Text embeddings, normalized vectors |
| `euclidean` | Euclidean (L2) distance | Image features, spatial data |
| `dot` | Dot product | When vectors are pre-normalized |
| `hamming` | Hamming distance | Binary vectors, fingerprints, hashes |
| `jaccard` | Jaccard similarity | Set similarity, tags, recommendations |

## Performance

VelesDB is built in Rust with explicit SIMD optimizations:

| Operation | Time (768d) | Throughput |
|-----------|-------------|------------|
| Cosine | ~93 ns | 11M ops/sec |
| Euclidean | ~46 ns | 22M ops/sec |
| Dot Product | ~36 ns | 28M ops/sec |
| Hamming | ~6 ns | 164M ops/sec |

### Benchmark: VelesDB vs pgvector (HNSW)

Tested on clustered embeddings (768D) — realistic AI workloads:

| Dataset Size | VelesDB Recall | VelesDB P50 | pgvector P50 | **Speedup** |
|--------------|----------------|-------------|--------------|-------------|
| **1,000** | 100.0% | **0.5ms** | 50ms | **100x** |
| **10,000** | 99.0% | **2.5ms** | 50ms | **20x** |
| **100,000** | 97.8% | **4.3ms** | 50ms | **12x** |

- **12-100x faster** than pgvector depending on dataset size
- **97-100% recall** across all scales
- **Sub-5ms latency** even at 100k vectors

## Requirements

- Python 3.9+
- No external dependencies (pure Rust engine)
- Optional: NumPy for array support

## License

Elastic License 2.0 (ELv2)

See [LICENSE](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE) for details.

## Links

- [GitHub Repository](https://github.com/cyberlife-coder/VelesDB)
- [Documentation](https://github.com/cyberlife-coder/VelesDB#readme)
- [Issue Tracker](https://github.com/cyberlife-coder/VelesDB/issues)
