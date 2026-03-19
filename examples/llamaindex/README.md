# VelesDB + LlamaIndex Integration

> **Difficulty: Intermediate** | Showcases: Hybrid search (dense + sparse), RRF fusion, Product Quantization, LlamaIndex VectorStore interface

Example integration showing VelesDB as a hybrid dense+sparse vector store for LlamaIndex with Product Quantization (PQ) support.

> **Note:** This is an example integration, not a published package. It demonstrates the integration pattern for building your own LlamaIndex-compatible VelesDB wrapper.

## Why VelesDB for LlamaIndex?

VelesDB provides a single-engine solution for hybrid search that eliminates the need to run separate systems:

- **Dense search** via HNSW with SIMD acceleration
- **Sparse search** via inverted index with MaxScore optimization
- **Hybrid fusion** via built-in Reciprocal Rank Fusion (RRF)
- **Product Quantization** for ~8x memory reduction on large collections
- **Sub-millisecond latency**, local-first, no cloud dependency

## Prerequisites

```bash
pip install velesdb llama-index-core
```

## Usage

```bash
python hybrid_search.py
```

The example uses synthetic embeddings (random vectors) so it runs without an embedding model or API key.

## What the Example Shows

1. **`VelesDBVectorStore`** class implementing LlamaIndex's `BasePydanticVectorStore`
2. **Node insertion** with both dense embeddings and sparse vectors
3. **Product Quantization training** for memory-efficient storage (~8x compression)
4. **Dense-only search** using embedding vectors
5. **Sparse-only search** using keyword weights
6. **Hybrid search** combining both signals via built-in RRF fusion

## Product Quantization

The example demonstrates PQ training after inserting vectors:

```python
# Train PQ: 8 sub-quantizers, 256 centroids each
status = store.train_pq(m=8, k=256)
```

PQ divides each vector into `m` sub-spaces and quantizes each with `k` centroids, reducing memory from `dim * 4 bytes` to `m * 1 byte` per vector (when k=256). This enables scaling to millions of vectors on modest hardware.

## Expected Output

```
=== VelesDB + LlamaIndex Hybrid Search Demo ===

--- Inserting 5 documents ---
Inserted 5 nodes

--- Training Product Quantization ---
PQ trained: 8 sub-quantizers, 256 centroids

--- Dense Search (3 results) ---
  [1] node_xxx (score: 0.xxx)
  ...

--- Sparse Search (3 results) ---
  [1] node_xxx (score: 0.xxx)
  ...

--- Hybrid Search (RRF fusion, 3 results) ---
  [1] node_xxx (score: 0.xxx)
  ...
```

## Adapting for Production

```python
from llama_index.embeddings.openai import OpenAIEmbedding

embed_model = OpenAIEmbedding()
store = VelesDBVectorStore(
    collection_name="my_docs",
    db_path="./data",
    dimension=1536,  # Match your embedding model
)
```
