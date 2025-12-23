# 📊 VelesDB Performance Benchmarks

*Last updated: December 2025*

This document details the performance benchmarks for VelesDB v0.2.0. Tests were conducted on a standard workstation (8-core CPU, AVX2 support).

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
| **HNSW HighRecall** | 99.8% | Large datasets, near-exact results |
| **HNSW Balanced** | 98.7% | Best performance/quality tradeoff |
| **HNSW Fast** | 91.1% | Maximum speed, acceptable quality |

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
| **Cosine Similarity** | Latency | **~76 ns** | ~13M ops/sec | **4.2x** |
| **Euclidean Distance** | Latency | **~47 ns** | ~21M ops/sec | **2.9x** |
| **Dot Product** | Latency | **~45 ns** | ~22M ops/sec | **2.9x** |
| **Hamming (Binary)** | Latency | **~6 ns** | ~164M ops/sec | **~34x** (vs f32) |
| **ColumnStore Filter** | Eq String (100k) | **~27 µs** | ~3.7M items/sec | **122x** vs JSON |
| **VelesQL Parser** | Simple | **~528 ns** | ~1.9M qps | - |

> **Note**: All distance functions now use explicit SIMD via the `wide` crate (f32x8). The ColumnStore provides columnar filtering that is 44-122x faster than JSON-based filtering.

---

## ⚡ SIMD Vector Operations

Comparison between standard Rust iterators (Baseline) and VelesDB's explicit SIMD optimizations (Optimized).
Results are from `search_benchmark` which measures the full public API call overhead.

### Cosine Similarity (768 dimensions)
> Used for semantic search and text embeddings.

| Implementation | Time per op | Throughput |
|----------------|-------------|------------|
| Baseline (Auto-vec) | 320 ns | 3.1M ops/s |
| **VelesDB Optimized (SIMD)** | **76 ns** | **13M ops/s** |
| **Improvement** | **-76% latency** | **4.2x throughput** |

### Euclidean Distance (768 dimensions)
> Used for spatial data and image features.

| Implementation | Time per op | Throughput |
|----------------|-------------|------------|
| Baseline (Auto-vec) | 138 ns | 7.2M ops/s |
| **VelesDB Optimized (SIMD)** | **47 ns** | **21M ops/s** |
| **Improvement** | **-66% latency** | **2.9x throughput** |

### Dot Product (768 dimensions)
> Used for raw similarity and inner product.

| Implementation | Time per op | Throughput |
|----------------|-------------|------------|
| Baseline (Auto-vec) | 130 ns | 7.7M ops/s |
| **VelesDB Optimized (SIMD)** | **45 ns** | **22M ops/s** |
| **Improvement** | **-65% latency** | **2.9x throughput** |

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

---

## 📈 Search Quality (Recall@k)

VelesDB provides configurable recall/latency tradeoffs through search quality profiles.

### HNSW Recall by Quality Profile

Measured on 10,000 vectors (128 dimensions) with cosine similarity.
Index built with `HnswParams::max_recall()` (M=32, ef_construction=500):

| Quality Profile | ef_search | Recall@10 | Latency (k=10) |
|-----------------|-----------|-----------|----------------|
| **Fast** | 64 | **91.1%** | ~3.5ms |
| **Balanced** | 128 | **98.7%** | ~7.5ms |
| **Accurate** | 256 | **99.8%** | ~11ms |
| **HighRecall** | 512 | **99.8%** | ~25ms |

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
| 100 | ~180ms | ~25ms | **7.2x** |
| 1000 | ~1.8s | ~250ms | **7.2x** |

### Brute-Force Parallel Search

Exact search with 100% recall, parallelized across cores:

| Dataset | 1 Thread | 4 Threads | 8 Threads | Scaling |
|---------|----------|-----------|-----------|---------|
| 10,000 | ~5ms | ~1.3ms | ~0.7ms | ~7x |
| 50,000 | ~25ms | ~6.5ms | ~3.5ms | ~7x |
| 100,000 | ~50ms | ~13ms | ~7ms | ~7x |

> **Note**: Scaling efficiency depends on memory bandwidth and CPU cache hierarchy. NUMA systems may see reduced scaling on cross-socket access.

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

## 🧪 Methodology

- **Hardware**: Windows Workstation, 8-core CPU
- **Environment**: Rust 1.83, Release build (`--release`)
- **Framework**: Criterion.rs
- **Optimizations**: AVX2 enabled, `target-cpu=native`
