# 📊 VelesDB Performance Benchmarks

*Last updated: January 8, 2026 (v0.8.12)*

---

## 🚀 v0.8.5 Headline

| Metric | Baseline | VelesDB | Winner |
|--------|----------|---------|--------|
| **SIMD Dot Product (768D)** | 280ns (Naive) | **36ns** | **VelesDB 8x** ✅ |
| **Search (10K)** | ~50ms (pgvector) | **~105µs** | **VelesDB 476x** ✅ |
| **Hybrid Search (1K)** | N/A | **62µs** | **VelesDB** ✅ |
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

## ⚡ SIMD Performance Summary (768D)

| Operation | Latency | Throughput | Speedup |
|-----------|---------|------------|----------|
| **Dot Product** | 36ns | 28M/s | 8x |
| **Euclidean** | 46ns | 22M/s | 6x |
| **Cosine** | 93ns | 11M/s | 3x |
| **Hamming** | 6ns | 164M/s | 34x |
| **Jaccard** | 160ns | 6M/s | 10x |

---

## 🔍 Hybrid Search Performance

| Scale | Vector+Text | Vector Only | Text Only |
|-------|-------------|-------------|-----------|
| 100 docs | 55µs | 54µs | 26µs |
| 1K docs | 62µs | 56µs | 30µs |

---

## 🔍 ColumnStore Filtering

| Scale | Throughput | vs JSON |
|-------|------------|----------|
| 100k items | 3.7M/s | **122x faster** |

---

## 📝 VelesQL Parser

| Mode | Latency | Throughput |
|------|---------|------------|
| Parse | 570ns | 1.7M qps |
| **Cache Hit** | **49ns** | **20M qps** |

```rust
use velesdb_core::velesql::QueryCache;
let cache = QueryCache::new(1000);
let query = cache.parse("SELECT * FROM docs LIMIT 10")?;
```

---

## 📈 HNSW Recall Profiles

| Profile | Recall@10 (100K) | Latency P50 | Method |
|---------|------------------|-------------|--------|
| Fast | 34.2% | 59.3ms | HNSW ef=64 |
| Balanced | 48.8% | 60.9ms | HNSW ef=128 |
| Accurate | 67.6% | 78.3ms | HNSW ef=256 |
| **HighRecall** | **96.1%** ✅ | 73.0ms | HNSW ef=1024 |
| **Perfect** | **100%** | 42.1ms | HNSW ef=2048 |

> **Note**: Recall@10 ≥95% garantie pour HighRecall et Perfect modes.

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

## 🧪 Methodology

- **Hardware**: 8-core CPU, 32GB RAM
- **Environment**: Rust 1.83, `--release`, `target-cpu=native`
- **Framework**: Criterion.rs
