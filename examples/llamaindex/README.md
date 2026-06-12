# VelesDB + LlamaIndex Integration

> **Difficulty: Intermediate** | Showcases: Hybrid search (dense + sparse), RRF fusion, Product Quantization, LlamaIndex VectorStore interface

Example showing VelesDB as a hybrid dense+sparse vector store for LlamaIndex with Product Quantization (PQ) support, using the published [`llama-index-vector-stores-velesdb`](../../integrations/llamaindex) connector — adopting VelesDB is a single dependency change.

## Why VelesDB for LlamaIndex?

VelesDB provides a single-engine solution for hybrid search that eliminates the need to run separate systems:

- **Dense search** via HNSW with SIMD acceleration
- **Sparse search** via inverted index with MaxScore optimization
- **Hybrid fusion** via built-in Reciprocal Rank Fusion (RRF)
- **Product Quantization** for ~8x memory reduction on large collections
- **Sub-millisecond latency**, local-first, no cloud dependency

## Prerequisites

```bash
pip install llama-index-vector-stores-velesdb
```

## Usage

```bash
python hybrid_search.py
```

The example uses synthetic embeddings (random vectors) so it runs without an embedding model or API key.

## What the Example Shows

1. **`llamaindex_velesdb.VelesDBVectorStore`** — the published LlamaIndex connector
2. **Node insertion** with both dense embeddings and sparse vectors (`add(nodes, sparse_vectors=...)`)
3. **Product Quantization training** via `train_pq` (~8x compression)
4. **Dense-only search** via `query(VectorStoreQuery(...))`
5. **Hybrid dense+sparse search** by passing `sparse_vector=` (RRF fusion)
6. **Hybrid vector+BM25 search** via `hybrid_query`

## Product Quantization

The example demonstrates PQ training after inserting vectors:

```python
# Train PQ: 8 sub-quantizers, 256 centroids each
status = store.train_pq(m=8, k=256)
```

PQ divides each vector into `m` sub-spaces and quantizes each with `k` centroids, reducing memory from `dim * 4 bytes` to `m * 1 byte` per vector (when k=256). This enables scaling to millions of vectors on modest hardware. Note: training needs at least `k` vectors — with the demo's 50 documents the engine reports this and the example continues uncompressed.

## Expected Output

```
Inserted 50 nodes

=== Training Product Quantization ===
  PQ training skipped (expected with small dataset): ...

=== Dense-Only Search ===
  [0.xxxx] Benchmarking with Criterion provides statistically rigorous measurements
  ...

=== Hybrid Search (Dense + Sparse) ===
  [0.xxxx] ...

=== Hybrid Search (Vector + BM25) ===
  [0.xxxx] ...
```

## Adapting for Production

```python
from llamaindex_velesdb import VelesDBVectorStore

store = VelesDBVectorStore(
    path="./data",
    collection_name="my_docs",
)
# Provide nodes with embeddings from your model (OpenAI, HuggingFace, ...)
store.add(nodes)
```
