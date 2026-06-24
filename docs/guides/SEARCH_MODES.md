# 🎯 Search Modes - Recall Configuration Guide

*Version 3.3.0 -- 2026-06-12*

Complete guide to configuring the **recall vs latency** trade-off in VelesDB. Covers dense search (HNSW), sparse search (SPLADE/BM42), and hybrid search (dense+sparse with fusion). Includes a comparison with Milvus, OpenSearch, and Qdrant practices.

---

## Table of Contents

1. [Overview](#overview)
2. [The 5 Search Modes (+Custom)](#the-5-search-modes-custom)
3. [Detailed HNSW Parameters](#detailed-hnsw-parameters)
4. [Sparse Vector Search](#sparse-vector-search)
5. [Hybrid Search](#hybrid-search)
6. [Fusion Strategies](#fusion-strategies)
7. [Comparison with the Competition](#comparison-with-the-competition)
8. [Configuration Guide by Use Case](#configuration-guide-by-use-case)
9. [API and Examples](#api-and-examples)
10. [Benchmarks](#benchmarks)
11. [FAQ](#faq)

---

## Overview

### What is Recall?

**Recall@k** measures the percentage of true nearest neighbors found among the k returned results.

```
Recall@10 = (Number of true top-10 neighbors found) / 10 × 100%
```

| Recall | Meaning |
|--------|---------------|
| **100%** | All true neighbors found (exact search) |
| **95-99%** | Excellent, sufficient for 99% of RAG/recommendation cases |
| **90-95%** | Acceptable for exploration/prototyping |
| **< 90%** | Risk of missing important results |

### The fundamental trade-off

```
                    Latency
                        ↑
                        │
          Fast ●────────┤  < 1ms    (~92% recall)
                        │
      Adaptive ●╌╌╌╌╌╌╌┤  ~1-5ms   (95%+ recall, auto-escalation)
                        │
      Balanced ●────────┤  ~2ms     (~99% recall)
                        │
      Accurate ●────────┤  ~5ms     (~99.5%+ recall)
                        │
       Perfect ●────────┤  ~15ms+   (100% recall, exhaustive HNSW)
                        │
        ────────────────┴────────────────→ Recall
                   92%      95%      99%   100%
```

> The **Adaptive** mode is shown with a dashed line because its latency varies with query difficulty.
> For easy queries (~80% of typical traffic), it is close to Fast.
> For hard queries, it automatically escalates toward Balanced/Accurate.

---

## The 5 Search Modes (+Custom)

VelesDB exposes 5 predefined **presets** plus a `Custom` mode via the `SearchQuality` enum:

### 1. Fast — Minimal latency

| Parameter | Value |
|-----------|--------|
| `ef_search` | `max(64, k × 2)` |
| Typical recall | ~92% |
| Latency (100K vecs, 768D) | < 1 ms |

**Use cases:**
- Real-time autocomplete
- "As-you-type" suggestions
- Rapid prototyping

```rust
// ef_search = max(64, k * 2) = 64
collection.search_with_ef(&query, 10, 64)?;
```

---

### 2. Balanced — Recommended default ⭐

| Parameter | Value |
|-----------|--------|
| `ef_search` | `max(128, k × 4)` |
| Typical recall | ~99% |
| Latency (100K vecs, 768D) | ~2 ms |

**Use cases:**
- RAG / Retrieval-Augmented Generation
- General semantic search
- Context-aware chatbots

```rust
// Default when unspecified
collection.search(&query, 10);
```

---

### 3. Accurate — High precision

| Parameter | Value |
|-----------|--------|
| `ef_search` | `max(512, k × 16)` |
| Typical recall | ~99.5%+ |
| Latency (100K vecs, 768D) | ~5 ms |

**Use cases:**
- Legal document search
- E-commerce (product recommendations)
- Plagiarism detection
- Medical/scientific search
- Compliance auditing
- Critical deduplication

```rust
// ef_search = max(512, k * 16) = 512
collection.search_with_ef(&query, 10, 512)?;
```

---

### 4. Perfect — Guaranteed 100% recall

| Parameter | Value |
|-----------|--------|
| Algorithm | **Exhaustive HNSW** with `ef_search = max(4096, k × 100)` |
| Recall | **100%** guaranteed (via an exhaustive candidate pool) |
| Latency (100K vecs, 768D) | ~15 ms |

**Use cases:**
- Validating/benchmarking HNSW recall
- Legal/forensic search
- Small critical datasets (< 50K vectors)

```rust
// HNSW exhaustive search: ef_search = max(4096, k * 100) = 4096
collection.search_with_ef(&query, 10, 4096)?;
```

> **Note**: Perfect mode still uses the HNSW graph, but with a candidate pool large enough to guarantee 100% recall in practice.

---

### 5. Adaptive — Adaptive optimal latency

| Parameter | Value |
|-----------|--------|
| `ef_search` | Phase 1: `min_ef` (e.g. 32). Phase 2: `min_ef × 2` if the query is hard (cap: `max_ef`) |
| Typical recall | 95%+ (≥99% on hard queries thanks to escalation) |
| Latency (100K vecs, 768D) | ~1 ms (easy queries), ~3-5 ms (hard queries) |

**Two-phase operation:**

1. Fast search with `min_ef` (e.g. 32)
2. Analyze the **spread** of the results: `(max_distance - min_distance) / min_distance`
3. If spread > 2.0 (scattered results = hard query) → re-search with doubled ef
4. If spread ≤ 2.0 (dense cluster = easy query) → return the results immediately

**Use cases:**
- Mixed workloads where most queries are easy
- APIs with a latency SLA on the P50 (not only the P99)
- Production RAG with varied queries (some close to a cluster, others ambiguous)

```rust
use velesdb_core::SearchQuality;

// Adaptive ef between 32 (easy queries) and 512 (hard queries)
let quality = SearchQuality::Adaptive { min_ef: 32, max_ef: 512 };
let results = index.search_with_quality(&query, 10, quality);
```

```sql
-- In VelesQL
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
WITH (mode = 'adaptive');
```

**Measured impact**: 2-4x reduction in median latency compared to Balanced mode, with no regression on P99 recall.

---

## Detailed HNSW Parameters

### Build-time parameters (index-time)

| Parameter | Description | VelesDB default | Impact |
|-----------|-------------|----------------|--------|
| `M` | Connections per node | **24-32** (auto) | ↑ M = ↑ recall, ↑ memory |
| `ef_construction` | Candidate pool size at build time | **300-400** (auto) | ↑ ef = ↑ index quality, ↑ build time |

### Search-time parameters (query-time)

| Parameter | Description | Range | Impact |
|-----------|-------------|-------|--------|
| `ef_search` | Candidate pool size at search time | 64 - 4096+ | ↑ ef = ↑ recall, ↑ latency |
| `k` | Number of requested results | 1 - 1000 | Must be ≤ ef_search |

### Golden rule

```
ef_search ≥ k × multiplier

Recommended multiplier per mode:
- Fast:      2x
- Balanced:  4x
- Accurate:  16x
- Perfect:   100x
```

### VelesDB auto-scaling

VelesDB automatically tunes `M` and `ef_construction` based on vector dimensionality:

| Dimension | M | ef_construction | Rationale |
|-----------|---|-----------------|---------------|
| 0-256 | 24 | 300 | Small embeddings (word2vec, MiniLM) |
| 257+ | 32 | 400 | Standard and large embeddings (BERT, OpenAI, Cohere) |

---

## Sparse Vector Search

### Overview

Sparse search uses sparse vectors, where only a few dimensions have non-zero values. This format is typical of keyword-based retrieval models such as **SPLADE**, **BM42**, or **TF-IDF**.

```
Dense vector:  [0.12, 0.45, 0.03, 0.67, 0.22, ...]  (all dimensions)
Sparse vector: {42: 0.8, 156: 0.3, 891: 0.5}         (a few dimensions)
```

### Sparse Vector Format

VelesDB stores sparse vectors as `(index, value)` pairs:

```json
{
  "sparse_vector": {
    "default": {42: 0.8, 156: 0.3, 891: 0.5, 2048: 0.1}
  }
}
```

> **Note**: The REST API also accepts the parallel-array format (`{indices: [...], values: [...]}`) for backward compatibility.

Sparse vectors support **named vectors**: a point can have several named sparse vectors (for example `"bm25"`, `"splade"`).

### Scoring

Sparse similarity is computed as the **inner product** (dot product) over the shared dimensions:

```
score = sum(query[i] * doc[i]) for every i where both vectors have a value
```

### Search Algorithms

VelesDB automatically selects the optimal algorithm:

| Algorithm | Condition | Description |
|-----------|-----------|-------------|
| **MaxScore DAAT** | Default | Document-At-A-Time with early termination. Sorts terms by contribution and skips terms that cannot improve the top-K |
| **Linear Scan** | > 30% coverage | Linear scan when the query covers more than 30% of the documents (total_postings > 0.3 * doc_count * query_nnz) |

### Accumulator

| Corpus size | Accumulator |
|-----------------|-------------|
| <= 10M documents | Dense array (O(1) access) |
| > 10M documents | FxHashMap (memory proportional to hits) |

### VelesQL Example

```sql
-- Sparse-only search
SELECT * FROM docs WHERE vector SPARSE_NEAR $keywords LIMIT 10

-- With a metadata filter
SELECT * FROM docs
WHERE vector SPARSE_NEAR $bm25_query AND category = 'tech'
LIMIT 20
```

### REST API Example

```bash
curl -X POST http://localhost:8080/collections/docs/search/sparse \
  -H "Content-Type: application/json" \
  -d '{
    "sparse_vector": {42: 0.8, 156: 0.3, 891: 0.5},
    "top_k": 10
  }'
```

### Python SDK Example

```python
import velesdb

db = velesdb.Database("./data")
coll = db.get_collection("docs")

results = coll.search_request(velesdb.SearchOptions(
    sparse_vector={42: 0.8, 156: 0.3, 891: 0.5},
    top_k=10
))
```

---

## Hybrid Search

### Overview

Hybrid search combines **dense** search (semantic embeddings) and **sparse** search (keywords) to get the best of both worlds:

- **Dense**: understands semantic meaning ("car" ~ "automobile")
- **Sparse**: precision on exact terms ("RUSTSEC-2025-0141")
- **Hybrid**: combines both for higher recall

### When to use each mode

| Mode | Strengths | Weaknesses | Use cases |
|------|--------|-----------|-------------|
| **Dense only** | Semantics, languages, paraphrases | Rare technical terms | General RAG, chatbots |
| **Sparse only** | Exact terms, acronyms, codes | No semantic understanding | Log search, error codes |
| **Hybrid** | Combines both | More compute | Production RAG, e-commerce |

### VelesQL Example

```sql
-- Hybrid search with RRF (USING FUSION is a trailing clause: after LIMIT)
SELECT * FROM products
WHERE vector NEAR $embedding AND vector SPARSE_NEAR $bm25
LIMIT 10 USING FUSION(strategy = 'rrf', k = 60)

-- Hybrid search with explicit weights
SELECT * FROM docs
WHERE vector NEAR $dense AND vector SPARSE_NEAR $sparse
LIMIT 20 USING FUSION(strategy = 'rsf', dense_weight = 0.7, sparse_weight = 0.3)
```

### REST API Example

```bash
curl -X POST http://localhost:8080/collections/docs/search \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.1, 0.2, 0.3, ...],
    "sparse_vector": {42: 0.8, 156: 0.3},
    "top_k": 10
  }'
```

When both the `vector` and `sparse_vector` fields are provided, VelesDB automatically runs a hybrid search with RRF fusion (k=60) by default.

### Python SDK Example

```python
results = coll.search_request(velesdb.SearchOptions(
    vector=[0.1, 0.2, 0.3, ...],
    sparse_vector={42: 0.8, 156: 0.3},
    top_k=10
))
```

### Parallel execution

With the `persistence` feature flag enabled (default), the dense and sparse branches are executed in parallel via `rayon::join`. Without `persistence`, they are executed sequentially.

---

## Fusion Strategies

### RRF (Reciprocal Rank Fusion)

RRF combines results by **position in the ranking**. The fused score is:

```
score_rrf(d) = 1/(k + rank_dense(d)) + 1/(k + rank_sparse(d))
```

| Parameter | Default | Description |
|-----------|--------|-------------|
| `k` | 60 | Ranking constant. Smaller k = more weight on the top ranks |

**Advantages:**
- No need to normalize scores
- Robust to scale differences between dense and sparse
- Recommended default for most cases

**VelesQL:**
```sql
USING FUSION(strategy = 'rrf', k = 60)
```

### RSF (Reciprocal Score Fusion)

RSF combines results by **normalized scores** with explicit weights:

```
score_rsf(d) = dense_weight * norm(score_dense(d)) + sparse_weight * norm(score_sparse(d))
```

Normalization is min-max per branch. `dense_weight + sparse_weight` must equal 1.0.

| Parameter | Default | Description |
|-----------|--------|-------------|
| `dense_weight` | 0.5 | Weight of the dense score |
| `sparse_weight` | 0.5 | Weight of the sparse score |

**Advantages:**
- Fine-grained control over the relative importance of each source
- Useful when one source is consistently more reliable

**VelesQL:**
```sql
USING FUSION(strategy = 'rsf', dense_weight = 0.7, sparse_weight = 0.3)
```

### RRF vs RSF Comparison

| Aspect | RRF | RSF |
|--------|-----|-----|
| **Tuning** | 1 parameter (k) | 2 parameters (weights) |
| **Normalization** | By rank (implicit) | By score (min-max) |
| **When to use** | Default, no tuning | When one source is more reliable |
| **Robustness** | Very robust | Sensitive to score distribution |

---

## Comparison with the Competition

### VelesDB vs Milvus

| Aspect | VelesDB | Milvus |
|--------|---------|--------|
| **Presets** | 4 named modes (Fast→Perfect) + Custom | No presets, manual `search_params` |
| **100% recall** | `SearchQuality::Perfect` (exhaustive HNSW) | Separate `FLAT` index |
| **Main parameter** | `SearchQuality` enum | `params={"ef": N}` |
| **Auto-tuning** | ✅ Dimension-based | ❌ Manual |

**Milvus equivalence:**
```python
# Milvus
search_params = {"metric_type": "COSINE", "params": {"ef": 128}}

# VelesDB equivalent
SearchQuality::Balanced  // ef_search = 128
```

### VelesDB vs OpenSearch

| Aspect | VelesDB | OpenSearch k-NN |
|--------|---------|-----------------|
| **Presets** | 4 modes + Custom | No presets |
| **100% recall** | Perfect mode (exhaustive HNSW) | `"method": "exact"` in mapping |
| **Parameter** | `SearchQuality` | `ef_search` in query |
| **Approach** | Query-time | Query-time or index-time |

**OpenSearch equivalence:**
```json
// OpenSearch
{
  "query": {
    "knn": {
      "vector_field": {
        "vector": [...],
        "k": 10,
        "ef_search": 512
      }
    }
  }
}

// VelesDB equivalent
SearchQuality::Accurate  // ef_search = 512
```

### VelesDB vs Qdrant

| Aspect | VelesDB | Qdrant |
|--------|---------|--------|
| **Presets** | 4 modes + Custom | No official presets |
| **100% recall** | Perfect mode (exhaustive HNSW) | `exact: true` in search |
| **Parameter** | `SearchQuality` | `hnsw_ef` in search params |
| **Quantization** | SQ8, Binary | Scalar, Product |

**Qdrant equivalence:**
```json
// Qdrant
{
  "vector": [...],
  "limit": 10,
  "params": { "hnsw_ef": 128, "exact": false }
}

// VelesDB equivalent
SearchQuality::Balanced
```

### Equivalence summary table

| VelesDB Mode | ef_search | Milvus ef | OpenSearch ef_search | Qdrant hnsw_ef |
|--------------|-----------|-----------|----------------------|----------------|
| Fast | 64 | 64 | 64 | 64 |
| Balanced | 128 | 128 | 128 | 128 |
| Accurate | 512 | 512 | 512 | 512 |
| Perfect | 4096 | FLAT index | `"exact": true` | `"exact": true` |

---

## Configuration Guide by Use Case

### 🤖 RAG / Chatbot

```rust
// Recommended production configuration (optimal latency)
SearchQuality::Adaptive { min_ef: 32, max_ef: 512 }  // 95%+, ~1-5ms depending on query

// Fixed alternative for constant recall
SearchQuality::Balanced  // ~99% recall, ~2ms

// For critical answers (medical, legal)
SearchQuality::Accurate  // ~99.5%+ recall, ~5ms
```

### 🛒 E-commerce / Recommendations

```rust
// Real-time suggestions (autocomplete)
SearchQuality::Fast  // ~92% recall, < 1ms

// Product pages (mixed easy/hard)
SearchQuality::Adaptive { min_ef: 32, max_ef: 256 }  // fast on simple queries

// Product page (precision matters)
SearchQuality::Balanced  // ~99% recall
```

### 🔍 Document search

```rust
// Exploratory search
SearchQuality::Balanced

// Legal search / audit
SearchQuality::Accurate  // or Perfect for small corpora
```

### 🧬 Scientific/medical research

```rust
// Papers, genomic sequences
SearchQuality::Accurate  // ~99.5%+ recall

// Final validation
SearchQuality::Perfect  // guaranteed 100% recall
```

### 📱 Mobile / Edge / IoT

```rust
// Critical latency, limited battery
SearchQuality::Fast

// With binary quantization for memory
HnswParams::with_binary(dimension)
```

### 🔄 Deduplication / Near-duplicate detection

```rust
// Exact duplicate detection
SearchQuality::Perfect  // No false negatives

// Approximate detection (OK if a few duplicates slip through)
SearchQuality::Accurate
```

---

## API and Examples

### Rust

```rust
use velesdb_core::VectorCollection;

// Method 1: Default mode (Balanced, ef_search=128)
let results = collection.search(&query_vector, 10)?;

// Method 2: Custom ef_search (high precision)
let results = collection.search_with_ef(&query_vector, 10, 1024)?;

// Method 3: ef_search for fast mode
let results = collection.search_with_ef(&query_vector, 10, 64)?;

// Method 4: Perfect mode (exhaustive HNSW, ef_search=4096)
let results = collection.search_with_ef(&query_vector, 10, 4096)?;
```

### REST API

```bash
# Default mode (Balanced)
curl -X POST http://localhost:8080/collections/my_collection/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [0.1, 0.2, ...], "top_k": 10}'

# Custom ef_search
curl -X POST http://localhost:8080/collections/my_collection/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [0.1, 0.2, ...], "top_k": 10, "ef_search": 512}'

# Mode via the "mode" parameter (v1.9.2)
curl -X POST http://localhost:8080/collections/my_collection/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [0.1, 0.2, ...], "top_k": 10, "mode": "accurate"}'

# Custom ef_search via "mode" (v1.9.2)
curl -X POST http://localhost:8080/collections/my_collection/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [0.1, 0.2, ...], "top_k": 10, "mode": "custom:256"}'

# Adaptive ef_search via "mode" (v1.9.2)
curl -X POST http://localhost:8080/collections/my_collection/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [0.1, 0.2, ...], "top_k": 10, "mode": "adaptive:32:512"}'
```

### VelesQL

```sql
-- Default mode (Balanced)
SELECT * FROM my_collection
WHERE vector NEAR $query
LIMIT 10;

-- Explicit mode
SELECT * FROM my_collection
WHERE vector NEAR $query
LIMIT 10
WITH (mode = 'accurate');

-- Adaptive mode (optimal latency for mixed workloads)
SELECT * FROM my_collection
WHERE vector NEAR $query
LIMIT 10
WITH (mode = 'adaptive');

-- Custom ef_search
SELECT * FROM my_collection
WHERE vector NEAR $query
LIMIT 10
WITH (ef_search = 512);
```

### CLI REPL

```
velesdb> \set mode balanced
mode = balanced

velesdb> \set ef_search 256
ef_search = 256

velesdb> \show

Session Settings
  mode = balanced
  ef_search = 256
  timeout_ms = 30000
  rerank = true
  max_results = 100
  collection = (none)

velesdb> SELECT * FROM products WHERE vector NEAR $v LIMIT 10;
```

> Note: `max_results` is a REPL display setting. Independently of the CLI,
> the engine applies a default `LIMIT 10` to any SELECT without an explicit
> `LIMIT` clause (exceptions: `MATCH ... RETURN` and UNION/INTERSECT/EXCEPT
> queries, which have no default).

---

## Benchmarks

### Test conditions

- **CPU**: AMD Ryzen 9 5900X (12 cores)
- **RAM**: 64 GB DDR4
- **Dataset**: 100K vectors, 768 dimensions (OpenAI embeddings)
- **Metric**: Cosine similarity

### Results

| Mode | ef_search | Recall@10 | p50 latency | p99 latency | QPS |
|------|-----------|-----------|-------------|-------------|-----|
| Fast | 64 | ~92% | 0.8 ms | 1.5 ms | 12,500 |
| Balanced | 128 | ~99% | 1.9 ms | 3.2 ms | 5,200 |
| Accurate | 512 | ~99.5% | 4.1 ms | 6.8 ms | 2,400 |
| Perfect | 4096 | 100.0% | 14.2 ms | 22.1 ms | 700 |

### Scaling with dataset size

| Dataset Size | Balanced Latency | Perfect Latency | Ratio |
|--------------|------------------|-----------------|-------|
| 10K | 0.4 ms | 5 ms | 12x |
| 100K | 1.9 ms | 48 ms | 25x |
| 500K | 3.2 ms | 240 ms | 75x |
| 1M | 4.8 ms | 480 ms | 100x |

> **Observation**: The Fast, Balanced, and Accurate modes scale in O(log n) thanks to HNSW. Perfect mode also uses HNSW but with a very large candidate pool, which increases latency. For very large datasets, Accurate offers an excellent recall/latency trade-off.

---

## FAQ

### Q: Which mode should I pick for RAG?

**A:** `Balanced` (default) fits 95% of RAG cases. If you have legal/medical requirements, use `Accurate`.

### Q: Is Perfect mode really 100% recall?

**A:** Yes, guaranteed in practice. It uses HNSW with an exhaustive candidate pool (`ef_search = max(4096, k * 100)`), which forces the graph to explore enough nodes to find all true neighbors.

### Q: Can I use Perfect in production?

**A:** Yes, but with precautions:
- Datasets < 50K: Acceptable (~25ms)
- Datasets 50K-200K: Critical cases only
- Datasets > 200K: Recommended only for batch/offline workloads

### Q: How do I measure the recall of my index?

**A:** Compare ANN vs Perfect results on a sample:

```rust
// Benchmark recall
let ann_results = collection.search(&query, 10)?;           // Balanced (ef_search=128)
let exact_results = collection.search_with_ef(&query, 10, 4096)?; // Perfect (100% recall)

let recall = calculate_recall(&ann_results, &exact_results);
println!("Recall@10: {:.1}%", recall * 100.0);
```

### Q: Can ef_search exceed the number of vectors?

**A:** Yes, but beyond a certain threshold, the recall gain is negligible while latency increases significantly. Perfect mode (`ef_search = 4096`) is already calibrated to guarantee 100% recall.

### Q: Milvus uses `ef` and VelesDB uses `ef_search` — are they the same thing?

**A:** Yes, they are the same. `ef_search` is the standard name in the HNSW literature.

---

## Resources

- [Original HNSW paper (Malkov & Yashunin, 2018)](https://arxiv.org/abs/1603.09320)
- [Milvus HNSW tuning guide](https://milvus.io/docs/index-with-milvus.md)
- [OpenSearch k-NN performance guide](https://opensearch.org/docs/latest/search-plugins/knn/performance-tuning/)
- [Qdrant HNSW configuration](https://qdrant.tech/documentation/concepts/indexing/)

---

*VelesDB Documentation -- 2026-06-12*
