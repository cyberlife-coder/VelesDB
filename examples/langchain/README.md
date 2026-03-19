# VelesDB + LangChain Integration

> **Difficulty: Intermediate** | Showcases: Hybrid search (dense + sparse), RRF fusion, LangChain VectorStore interface

Example integration showing VelesDB as a hybrid dense+sparse vector store for LangChain applications.

> **Note:** This is an example integration, not a published package. It demonstrates the integration pattern for building your own LangChain-compatible VelesDB wrapper.

## Why VelesDB for LangChain?

Most RAG pipelines need both semantic search (dense vectors) and keyword search (sparse/BM25). This typically requires running two separate systems (e.g., Pinecone + Elasticsearch) and writing glue code to fuse results.

VelesDB handles **dense + sparse + fusion in a single engine**:

- **Dense search** via HNSW with AVX2/AVX-512 SIMD acceleration
- **Sparse search** via inverted index with MaxScore optimization
- **Hybrid fusion** via Reciprocal Rank Fusion (RRF) built in
- **Sub-millisecond latency**, local-first, no cloud dependency

## Prerequisites

```bash
pip install velesdb langchain-core
```

## Usage

```bash
python hybrid_search.py
```

The example uses synthetic embeddings (random vectors) so it runs without an embedding model or API key. In production, replace the random vectors with a real embedding model (OpenAI, Sentence-Transformers, Cohere, etc.).

## What the Example Shows

1. **`VelesDBVectorStore`** class implementing LangChain's `VectorStore` interface
2. **`add_texts`** with both dense embeddings and sparse vectors in one call
3. **Dense-only search** using just embedding vectors
4. **Sparse-only search** using just keyword weights
5. **Hybrid search** combining both signals via VelesDB's built-in RRF fusion

## Expected Output

```
=== VelesDB + LangChain Hybrid Search Demo ===

--- Dense Search Results (3) ---
  [1] Machine learning is ... (score: 0.xxx)
  [2] Neural networks ...     (score: 0.xxx)
  ...

--- Sparse Search Results (3) ---
  [1] ... (score: 0.xxx)
  ...

--- Hybrid Search Results (RRF fusion, 3) ---
  [1] ... (score: 0.xxx)
  ...
```

## Adapting for Production

```python
from langchain_openai import OpenAIEmbeddings

store = VelesDBVectorStore(
    collection_name="my_docs",
    db_path="./data",
    dimension=1536,  # Match your embedding model
    embedding_function=OpenAIEmbeddings(),
)

# add_texts will auto-generate embeddings via the embedding function
store.add_texts(["Document text here..."])
```
