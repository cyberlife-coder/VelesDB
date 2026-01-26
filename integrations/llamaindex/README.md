# LlamaIndex VelesDB Integration

[![PyPI](https://img.shields.io/pypi/v/llama-index-vector-stores-velesdb)](https://pypi.org/project/llama-index-vector-stores-velesdb/)
[![License](https://img.shields.io/badge/license-ELv2-blue)](../../LICENSE)

VelesDB vector store integration for [LlamaIndex](https://www.llamaindex.ai/).

## Features

- 🚀 **Microsecond latency** — SIMD-optimized vector search
- 📦 **Zero dependencies** — Single VelesDB binary, no external services
- 🔒 **Local-first** — All data stays on your machine
- 🧠 **RAG-ready** — Built for Retrieval-Augmented Generation
- 🔀 **Multi-Query Fusion** — Native MQG support with RRF/Weighted strategies ⭐ NEW

## Installation

```bash
pip install llama-index-vector-stores-velesdb
```

## Quick Start

```python
from llama_index.core import VectorStoreIndex, SimpleDirectoryReader
from llamaindex_velesdb import VelesDBVectorStore

# Create vector store
vector_store = VelesDBVectorStore(
    path="./velesdb_data",
    collection_name="my_docs",
    metric="cosine",
)

# Load and index documents
documents = SimpleDirectoryReader("./data").load_data()
index = VectorStoreIndex.from_documents(
    documents,
    vector_store=vector_store,
)

# Query
query_engine = index.as_query_engine()
response = query_engine.query("What is VelesDB?")
print(response)
```

## Usage with Existing Index

```python
from llama_index.core import VectorStoreIndex
from llamaindex_velesdb import VelesDBVectorStore

# Connect to existing data
vector_store = VelesDBVectorStore(path="./existing_data")
index = VectorStoreIndex.from_vector_store(vector_store)

# Query
query_engine = index.as_query_engine()
response = query_engine.query("Summarize the key points")
```

## API Reference

### VelesDBVectorStore

```python
VelesDBVectorStore(
    path: str = "./velesdb_data",      # Database directory
    collection_name: str = "llamaindex", # Collection name
    metric: str = "cosine",             # Distance metric
)
```

**Parameters:**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `path` | `str` | `"./velesdb_data"` | Path to database directory |
| `collection_name` | `str` | `"llamaindex"` | Name of the collection |
| `metric` | `str` | `"cosine"` | Distance metric: `cosine`, `euclidean`, `dot`, `hamming`, `jaccard` |

**Methods:**

| Method | Description |
|--------|-------------|
| **Core Operations** | |
| `add(nodes)` | Add nodes with embeddings |
| `add_bulk(nodes)` | Bulk insert (2-3x faster for large batches) |
| `delete(ref_doc_id)` | Delete by document ID |
| `get_nodes(node_ids)` | Retrieve nodes by their IDs |
| `flush()` | Flush pending changes to disk |
| **Search** | |
| `query(query)` | Query with vector |
| `batch_query(queries)` | Batch query multiple vectors in parallel |
| `multi_query_search(embeddings, ...)` | **Multi-query fusion search** ⭐ NEW |
| `hybrid_query(query_str, query_embedding, ...)` | Hybrid vector+BM25 search |
| `text_query(query_str, ...)` | Full-text BM25 search |
| `velesql(query_str, params)` | Execute VelesQL query |
| **Utilities** | |
| `get_collection_info()` | Get collection metadata |
| `is_empty()` | Check if collection is empty |

## Advanced Features

### Multi-Query Fusion (MQG) ⭐ NEW

Search with multiple query embeddings and fuse results using various strategies.
Perfect for RAG pipelines using Multiple Query Generation (MQG).

```python
from llamaindex_velesdb import VelesDBVectorStore

vector_store = VelesDBVectorStore(path="./velesdb_data")

# Basic usage with RRF (Reciprocal Rank Fusion)
results = vector_store.multi_query_search(
    query_embeddings=[emb1, emb2, emb3],  # Multiple query reformulations
    similarity_top_k=10,
    fusion="rrf",
    fusion_params={"k": 60}
)

# With weighted fusion (like SearchXP's scoring)
results = vector_store.multi_query_search(
    query_embeddings=[emb1, emb2],
    similarity_top_k=10,
    fusion="weighted",
    fusion_params={
        "avg_weight": 0.6,   # Average score weight
        "max_weight": 0.3,   # Maximum score weight  
        "hit_weight": 0.1,   # Hit ratio weight
    }
)

for node in results.nodes:
    print(f"{node.metadata}: {node.text[:50]}...")
```

**Fusion Strategies:**
- `"rrf"` - Reciprocal Rank Fusion (default, robust to score scale differences)
- `"average"` - Mean score across all queries
- `"maximum"` - Maximum score from any query
- `"weighted"` - Custom combination of avg, max, and hit ratio

### Hybrid Search (Vector + BM25)

```python
from llamaindex_velesdb import VelesDBVectorStore

vector_store = VelesDBVectorStore(path="./velesdb_data")

# Hybrid search combining semantic and keyword matching
results = vector_store.hybrid_query(
    query_str="machine learning optimization",
    query_embedding=embedding_model.get_query_embedding("machine learning optimization"),
    similarity_top_k=10,
    vector_weight=0.7  # 70% vector, 30% BM25
)
for node in results.nodes:
    print(node.text)
```

### Full-Text Search (BM25)

```python
# Pure keyword-based search without embeddings
results = vector_store.text_query(
    query_str="VelesDB performance",
    similarity_top_k=5
)
```

## Performance

| Operation | Latency | Throughput |
|-----------|---------|------------|
| Insert (768D) | ~1 µs | 1M/s |
| Search (10K vectors) | ~2.5 ms | 400 QPS |
| Hybrid (BM25 + Vector) | ~5 ms | 200 QPS |

## Comparison with Other Stores

| Feature | VelesDB | Chroma | Pinecone |
|---------|---------|--------|----------|
| **Latency** | ~2.5 ms | ~10 ms | ~50 ms |
| **Deployment** | Local binary | Docker | Cloud |
| **Cost** | Free | Free | $$$  |
| **Offline** | ✅ | ✅ | ❌ |

## License

MIT License (this integration)

VelesDB Core is licensed under ELv2. See [LICENSE](./LICENSE) for details.
