# 📊 VelesDB Performance Benchmarks

*Last updated: December 30, 2025 (v0.5.0 - Rounds 7-9 Optimization)*

This document details the performance benchmarks for VelesDB. Tests were conducted on a standard workstation (8-core CPU, AVX2/AVX-512 support).

---

## 🚀 v0.5.0 Headline: VelesDB 5.3x Faster + 98.8% Recall

### Performance Summary (50,000 vectors, 768D, Docker)

| Metric | pgvector | VelesDB REST | Winner |
|--------|----------|--------------|--------|
| **Insert + Index** | 154.59s | **29.02s** | **VelesDB 5.3x** ✅ |
| **Search P50** | 50.4ms | **49.5ms** | **VelesDB** ✅ |
| **Search P99** | 60.0ms | 60.6ms | ~Equal |
| **Recall@10** | **100%** | 98.8% | pgvector (-1.2%) |

### Why VelesDB REST is Slower on Search

The ~2ms latency difference is due to **protocol overhead**, not algorithm performance:

```
pgvector:  Binary PostgreSQL protocol (~0.5ms overhead)
VelesDB:   HTTP/JSON REST API (~2-3ms overhead)
```

This is architectural and cannot be optimized away.

### Where VelesDB Truly Shines: Embedded Mode

| Mode | VelesDB | pgvector | Winner |
|------|---------|----------|--------|
| **Embedded (PyO3)** | **2.5ms** | 50ms | **VelesDB 20x** ✅ |

### Key Optimizations in v0.5.0

- **SIMD-accelerated HNSW** - AVX2/SSE distance calculations via `simdeez_f`
- **Parallel insertion** - Native Rayon-based graph construction
- **Deferred index save** - No disk I/O during batch operations
- **Async-safe server** - `spawn_blocking` for bulk operations

---

## 🆚 Competitor Comparison: VelesDB vs pgvector (HNSW)

We benchmarked VelesDB against [pgvector](https://github.com/pgvector/pgvector) HNSW on **clustered embeddings (768D)** — realistic AI workloads.

### Test Configuration

| Parameter | Value |
|-----------|-------|
| **Datasets** | 50k vectors (realistic scale) |
| **Dimensions** | 768 (OpenAI/Cohere-sized) |
| **Data Type** | Clustered embeddings (50 clusters) |
| **Queries** | 100 queries |
| **Top-K** | 10 results |
| **Metric** | Cosine similarity |
| **VelesDB** | REST API (Docker), `upsert_bulk()` |
| **pgvector** | Docker, HNSW index (m=16, ef_construction=200) |

### Fair Comparison Methodology

Both databases measured with **total time including index construction**:

- **VelesDB**: Insert + inline HNSW indexing
- **pgvector**: Raw INSERT + CREATE INDEX time

This ensures an apples-to-apples comparison of the complete ingestion pipeline.

### Results: Docker vs Docker (Fair Comparison - 50k vectors)

| Metric | pgvector | VelesDB REST | Winner |
|--------|----------|--------------|--------|
| **Insert + Index** | 154.59s | **29.02s** | **VelesDB 5.3x** ✅ |
| **Search P50** | 50.4ms | **49.5ms** | **VelesDB** ✅ |
| **Search P99** | 60.0ms | 60.6ms | ~Equal |
| **Recall@10** | **100%** | 98.8% | pgvector (-1.2%) |

### Embedded Mode (VelesDB's Strength)

| Dataset | VelesDB (native) | pgvector (Docker) | Speedup |
|---------|------------------|-------------------|---------|
| 10,000 | **2.5ms** | 50ms | **20x** |

### Key Findings

**Insertion (v0.5.0 Rounds 7-9):**
- VelesDB **5.3x faster** than pgvector for bulk imports (50k vectors)
- SIMD + parallel insertion + O(1) storage lookup optimizations
- Higher M=20 and ef_construction=300 for better recall (98.8%)

**Search:**
- Embedded mode: VelesDB 20x faster (no network overhead)
- REST API mode: pgvector slightly faster (optimized PostgreSQL stack)

**Recall:**
- Both achieve **99-100% recall** with equivalent HNSW parameters
- VelesDB 99.7% vs pgvector 100% — negligible difference

### When to Choose Each

| Use Case | Recommendation | Why |
|----------|----------------|-----|
| **Bulk import speed** | **VelesDB** ✅ | 6.8x faster insertion |
| **Embedded/Desktop apps** | **VelesDB** ✅ | 20x faster (no network) |
| **Edge/IoT/WASM** | **VelesDB** ✅ | Single binary, no deps |
| **Real-time search** | Depends | Embedded: VelesDB, REST: pgvector |
| **Existing PostgreSQL** | **pgvector** ✅ | Native integration |
| **100% recall required** | **pgvector** ✅ | Better graph structure |

### How to Reproduce

```bash
cd benchmarks/
docker-compose up -d --build  # Start both servers
pip install -r requirements.txt
python benchmark_docker.py --vectors 50000 --clusters 50
```

> 📂 **Benchmark kit**: See [benchmarks/](../benchmarks/) for the complete reproducible test suite.

> 📈 **See also**: [Performance Optimization Roadmap](./PERFORMANCE_ROADMAP.md) for planned improvements.
>
> 🔧 **See also**: [Benchmarking Guide](./BENCHMARKING_GUIDE.md) for reproducible benchmark setup.

---

## ⚠️ What We Measure (Important Disclaimer)

VelesDB benchmarks measure **kernel-level performance**:

| Measured | NOT Measured (yet) |
|----------|-------------------|
| Pure SIMD distance computations | End-to-end query latency with I/O |
| In-memory operations | Network/disk overhead |
| Single-threaded throughput | Full multi-threaded scaling |
| **HNSW Recall@k** (91-99.8%) | Disk-based index performance |

### 🎯 Our Focus

> **"VelesDB focuses on raw CPU efficiency and predictable microsecond latency for in-memory workloads."**

### 📊 Recall Options

VelesDB v0.2 provides **both exact and approximate search**:

| Mode | Recall | Use Case |
|------|--------|----------|
| **Brute-force** | 100% | Small datasets (<10k), quality-critical |
| **HNSW HighRecall** | 99.4% | Large datasets, near-exact results |
| **HNSW Balanced** | 98.0% | Best performance/quality tradeoff |
| **HNSW Fast** | 90.2% | Maximum speed, acceptable quality |

See [Search Quality](#-search-quality-recallk) for detailed benchmarks.

### 🔍 Real-World Considerations

Actual end-to-end latency includes:
- Query parsing (~500ns for simple queries)
- Memory allocation overhead
- Result serialization
- Network latency (for REST API)

The benchmarks below isolate **compute performance** to help you understand the raw efficiency of VelesDB's core algorithms.

---

## 🚀 Summary

| Operation | Metric | Time (768d) | Throughput | Speedup vs Baseline |
|-----------|--------|-------------|------------|---------------------|
| **Cosine Similarity** | Latency | **~81 ns** | ~12M ops/sec | **3.3x** |
| **Euclidean Distance** | Latency | **~49 ns** | ~20M ops/sec | **5.3x** |
| **Dot Product** | Latency | **~39 ns** | ~26M ops/sec | **6.8x** |
| **Hamming (Binary)** | Latency | **~6 ns** | ~164M ops/sec | **~34x** (vs f32) |
| **ColumnStore Filter** | Eq String (100k) | **~27 µs** | ~3.7M items/sec | **122x** vs JSON |
| **VelesQL Parser** | Simple | **~528 ns** | ~1.9M qps | - |
| **VelesQL Cache Hit** | Cached | **~15 ns** | ~67M qps | **35x** vs parse |

> **Note**: All distance functions now use explicit SIMD via the `wide` crate (f32x8). The ColumnStore provides columnar filtering that is 44-122x faster than JSON-based filtering. Query caching provides 35x speedup for repetitive workloads.

---

## ⚡ SIMD Vector Operations

Comparison between standard Rust iterators (Baseline) and VelesDB's explicit SIMD optimizations (Optimized).
Results are from `search_benchmark` which measures the full public API call overhead.

### Cosine Similarity (768 dimensions)
> Used for semantic search and text embeddings.

| Implementation | Time per op | Throughput |
|----------------|-------------|------------|
| Baseline (Auto-vec) | 265 ns | 3.8M ops/s |
| **VelesDB Optimized (SIMD)** | **81 ns** | **12M ops/s** |
| **Improvement** | **-69% latency** | **3.3x throughput** |

### Euclidean Distance (768 dimensions)
> Used for spatial data and image features.

| Implementation | Time per op | Throughput |
|----------------|-------------|------------|
| Baseline (Auto-vec) | 258 ns | 3.9M ops/s |
| **VelesDB Optimized (SIMD)** | **49 ns** | **20M ops/s** |
| **Improvement** | **-81% latency** | **5.3x throughput** |

### Dot Product (768 dimensions)
> Used for raw similarity and inner product.

| Implementation | Time per op | Throughput |
|----------------|-------------|------------|
| Baseline (Auto-vec) | 265 ns | 3.8M ops/s |
| **VelesDB Optimized (SIMD)** | **39 ns** | **26M ops/s** |
| **Improvement** | **-85% latency** | **6.8x throughput** |

### Binary Hamming Distance (768 bits / 12 u64)
> Used for binary fingerprints and image hashing.

| Implementation | Time per op | Throughput |
|----------------|-------------|------------|
| Float32 Baseline | ~206 ns | 4.8M ops/s |
| **VelesDB Optimized** | **~6.1 ns** | **164M ops/s** |
| **Improvement** | **-97% latency** | **~34x throughput** |

---

## 🔍 Metadata Filtering

Benchmarks for filtering operations.

| Filter Type | Condition | Time (10k items) | Throughput |
|-------------|-----------|------------------|------------|
| **Equality** | `category = 'tech'` | ~194 µs | 51M items/s |
| **In List** | `status IN ('a', 'b')` | ~242 µs | 41M items/s |
| **Range** | `price > 100` | ~416 µs | 24M items/s |
| **Nested** | `meta.tags CONTAINS 'a'` | ~355 µs | 28M items/s |
| **Complex** | `(A AND B) OR C` | ~500 µs | 20M items/s |

### Throughput vs Scale

| Dataset Size | Time per Batch | Effective Throughput |
|--------------|----------------|----------------------|
| 1,000 items | 14.6 µs | **68.5 M/s** |
| 10,000 items | 151 µs | **66.2 M/s** |
| 100,000 items | 5.15 ms | **19.4 M/s** |

> **Note**: For high-performance filtering at scale, use the new `ColumnStore` module which provides 44-122x faster filtering than JSON. See `column_store.rs` for bitmap-based filtering that supports efficient AND/OR combinations.

---

## 📝 VelesQL Parsing

Performance of the SQL-like query parser.

| Query Type | Complexity | Time | Throughput |
|------------|------------|------|------------|
| **Simple** | `SELECT * FROM table` | 528 ns | 1.9M qps |
| **Vector** | `... WHERE vector NEAR $v` | 835 ns | 1.2M qps |
| **Complex** | Multiple conditions | 3.6 µs | 277k qps |

### Query Cache

For repetitive workloads, use `QueryCache` to avoid re-parsing identical queries:

| Scenario | Time | Throughput | Improvement |
|----------|------|------------|-------------|
| **Direct Parse** | 528 ns | 1.9M qps | baseline |
| **Cache Miss** | ~600 ns | 1.7M qps | +14% overhead |
| **Cache Hit** | **~15 ns** | **67M qps** | **35x faster** |

```rust
use velesdb_core::velesql::QueryCache;

// Create cache with max 1000 entries
let cache = QueryCache::new(1000);

// First call: cache miss (parses query)
let query = cache.parse("SELECT * FROM docs LIMIT 10")?;

// Subsequent calls: cache hit (~35x faster)
let query = cache.parse("SELECT * FROM docs LIMIT 10")?;

// Check hit rate
println!("Hit rate: {:.1}%", cache.stats().hit_rate());
```

> **Tip**: For REST API servers with repetitive queries, `QueryCache` can reduce parsing overhead by 95%+.

---

## 📈 Search Quality (Recall@k)

VelesDB provides configurable recall/latency tradeoffs through search quality profiles.

### HNSW Recall by Quality Profile

Measured on 10,000 vectors (128 dimensions) with cosine similarity.
Index built with `HnswParams::max_recall()` (M=32, ef_construction=500):

| Quality Profile | ef_search | Recall@10 | Latency (k=10) |
|-----------------|-----------|-----------|----------------|
| **Fast** | 64 | **89.2%** | ~3.5ms |
| **Balanced** | 128 | **98.2%** | ~7.5ms |
| **Accurate** | 256 | **99.4%** | ~11ms |
| **HighRecall** | 512 | **99.6%** | ~26ms |

> **Note**: These results use `HnswParams::max_recall()` for quality-critical applications.
> For faster indexing with slightly lower recall, use `HnswParams::auto()` or `HnswParams::fast_indexing()`.

### Available HNSW Parameter Presets

| Preset | M | ef_construction | Use Case |
|--------|---|-----------------|----------|
| `auto(dim)` | 16-32 | 200-500 | General purpose |
| `high_recall(dim)` | 24-40 | 400-700 | Quality-sensitive |
| `max_recall(dim)` | 32-64 | 500-1000 | Maximum quality |
| `fast_indexing(dim)` | 8-16 | 100-250 | Fast bulk inserts |

### Brute Force (Exact Search)

For applications requiring **100% recall**, use brute-force search:

| Dataset Size | Recall@k | Latency (k=10) |
|--------------|----------|----------------|
| 1,000 | **100%** | ~0.5ms |
| 10,000 | **100%** | ~5ms |
| 100,000 | **100%** | ~50ms |

> **Tip**: For datasets under 10k vectors, brute-force may be faster than HNSW index construction overhead.

### How to Run Recall Benchmarks

```bash
cargo bench --bench recall_benchmark
```

---

## 🚀 Parallel Search (Multi-Core Scaling)

VelesDB supports parallel search operations using Rayon for multi-core scaling.

### Batch Query Parallelization

Process multiple queries in parallel:

| Queries | Sequential | Parallel (8 cores) | Speedup |
|---------|------------|-------------------|---------|
| 100 | ~86ms | ~4.5ms | **19x** |
| 1000 | ~860ms | ~45ms | **19x** |

### Brute-Force Parallel Search

Exact search with 100% recall, parallelized across cores:

| Dataset | 1 Thread | 2 Threads | 4 Threads | 8 Threads | Scaling |
|---------|----------|-----------|-----------|-----------|--------|
| 1,000 | ~0.8ms | - | - | ~0.25ms | ~3x |
| 10,000 | ~2.6ms | - | - | ~0.9ms | ~3x |
| 50,000 | ~7.7ms | ~4.7ms | ~3.4ms | ~2.9ms | **2.7x** |

> **Note**: Scaling efficiency depends on memory bandwidth and CPU cache hierarchy. NUMA systems may see reduced scaling on cross-socket access.

### Parallel Batch Insert

Bulk vector insertion using `insert_batch_parallel` vs sequential `insert`:

| Vectors | Batch Parallel | Sequential | Speedup |
|---------|----------------|------------|---------|
| 1,000 | **17 ms** | 200 ms | **12x** |
| 5,000 | **167 ms** | 2.6 s | **16x** |
| 10,000 | **445 ms** | 8.1 s | **18x** |

> **Perf**: Refactored `HnswMappings` to reduce lock contention from 4 locks to 2 locks per insert operation. Combined with `hnsw_rs` native parallel insertion, this enables massive speedups for bulk imports.

### API Usage

```rust
// Batch parallel search (multiple queries)
let results = index.search_batch_parallel(&queries, k, SearchQuality::Balanced);

// Exact brute-force with 100% recall (parallelized)
let results = index.brute_force_search_parallel(&query, k);
```

### How to Run Parallel Benchmarks

```bash
cargo bench --bench parallel_benchmark
```

---

## 🔥 Performance Optimizations (v0.3.1)

New optimizations added in v0.3.1 for maximum throughput:

### ContiguousVectors + Prefetch

| Benchmark | Result | Improvement |
|-----------|--------|-------------|
| Random Access | **2.3 Gelem/s** | +12% with prefetch |
| Insert (128D) | **100M elem/s** | Contiguous layout |
| Insert (768D) | **1.84M elem/s** | Batch WAL |
| Bulk Import | **15.4K vec/s** | 10x vs regular upsert |

### Optimizations Under the Hood

- **64-byte aligned memory**: Cache line optimization
- **CPU prefetch hints**: L2 cache warming for HNSW traversal
- **Batch WAL writes**: Single disk write per bulk import
- **Zero-copy mmap**: Memory-mapped files for instant startup

### How to Run Performance Benchmarks

```bash
cargo bench --bench perf_benchmark
```

---

## 🧪 Methodology

- **Hardware**: Windows Workstation, 8-core CPU, 32GB RAM
- **Environment**: Rust 1.83, Release build (`--release`)
- **Framework**: Criterion.rs
- **Optimizations**: AVX-512/AVX2 enabled, `target-cpu=native`
