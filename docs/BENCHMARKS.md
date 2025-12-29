# 📊 VelesDB Performance Benchmarks

*Last updated: December 29, 2025 (v0.4.0)*

This document details the performance benchmarks for VelesDB. Tests were conducted on a standard workstation (8-core CPU, AVX2/AVX-512 support).

---

## 🆚 Competitor Comparison: VelesDB vs pgvectorscale

We benchmarked VelesDB against [pgvectorscale](https://github.com/timescale/pgvectorscale) (Timescale's DiskANN extension for PostgreSQL).

### Test Configuration

| Parameter | Value |
|-----------|-------|
| **Dataset** | 10,000 vectors |
| **Dimensions** | 768 (BERT-sized) |
| **Queries** | 100 random queries |
| **Top-K** | 10 results |
| **Metric** | Cosine similarity |
| **VelesDB** | Native Python SDK (PyO3), `upsert_bulk()` |
| **pgvectorscale** | Docker (`timescale/timescaledb-ha:pg16-oss`), DiskANN index |

### Results

| Metric | pgvectorscale | VelesDB | Speedup |
|--------|---------------|---------|---------|
| **Total Ingest** | 22.3s | **3.0s** | **7.4x faster** |
| **Avg Search Latency** | 52.8ms | **4.0ms** | **13x faster** |
| **P50 Latency** | 50.1ms | **3.5ms** | **14x faster** |
| **P95 Latency** | 61.9ms | **5.0ms** | **12x faster** |
| **P99 Latency** | 60.2ms | **5.0ms** | **12x faster** |
| **Throughput** | 18.9 QPS | **246.8 QPS** | **13x higher** |

### Key Findings

1. **Ingestion**: VelesDB's parallel HNSW insertion (`upsert_bulk()`) + single flush outperforms pgvectorscale's sequential INSERT + separate DiskANN index build.

2. **Search Latency**: VelesDB's SIMD-optimized distance calculations and in-memory HNSW graph deliver sub-5ms latency vs pgvectorscale's 50ms+ (which includes PostgreSQL query overhead).

3. **Architecture Difference**:
   - **pgvectorscale** = PostgreSQL overhead + DiskANN (disk-resident, optimized for 50M+ vectors)
   - **VelesDB** = Purpose-built engine + HNSW (memory-mapped, optimized for sub-ms latency)

### How to Reproduce

```bash
cd benchmarks/
docker-compose up -d  # Start pgvectorscale
pip install -r requirements.txt
python benchmark.py --vectors 10000 --dim 768
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

### Query Cache (WIS-85)

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

### Parallel Batch Insert (WIS-9)

Bulk vector insertion using `insert_batch_parallel` vs sequential `insert`:

| Vectors | Batch Parallel | Sequential | Speedup |
|---------|----------------|------------|---------|
| 1,000 | **17 ms** | 200 ms | **12x** |
| 5,000 | **167 ms** | 2.6 s | **16x** |
| 10,000 | **445 ms** | 8.1 s | **18x** |

> **Perf**: WIS-9 refactored `HnswMappings` to reduce lock contention from 4 locks to 2 locks per insert operation. Combined with `hnsw_rs` native parallel insertion, this enables massive speedups for bulk imports.

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
