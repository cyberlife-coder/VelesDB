# langchain-velesdb

LangChain integration for [VelesDB](https://github.com/cyberlife-coder/VelesDB) — the local knowledge engine for AI agents combining **Vector + Graph + ColumnStore** in a single Rust binary.

## Installation

```bash
pip install langchain-velesdb
```

## Quick Start

```python
from langchain_velesdb import VelesDBVectorStore
from langchain_openai import OpenAIEmbeddings

vectorstore = VelesDBVectorStore(
    path="./my_data",
    collection_name="documents",
    embedding=OpenAIEmbeddings(),
)

# Add documents
vectorstore.add_texts([
    "VelesDB combines vector, graph, and columnar storage",
    "Built entirely in Rust for microsecond latencies",
    "Perfect for RAG applications and agent memory",
])

# Search
results = vectorstore.similarity_search("fast knowledge engine", k=2)
for doc in results:
    print(doc.page_content)
```

## Usage with RAG

```python
from langchain_velesdb import VelesDBVectorStore
from langchain_openai import ChatOpenAI, OpenAIEmbeddings
from langchain.chains import RetrievalQA

vectorstore = VelesDBVectorStore.from_texts(
    texts=["Document 1 content", "Document 2 content"],
    embedding=OpenAIEmbeddings(),
    path="./rag_data",
    collection_name="knowledge_base",
)

retriever = vectorstore.as_retriever(search_kwargs={"k": 3})
qa_chain = RetrievalQA.from_chain_type(
    llm=ChatOpenAI(),
    chain_type="stuff",
    retriever=retriever,
)

answer = qa_chain.run("What is VelesDB?")
print(answer)
```

## API Reference

### VelesDBVectorStore

```python
VelesDBVectorStore(
    embedding: Embeddings,
    path: str = "./velesdb_data",
    collection_name: str = "langchain",
    metric: str = "cosine",       # "cosine", "euclidean", "dot", "hamming", "jaccard"
    storage_mode: str = "full",   # "full", "sq8", "binary"
)
```

#### Core Operations

| Method | Description |
|--------|-------------|
| `add_texts(texts, metadatas, ids)` | Add texts with optional metadata and IDs |
| `add_texts_bulk(texts, metadatas, ids)` | Bulk insert (2-3x faster for large batches) |
| `delete(ids)` | Delete documents by ID |
| `get_by_ids(ids)` | Retrieve documents by their IDs |
| `flush()` | Flush pending changes to disk |
| `from_texts(texts, embedding, ...)` | Create store from texts (class method) |
| `as_retriever(**kwargs)` | Convert to LangChain retriever |

#### Search

| Method | Description |
|--------|-------------|
| `similarity_search(query, k)` | Semantic search for similar documents |
| `similarity_search_with_score(query, k)` | Search with similarity scores |
| `similarity_search_with_filter(query, k, filter)` | Search with metadata filtering |
| `batch_search(queries, k)` | Batch search multiple queries in parallel |
| `batch_search_with_score(queries, k)` | Batch search with scores |
| `multi_query_search(queries, k, fusion, ...)` | Multi-query fusion search (RRF/Weighted) |
| `multi_query_search_with_score(queries, k, ...)` | Multi-query search with fused scores |
| `hybrid_search(query, k, vector_weight, filter)` | Hybrid vector + BM25 search |
| `text_search(query, k, filter)` | Full-text BM25 search |
| `query(velesql_str, params)` | Execute a VelesQL query |

#### Knowledge Graph

| Method | Description |
|--------|-------------|
| `add_edge(id, source, target, label, metadata)` | Add a relationship edge |
| `get_edges(label, source, target)` | Get edges (optionally filtered) |
| `traverse_graph(source, max_depth, strategy, limit)` | BFS/DFS graph traversal → `List[Document]` |
| `stream_traverse_graph(source, max_depth, strategy, limit)` | Streaming traversal → `Iterator[Document]` |
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

## Advanced Features

### Multi-Query Fusion

Search with multiple query reformulations and fuse results. Ideal for RAG pipelines using Multiple Query Generation (MQG).

```python
# Reciprocal Rank Fusion (default)
results = vectorstore.multi_query_search(
    queries=["travel to Greece", "Greek vacation", "Athens trip"],
    k=10,
)

# Weighted fusion
results = vectorstore.multi_query_search(
    queries=["travel Greece", "vacation Mediterranean"],
    k=10,
    fusion="weighted",
    fusion_params={"avg_weight": 0.6, "max_weight": 0.3, "hit_weight": 0.1},
)

# With scores
for doc, score in vectorstore.multi_query_search_with_score(
    queries=["query1", "query2"], k=5, fusion="rrf", fusion_params={"k": 60}
):
    print(f"{score:.3f}: {doc.page_content}")
```

**Fusion strategies:** `"rrf"` (default), `"average"`, `"maximum"`, `"weighted"`

### Hybrid Search (Vector + BM25)

```python
results = vectorstore.hybrid_search(
    query="machine learning performance",
    k=5,
    vector_weight=0.7,  # 70% vector, 30% BM25
)
```

### Full-Text Search (BM25)

```python
results = vectorstore.text_search("VelesDB Rust", k=5)
```

### Metadata Filtering

```python
results = vectorstore.similarity_search_with_filter(
    query="database",
    k=5,
    filter={"condition": {"type": "eq", "field": "category", "value": "tech"}},
)
```

### Knowledge Graph Operations

VelesDB is more than a vector database — it natively combines vector search with a knowledge graph. Build and traverse relationships directly from the vectorstore:

```python
# Add edges (relationships between nodes)
vectorstore.add_edge(id=1, source=100, target=200, label="KNOWS")
vectorstore.add_edge(id=2, source=100, target=300, label="WORKS_AT")

# Traverse the graph (BFS or DFS)
neighbors = vectorstore.traverse_graph(source=100, max_depth=2, strategy="bfs")
for doc in neighbors:
    print(f"Node {doc.metadata['target_id']} at depth {doc.metadata['graph_depth']}")

# Streaming traversal (memory-efficient generator)
for doc in vectorstore.stream_traverse_graph(source=100, max_depth=3):
    process(doc)

# Analyze connectivity
degree = vectorstore.get_node_degree(node_id=100)
print(f"In: {degree['in_degree']}, Out: {degree['out_degree']}")
```

### VelesQL Queries

Execute structured queries using VelesDB's SQL-like query language:

```python
results = vectorstore.query(
    "SELECT * FROM documents WHERE similarity(vector, $q) > 0.8",
    params={"q": embedding},
)
```

### Agent Memory

LangChain-compatible memory classes backed by VelesDB's agent memory system:

```python
from langchain_velesdb import VelesDBChatMemory
from langchain.chains import ConversationChain
from langchain_openai import ChatOpenAI

memory = VelesDBChatMemory(path="./agent_data")
chain = ConversationChain(llm=ChatOpenAI(), memory=memory)
response = chain.predict(input="Hello!")
```

**Available classes:** `VelesDBChatMemory` (episodic), `VelesDBSemanticMemory` (fact storage)

### Graph Toolkit

Higher-level utilities for building knowledge graphs from documents:

- **`GraphRetriever`** — Seed + expand pattern: vector search → graph traversal → combined context
- **`GraphQARetriever`** — Graph-augmented QA retriever
- **Graph Toolkit** — `chunker`, `extractor`, `loader` for document → knowledge graph pipelines

```python
from langchain_velesdb import GraphRetriever

retriever = GraphRetriever(
    vector_store=vectorstore,
    max_depth=2,
    expand_k=5,
)
docs = retriever.get_relevant_documents("What is machine learning?")
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
| `add_texts()` | `collection.upsert()` | ✅ Available |
| `similarity_search()` | `collection.search()` | ✅ Available |
| `hybrid_search()` | `collection.hybrid_search()` | ✅ Available |
| `text_search()` | `collection.text_search()` | ✅ Available |
| `multi_query_search()` | `collection.multi_query_search()` | ✅ Available |
| `query()` | `collection.query()` | ✅ Available |
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
| `match_query()` | `collection.match_query()` | ✅ Available |
| `explain()` | `collection.explain()` | ✅ Available |

## Known Limitations

- **`list_collections()`** returns `List[str]` (collection names only, not full metadata)
- **Graph streaming** uses `traverse()` + yield (native SSE streaming planned for a future SDK release)

## License

[Elastic License 2.0 (ELv2)](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE)
