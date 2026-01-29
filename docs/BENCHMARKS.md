# 📊 VelesDB Performance Benchmarks

*Last updated: January 28, 2026 (v1.4.0)*

---

## 🚀 v1.2.0 Headline

| Metric | Baseline | VelesDB | Winner |
|--------|----------|---------|--------|
| **SIMD Dot Product (1536D)** | 280ns (Naive) | **110ns** | **VelesDB 2.5x** ✅ |
| **HNSW Search (10K/768D)** | ~50ms (pgvector) | **57µs** | **VelesDB 877x** ✅ |
| **ColumnStore Filter (100K)** | 3.9ms (JSON) | **88µs** | **VelesDB 44x** ✅ |
| **VelesQL Parse** | N/A | **84ns** (cache) | **VelesDB** ✅ |
| **Recall@10** | 100% | **100%** | **VelesDB Perfect** ✅ |

### When to Choose VelesDB

- ✅ **Ultra-low latency** — Microsecond-level search on local datasets
- ✅ **Embedded/Desktop** — Native Rust integration with zero network overhead
- ✅ **On-Prem/Edge** — Single binary, no dependencies
- ✅ **WASM/Browser** — Client-side vector search capability

### When to Choose pgvector

- ✅ Existing PostgreSQL infrastructure
- ✅ Need 100% recall

---

## ⚡ SIMD Performance Summary

| Operation | 384D | 768D | 1536D |
|-----------|------|------|-------|
| **Dot Product** | 31ns | 57ns | 110ns |
| **Euclidean** | 35ns | 66ns | 126ns |
| **Cosine** | 36ns | 68ns | 131ns |
| **Hamming (u64)** | 6ns | 6ns | 11ns |
| **Jaccard** | 80ns | 154ns | 306ns |

---

## 🔍 HNSW Vector Search

| Operation | Latency | Throughput |
|-----------|---------|------------|
| **Search k=10** | 57µs | 9.2K qps |
| **Search k=50** | 90µs | - |
| **Search k=100** | 174µs | - |
| **Insert 1K×768D** | 696ms | 1.4K elem/s |

---

## 🔍 ColumnStore Filtering

| Scale | ColumnStore | JSON | Speedup |
|-------|-------------|------|---------|
| 10K rows | 8.6µs | 397µs | **46x** |
| 100K rows | 88µs | 3.9ms | **44x** |
| 500K rows | 136µs | 18.6ms | **137x** |

---

## 📝 VelesQL Parser

| Mode | Latency | Throughput |
|------|---------|------------|
| Simple Parse | 1.4µs | 707K qps |
| Vector Query | 2.0µs | 490K qps |
| Complex Query | 7.9µs | 122K qps |
| **Cache Hit** | **84ns** | **12M qps** |
| EXPLAIN Plan | 61ns | 16M qps |

```rust
use velesdb_core::velesql::QueryCache;
let cache = QueryCache::new(1000);
let query = cache.parse("SELECT * FROM docs LIMIT 10")?;
```

---

## 📈 HNSW Recall Profiles (10K/128D)

| Profile | Recall@10 | Latency P50 | Change vs v1.0 |
|---------|-----------|-------------|----------------|
| Fast (ef=64) | 92.2% | **36µs** | 🆕 new |
| Balanced (ef=128) | 98.8% | **57µs** | 🚀 **-80%** |
| Accurate (ef=256) | 100.0% | **130µs** | 🚀 **-72%** |
| **Perfect (ef=2048)** | **100%** | **200µs** | 🚀 **-92%** |

> **Note**: Recall@10 ≥95% guaranteed for Balanced mode and above.
> 
> **v1.1.0 Performance Gains**: EPIC-CORE-003 optimizations (LRU Cache, Trigram Index, Lock-free structures) delivered **72-92% latency improvements** across all modes.

### ⚠️ Benchmark Interpretation Note

**Criterion benchmarks** measure **batch execution time** (100 queries total). To get **per-query latency**, divide by 100:

| Mode | Criterion Output | Per-Query Latency | Calculation |
|------|-----------------|-------------------|-------------|
| Fast | 3.6ms | **36µs** | 3.6ms ÷ 100 |
| Balanced | 5.7ms | **57µs** | 5.7ms ÷ 100 |
| Accurate | 13ms | **130µs** | 13ms ÷ 100 |
| Perfect | 20ms | **200µs** | 20ms ÷ 100 |

When comparing with other vector databases or previous VelesDB versions, always use **per-query latency** for accurate comparison.

---

## 🚀 Parallel Performance

| Operation | Speedup (8 cores) |
|-----------|------------------|
| Batch Search | **19x** |
| Batch Insert | **18x** |

---

## 🎯 Performance Targets by Scale

| Dataset Size | Search P99 | Recall@10 | Status |
|--------------|------------|-----------|--------|
| 10K vectors | **<1ms** | ≥98% | ✅ Achieved |
| 100K vectors | **<5ms** | ≥95% | ✅ Achieved (96.1%) |
| 1M vectors | **<50ms** | ≥95% | 🎯 Target |

> Use `HnswParams::for_dataset_size()` for automatic parameter tuning.

---

## 🆕 v0.8.12 Native HNSW Implementation

VelesDB now includes a **custom Native HNSW implementation** based on 2024-2026 research papers (Flash Method, VSAG Framework).

### Native vs hnsw_rs Comparison

*Benchmarked January 8, 2026 — 5,000 vectors, 128D, Euclidean distance*

| Operation | Native HNSW | hnsw_rs | Improvement |
|-----------|-------------|---------|-------------|
| **Search (100 queries)** | 26.9 ms | 32.4 ms | **1.2x faster** ✅ |
| **Parallel Insert (5k)** | 1.47 s | 1.57 s | **1.07x faster** ✅ |
| **Recall** | ~99% | baseline | Parity ✓ |

### Why Native HNSW?

- **No external dependency** — Full control over graph construction and search
- **SIMD-optimized distances** — Custom AVX2/SSE implementations
- **Lock-free reads** — Concurrent search without blocking
- **Future-ready** — Foundation for int8 quantized graph traversal

```bash
# Enable Native HNSW
cargo build --features native-hnsw

# Run comparison benchmark
cargo bench --bench hnsw_comparison_benchmark
```

📖 Full guide: [docs/reference/NATIVE_HNSW.md](reference/NATIVE_HNSW.md)

---

## 🔥 v0.8.5 Optimizations

- **Unified VelesQL execution** — `Collection::execute_query()` for all components
- **Batch search with filters** — Individual filters per query in batch operations
- **Buffer reuse** — Thread-local buffer for brute-force search (~40% allocation reduction)
- **Adaptive HNSW params** — `for_dataset_size()` and `million_scale()` APIs
- **32-wide SIMD unrolling** — 4x f32x8 accumulators for maximum ILP
- **Pre-normalized functions** — `cosine_similarity_normalized()` ~40% faster
- **SIMD-accelerated HNSW** — AVX2/SSE via `wide` crate
- **Parallel insertion** — Rayon-based graph construction
- **CPU prefetch hints** — L2 cache warming
- **GPU acceleration** — [Roadmap](GPU_ACCELERATION_ROADMAP.md) for batch operations

---

## 🔗 Graph (EdgeStore)

| Operation | Latency |
|-----------|---------|
| **get_neighbors (degree 10)** | 155ns |
| **get_neighbors (degree 50)** | 508ns |
| **add_edge** | 278ns |
| **BFS depth 3** | 3.6µs |
| **Parallel reads (8 threads)** | 346µs |

---

## 🧪 Methodology

- **Hardware**: 8-core CPU, 32GB RAM
- **Environment**: Rust 1.85, `--release`, `target-cpu=native`
- **Framework**: Criterion.rs
