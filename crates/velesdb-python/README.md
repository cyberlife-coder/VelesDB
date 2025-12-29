# VelesDB Python

[![PyPI](https://img.shields.io/pypi/v/velesdb)](https://pypi.org/project/velesdb/)
[![Python](https://img.shields.io/pypi/pyversions/velesdb)](https://pypi.org/project/velesdb/)
[![License](https://img.shields.io/pypi/l/velesdb)](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE)

Python bindings for [VelesDB](https://github.com/cyberlife-coder/VelesDB) - a high-performance vector database for AI applications.

## Features

- **Vector Similarity Search**: HNSW index with SIMD-optimized distance calculations
- **Multiple Distance Metrics**: Cosine, Euclidean, Dot Product, Hamming, Jaccard
- **Persistent Storage**: Memory-mapped files for efficient disk I/O
- **Metadata Support**: Store and retrieve JSON payloads with vectors
- **NumPy Integration**: Native support for NumPy arrays

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
    metric="cosine"  # Options: "cosine", "euclidean", "dot"
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
# {"name": "documents", "dimension": 768, "metric": "cosine", "point_count": 100}

# Insert/update vectors (with immediate flush)
collection.upsert([
    {"id": 1, "vector": [...], "payload": {"key": "value"}}
])

# Bulk insert (optimized for high-throughput - 3-7x faster)
# Uses parallel HNSW insertion + single flush at the end
collection.upsert_bulk([
    {"id": i, "vector": vectors[i].tolist()} for i in range(10000)
])

# Search
results = collection.search(vector=[...], top_k=10)

# Get specific points
points = collection.get([1, 2, 3])

# Delete points
collection.delete([1, 2, 3])

# Check if empty
is_empty = collection.is_empty()

# Flush to disk
collection.flush()
```

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
| Cosine | ~76 ns | 13M ops/sec |
| Euclidean | ~47 ns | 21M ops/sec |
| Hamming | ~6 ns | 164M ops/sec |

- **Sub-millisecond** search latency
- **4x memory reduction** with SQ8 quantization
- **Millions of vectors** per collection

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
