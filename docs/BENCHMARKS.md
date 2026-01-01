# 📊 VelesDB Performance Benchmarks

*Last updated: January 1, 2026 (v0.7.2)*

---

## 🚀 v0.7.2 Headline

| Metric | Baseline | VelesDB | Winner |
|--------|----------|---------|--------|
| **SIMD Dot Product (768D)** | 280ns (Naive) | **35ns** | **VelesDB 8x** ✅ |
| **Search (10K)** | ~50ms (pgvector) | **~105µs** | **VelesDB 476x** ✅ |
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
| **Dot Product** | 35ns | 28M/s | 8x |
| **Euclidean** | 44ns | 22M/s | 6x |
| **Cosine** | 82ns | 12M/s | 3.4x |
| **Hamming** | 6ns | 164M/s | 34x |

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

| Profile | Recall@10 | Latency (10K) | Method |
|---------|-----------|---------------|--------|
| Fast | 90.6% | ~7ms | HNSW ef=64 |
| **Balanced** | 98.2% | ~12ms | HNSW ef=128 |
| Accurate | 99.3% | ~18ms | HNSW ef=256 |
| HighRecall | 99.8% | ~37ms | HNSW ef=1024 |
| **Perfect** | **100%** | ~55ms | **Brute-force SIMD** |

---

## 🚀 Parallel Performance

| Operation | Speedup (8 cores) |
|-----------|------------------|
| Batch Search | **19x** |
| Batch Insert | **18x** |

---

## 🔥 v0.7.2 Optimizations

- **32-wide SIMD unrolling** — 4x f32x8 accumulators for maximum ILP
- **Pre-normalized functions** — `cosine_similarity_normalized()` ~40% faster
- **SIMD-accelerated HNSW** — AVX2/SSE via `simdeez_f`
- **Parallel insertion** — Rayon-based graph construction
- **CPU prefetch hints** — L2 cache warming
- **Batch WAL writes** — Single disk write per import

---

## 🧪 Methodology

- **Hardware**: 8-core CPU, 32GB RAM
- **Environment**: Rust 1.83, `--release`, `target-cpu=native`
- **Framework**: Criterion.rs
