# VelesDB Python

[![PyPI](https://img.shields.io/pypi/v/velesdb)](https://pypi.org/project/velesdb/)
[![Python](https://img.shields.io/pypi/pyversions/velesdb)](https://pypi.org/project/velesdb/)
[![License](https://img.shields.io/pypi/l/velesdb)](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE)

Python bindings for [VelesDB](https://github.com/cyberlife-coder/VelesDB) - a high-performance vector database for AI applications.

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
    print(f"  Payload: {result['payload']}")
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

# Insert/update vectors
collection.upsert([
    {"id": 1, "vector": [...], "payload": {"key": "value"}}
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

## License

Apache License 2.0

## Links

- [GitHub Repository](https://github.com/cyberlife-coder/VelesDB)
- [Documentation](https://github.com/cyberlife-coder/VelesDB#readme)
- [Issue Tracker](https://github.com/cyberlife-coder/VelesDB/issues)
