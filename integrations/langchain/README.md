# langchain-velesdb

LangChain integration for [VelesDB](https://github.com/cyberlife-coder/VelesDB) vector database.

## Installation

```bash
pip install langchain-velesdb
```

## Quick Start

```python
from langchain_velesdb import VelesDBVectorStore
from langchain_openai import OpenAIEmbeddings

# Initialize vector store
vectorstore = VelesDBVectorStore(
    path="./my_vectors",
    collection_name="documents",
    embedding=OpenAIEmbeddings()
)

# Add documents
vectorstore.add_texts([
    "VelesDB is a high-performance vector database",
    "Built entirely in Rust for speed and safety",
    "Perfect for RAG applications and semantic search"
])

# Search
results = vectorstore.similarity_search("fast database", k=2)
for doc in results:
    print(doc.page_content)
```

## Usage with RAG

```python
from langchain_velesdb import VelesDBVectorStore
from langchain_openai import ChatOpenAI, OpenAIEmbeddings
from langchain.chains import RetrievalQA

# Create vector store with documents
vectorstore = VelesDBVectorStore.from_texts(
    texts=["Document 1 content", "Document 2 content"],
    embedding=OpenAIEmbeddings(),
    path="./rag_data",
    collection_name="knowledge_base"
)

# Create RAG chain
retriever = vectorstore.as_retriever(search_kwargs={"k": 3})
qa_chain = RetrievalQA.from_chain_type(
    llm=ChatOpenAI(),
    chain_type="stuff",
    retriever=retriever
)

# Ask questions
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
    metric: str = "cosine"  # "cosine", "euclidean", "dot"
)
```

#### Methods

**Core Operations:**
- `add_texts(texts, metadatas=None, ids=None)` - Add texts to the store
- `add_texts_bulk(texts, metadatas=None, ids=None)` - Bulk insert (2-3x faster for large batches)
- `delete(ids)` - Delete documents by ID
- `get_by_ids(ids)` - Retrieve documents by their IDs
- `flush()` - Flush pending changes to disk

**Search:**
- `similarity_search(query, k=4)` - Search for similar documents
- `similarity_search_with_score(query, k=4)` - Search with similarity scores
- `similarity_search_with_filter(query, k=4, filter=None)` - Search with metadata filtering
- `batch_search(queries, k=4)` - Batch search multiple queries in parallel
- `batch_search_with_score(queries, k=4)` - Batch search with scores
- `multi_query_search(queries, k=4, fusion="rrf", ...)` - **Multi-query fusion search** ⭐ NEW
- `multi_query_search_with_score(queries, k=4, ...)` - Multi-query search with fused scores
- `hybrid_search(query, k=4, vector_weight=0.5, filter=None)` - Hybrid vector+BM25 search
- `text_search(query, k=4, filter=None)` - Full-text BM25 search
- `query(velesql_str, params=None)` - Execute VelesQL query

**Utilities:**
- `as_retriever(**kwargs)` - Convert to LangChain retriever
- `from_texts(texts, embedding, ...)` - Create store from texts (class method)
- `get_collection_info()` - Get collection metadata (name, dimension, point_count)
- `is_empty()` - Check if collection is empty

## Advanced Features

### Multi-Query Fusion (MQG) ⭐ NEW

Search with multiple query reformulations and fuse results using various strategies.
Perfect for RAG pipelines using Multiple Query Generation (MQG).

```python
# Basic usage with RRF (Reciprocal Rank Fusion)
results = vectorstore.multi_query_search(
    queries=["travel to Greece", "Greek vacation", "Athens trip"],
    k=10,
)

# With weighted fusion (like SearchXP's scoring)
results = vectorstore.multi_query_search(
    queries=["travel Greece", "vacation Mediterranean"],
    k=10,
    fusion="weighted",
    fusion_params={
        "avg_weight": 0.6,   # Average score weight
        "max_weight": 0.3,   # Maximum score weight  
        "hit_weight": 0.1,   # Hit ratio weight
    }
)

# Get fused scores
results_with_scores = vectorstore.multi_query_search_with_score(
    queries=["query1", "query2", "query3"],
    k=5,
    fusion="rrf",
    fusion_params={"k": 60}  # RRF parameter
)
for doc, score in results_with_scores:
    print(f"{score:.3f}: {doc.page_content}")
```

**Fusion Strategies:**
- `"rrf"` - Reciprocal Rank Fusion (default, robust to score scale differences)
- `"average"` - Mean score across all queries
- `"maximum"` - Maximum score from any query
- `"weighted"` - Custom combination of avg, max, and hit ratio

### Hybrid Search (Vector + BM25)

```python
# Combine vector similarity with keyword matching
results = vectorstore.hybrid_search(
    query="machine learning performance",
    k=5,
    vector_weight=0.7  # 70% vector, 30% BM25
)
for doc, score in results:
    print(f"{score:.3f}: {doc.page_content}")
```

### Full-Text Search (BM25)

```python
# Pure keyword-based search
results = vectorstore.text_search("VelesDB Rust", k=5)
for doc, score in results:
    print(f"{score:.3f}: {doc.page_content}")
```

### Metadata Filtering

```python
# Search with filters
results = vectorstore.similarity_search_with_filter(
    query="database",
    k=5,
    filter={"condition": {"type": "eq", "field": "category", "value": "tech"}}
)
```

## Features

- **High Performance**: VelesDB's Rust backend delivers microsecond latencies
- **SIMD Optimized**: Hardware-accelerated vector operations  
- **Multi-Query Fusion**: Native support for MQG pipelines with RRF/Weighted fusion ⭐ NEW
- **Hybrid Search**: Combine vector similarity with BM25 text matching
- **Full-Text Search**: BM25 ranking for keyword queries
- **Metadata Filtering**: Filter results by document attributes
- **Simple Setup**: Single binary, no external dependencies
- **Full LangChain Compatibility**: Works with all LangChain chains and agents

## License

MIT License (this integration)

See [LICENSE](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE) for details.
