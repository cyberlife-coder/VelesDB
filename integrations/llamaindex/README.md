# LlamaIndex VelesDB Integration

[![PyPI](https://img.shields.io/pypi/v/llama-index-vector-stores-velesdb)](https://pypi.org/project/llama-index-vector-stores-velesdb/)
[![License](https://img.shields.io/badge/license-ELv2-blue)](../../LICENSE)

LlamaIndex integration for [VelesDB](https://github.com/cyberlife-coder/VelesDB) — the local knowledge engine for AI agents combining **Vector + Graph + ColumnStore** in a single Rust binary.

## Features

- **Vector + Graph + Columns** — Unified semantic search, relationships, and structured data
- **Microsecond latency** — SIMD-optimized HNSW search (57µs)
- **Local-first** — 15MB binary, zero external dependencies, works offline
- **Multi-Query Fusion** — Native RRF/Weighted/Average/Maximum strategies
- **Knowledge Graph** — Add edges, traverse BFS/DFS, analyze connectivity
- **VelesQL** — SQL-like query language with similarity functions

## Installation

```bash
pip install llama-index-vector-stores-velesdb
```

## Quick Start

```python
from llama_index.core import VectorStoreIndex, SimpleDirectoryReader
from llamaindex_velesdb import VelesDBVectorStore

vector_store = VelesDBVectorStore(
    path="./velesdb_data",
    collection_name="my_docs",
    metric="cosine",
)

documents = SimpleDirectoryReader("./data").load_data()
index = VectorStoreIndex.from_documents(documents, vector_store=vector_store)

query_engine = index.as_query_engine()
response = query_engine.query("What is VelesDB?")
print(response)
```

## Usage with Existing Index

```python
from llama_index.core import VectorStoreIndex
from llamaindex_velesdb import VelesDBVectorStore

vector_store = VelesDBVectorStore(path="./existing_data")
index = VectorStoreIndex.from_vector_store(vector_store)

query_engine = index.as_query_engine()
response = query_engine.query("Summarize the key points")
```

## API Reference

### VelesDBVectorStore

```python
VelesDBVectorStore(
    path: str = "./velesdb_data",
    collection_name: str = "llamaindex",
    metric: str = "cosine",          # "cosine", "euclidean", "dot", "hamming", "jaccard"
    storage_mode: str = "full",      # "full", "sq8", "binary"
)
```

#### Core Operations

| Method | Description |
|--------|-------------|
| `add(nodes)` | Add nodes with embeddings |
| `add_bulk(nodes)` | Bulk insert (2-3x faster for large batches) |
| `delete(ref_doc_id)` | Delete by document reference ID |
| `get_nodes(node_ids)` | Retrieve nodes by their IDs |
| `flush()` | Flush pending changes to disk |

#### Search

| Method | Description |
|--------|-------------|
| `query(query)` | Query with vector → `VectorStoreQueryResult` |
| `query_with_score_threshold(query)` | Query with minimum score threshold |
| `batch_query(queries)` | Batch query multiple vectors in parallel |
| `multi_query_search(embeddings, ...)` | Multi-query fusion search (RRF/Weighted) |
| `hybrid_query(query_str, query_embedding, ...)` | Hybrid vector + BM25 search |
| `text_query(query_str, ...)` | Full-text BM25 search |
| `velesql(query_str, params)` | Execute a VelesQL query |

#### Knowledge Graph

| Method | Description |
|--------|-------------|
| `add_edge(id, source, target, label, metadata)` | Add a relationship edge |
| `get_edges(label, source, target)` | Get edges (optionally filtered) |
| `traverse_graph(source, max_depth, strategy, limit)` | BFS/DFS traversal → `List[NodeWithScore]` |
| `stream_traverse_graph(source, max_depth, strategy, limit)` | Streaming traversal → `Iterator[NodeWithScore]` |
| `get_node_degree(node_id)` | Get in/out degree of a node |

#### Index & Collection Management

| Method | Description |
|--------|-------------|
| `create_property_index(label, property_name)` | Create a property index for faster WHERE filters |
| `list_indexes()` | List all indexes on the collection |
| `drop_index(label, property_name)` | Remove a property index |
| `list_collections()` | List all collection names → `List[str]` |
| `delete_collection(name)` | Delete a collection |
| `create_metadata_collection(name)` | Create a metadata-only collection (no vectors) |

#### Utilities

| Method | Description |
|--------|-------------|
| `get_collection_info()` | Get collection metadata (name, dimension, point_count) |
| `is_empty()` | Check if collection is empty |
| `is_metadata_only()` | Check if collection is metadata-only |

### GraphLoader

Build knowledge graphs from LlamaIndex nodes:

```python
from llamaindex_velesdb import VelesDBVectorStore, GraphLoader

vector_store = VelesDBVectorStore(path="./db")
loader = GraphLoader(vector_store)

# Add nodes (with or without vectors)
loader.add_node(id=1, label="PERSON", metadata={"name": "Alice"})
loader.add_node(id=2, label="PERSON", metadata={"name": "Bob"}, vector=[0.1, 0.2, ...])

# Add edges
loader.add_edge(id=1, source=1, target=2, label="KNOWS", metadata={"since": "2024"})

# Query edges
edges = loader.get_edges(label="KNOWS")

# Bulk load from LlamaIndex nodes
from llama_index.core.schema import TextNode
nodes = [TextNode(text="Hello world", id_="doc1")]
counts = loader.load_from_nodes(nodes, node_label="DOCUMENT")
```

| Method | Description |
|--------|-------------|
| `add_node(id, label, metadata, vector)` | Add a graph node (uses `upsert_metadata` if no vector) |
| `add_edge(id, source, target, label, metadata)` | Add a relationship edge |
| `get_edges(label)` | Get edges, optionally filtered by label |
| `load_from_nodes(nodes, node_label, extract_relations)` | Bulk load LlamaIndex nodes as graph nodes |

### GraphRetriever

Higher-level retriever implementing the "seed + expand" pattern for GraphRAG:

```python
from llamaindex_velesdb import GraphRetriever

retriever = GraphRetriever(
    index=index,
    server_url="http://localhost:8080",
    max_depth=2,
)
nodes = retriever.retrieve("What is machine learning?")
```

- **`GraphRetriever`** — Vector search → graph traversal → combined context
- **`GraphQARetriever`** — Graph-augmented QA retriever

## Advanced Features

### Multi-Query Fusion

Search with multiple query embeddings and fuse results:

```python
# Reciprocal Rank Fusion (default)
results = vector_store.multi_query_search(
    query_embeddings=[emb1, emb2, emb3],
    similarity_top_k=10,
    fusion="rrf",
    fusion_params={"k": 60},
)

# Weighted fusion
results = vector_store.multi_query_search(
    query_embeddings=[emb1, emb2],
    similarity_top_k=10,
    fusion="weighted",
    fusion_params={"avg_weight": 0.6, "max_weight": 0.3, "hit_weight": 0.1},
)

for node in results.nodes:
    print(f"{node.metadata}: {node.text[:50]}...")
```

**Fusion strategies:** `"rrf"` (default), `"average"`, `"maximum"`, `"weighted"`

### Hybrid Search (Vector + BM25)

```python
results = vector_store.hybrid_query(
    query_str="machine learning optimization",
    query_embedding=embed_model.get_query_embedding("machine learning optimization"),
    similarity_top_k=10,
    vector_weight=0.7,  # 70% vector, 30% BM25
)
```

### Full-Text Search (BM25)

```python
results = vector_store.text_query(query_str="VelesDB performance", similarity_top_k=5)
```

### Knowledge Graph Operations

VelesDB natively combines vector search with a knowledge graph. Build and traverse relationships directly from the vectorstore:

```python
# Add edges
vector_store.add_edge(id=1, source=100, target=200, label="KNOWS")

# Traverse (BFS or DFS)
neighbors = vector_store.traverse_graph(source=100, max_depth=2, strategy="bfs")
for node in neighbors:
    print(f"Node {node.metadata['target_id']} at depth {node.metadata['graph_depth']}")

# Streaming traversal (memory-efficient generator)
for node in vector_store.stream_traverse_graph(source=100, max_depth=3):
    process(node)

# Analyze connectivity
degree = vector_store.get_node_degree(node_id=100)
print(f"In: {degree['in_degree']}, Out: {degree['out_degree']}")
```

### VelesQL Queries

Execute structured queries using VelesDB's SQL-like query language:

```python
results = vector_store.velesql(
    "SELECT * FROM docs WHERE similarity(vector, $q) > 0.8",
    params={"q": embedding},
)
```

### Storage Modes

| Mode | Description | Use Case |
|------|-------------|----------|
| `"full"` | Full float32 precision (default) | Maximum accuracy |
| `"sq8"` | Scalar quantization (8-bit) | 4x memory reduction |
| `"binary"` | Binary quantization | Maximum compression |

## SDK Method Parity

This table shows the mapping between integration methods and the underlying `velesdb.Collection` SDK methods:

| Integration Method | SDK Method | Status |
|-------------------|------------|--------|
| `add()` | `collection.upsert()` | ✅ Available |
| `query()` | `collection.search()` | ✅ Available |
| `hybrid_query()` | `collection.hybrid_search()` | ✅ Available |
| `text_query()` | `collection.text_search()` | ✅ Available |
| `multi_query_search()` | `collection.multi_query_search()` | ✅ Available |
| `velesql()` | `collection.query()` | ✅ Available |
| `add_edge()` | `collection.add_edge()` | ✅ Available |
| `get_edges()` | `collection.get_edges()` / `get_edges_by_label()` | ✅ Available |
| `traverse_graph()` | `collection.traverse()` | ✅ Available |
| `stream_traverse_graph()` | `collection.traverse()` (yield) | ✅ Available |
| `get_node_degree()` | `collection.get_node_degree()` | ✅ Available |
| `create_property_index()` | `collection.create_property_index()` | ✅ Available |
| `list_indexes()` | `collection.list_indexes()` | ✅ Available |
| `drop_index()` | `collection.drop_index()` | ✅ Available |
| `list_collections()` | `db.list_collections()` | ✅ Available |
| `delete_collection()` | `db.delete_collection()` | ✅ Available |
| `match_query()` | — | ⏳ Planned v2.0 |
| `explain()` | — | ⏳ Planned v2.0 |

## Known Limitations

- **`match_query()`** raises `NotImplementedError` — MATCH execution engine planned for v2.0
- **`explain()`** raises `NotImplementedError` — query plan analysis planned for v2.0
- **`list_collections()`** returns `List[str]` (collection names only, not full metadata)
- **GraphLoader `add_node()`** without a vector uses `upsert_metadata` internally
- **AgentMemory** wrapper not yet available for LlamaIndex (available in LangChain integration)
- **Graph streaming** uses `traverse()` + yield (native SSE streaming planned for a future SDK release)

## License

[Elastic License 2.0 (ELv2)](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE)
