# LlamaIndex VelesDB Integration

[![PyPI](https://img.shields.io/pypi/v/llama-index-vector-stores-velesdb)](https://pypi.org/project/llama-index-vector-stores-velesdb/)
[![License](https://img.shields.io/badge/license-ELv2-blue)](../../LICENSE)

VelesDB vector store integration for [LlamaIndex](https://www.llamaindex.ai/).

## Features

- 🚀 **Microsecond latency** — SIMD-optimized vector search
- 📦 **Zero dependencies** — Single VelesDB binary, no external services
- 🔒 **Local-first** — All data stays on your machine
- 🧠 **RAG-ready** — Built for Retrieval-Augmented Generation

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
| `metric` | `str` | `"cosine"` | Distance metric: `cosine`, `euclidean`, `dot` |

**Methods:**

| Method | Description |
|--------|-------------|
| `add(nodes)` | Add nodes with embeddings |
| `delete(ref_doc_id)` | Delete by document ID |
| `query(query)` | Query with vector |

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

Elastic License 2.0 (ELv2)

See [LICENSE](../../LICENSE) for details.
