# VelesDB + LangChain Integration

> **Difficulty: Intermediate** | Showcases: Hybrid search (dense + sparse), RRF fusion, LangChain VectorStore interface

Example showing VelesDB as a hybrid dense+sparse vector store for LangChain applications, using the published [`langchain-velesdb`](../../integrations/langchain) connector — adopting VelesDB is a single dependency change.

## Why VelesDB for LangChain?

Most RAG pipelines need both semantic search (dense vectors) and keyword search (sparse/BM25). This typically requires running two separate systems (e.g., Pinecone + Elasticsearch) and writing glue code to fuse results.

VelesDB handles **dense + sparse + fusion in a single engine**:

- **Dense search** via HNSW with AVX2/AVX-512 SIMD acceleration
- **Sparse search** via inverted index with MaxScore optimization
- **Hybrid fusion** via Reciprocal Rank Fusion (RRF) built in
- **Sub-millisecond latency**, local-first, no cloud dependency

## Prerequisites

```bash
pip install langchain-velesdb
```

## Usage

```bash
python hybrid_search.py
```

The example uses deterministic synthetic embeddings so it runs without an embedding model or API key. In production, replace `DemoEmbeddings` with a real embedding model (OpenAI, Sentence-Transformers, Cohere, etc.).

## What the Example Shows

1. **`langchain_velesdb.VelesDBVectorStore`** — the published LangChain connector
2. **`add_texts`** with metadata and sparse vectors in one call
3. **Dense-only search** via `similarity_search_with_score`
4. **Hybrid dense+sparse search** by passing `sparse_vector=` (RRF fusion)
5. **Hybrid vector+BM25 search** via `hybrid_search`

## Expected Output

```
Inserted 20 documents

=== Dense-Only Search ===
  [0.xxxx] LangChain provides abstractions for building LLM applications
  ...

=== Hybrid Search (Dense + Sparse) ===
  [0.xxxx] Sub-millisecond latency is critical for real-time AI applications
  ...

=== Hybrid Search (Vector + BM25) ===
  [0.xxxx] Hybrid search fuses dense and sparse signals for better recall
  ...
```

## Adapting for Production

```python
from langchain_openai import OpenAIEmbeddings
from langchain_velesdb import VelesDBVectorStore

store = VelesDBVectorStore(
    embedding=OpenAIEmbeddings(),
    path="./data",
    collection_name="my_docs",
)

# add_texts auto-generates embeddings via the embedding model
store.add_texts(["Document text here..."])
```
