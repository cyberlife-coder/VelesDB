# 📊 VelesDB Performance Benchmarks

*Last updated: December 30, 2025 (v0.5.1)*

---

## 🚀 v0.5.1 Headline

| Metric | Baseline | VelesDB | Winner |
|--------|----------|---------|--------|
| **SIMD Cosine (768D)** | 280ns (Naive) | **41ns** | **VelesDB 6.8x** ✅ |
| **Search (10K)** | ~50ms (pgvector) | **128µs** | **VelesDB 390x** ✅ |
| **Recall@10** | 100% | 99.4% | Baseline |

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
| **Dot Product** | 41ns | 24M/s | 6.8x |
| **Euclidean** | 49ns | 20M/s | 5.3x |
| **Cosine** | 81ns | 12M/s | 3.3x |
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

| Profile | ef_search | Recall@10 |
|---------|-----------|----------|
| Fast | 64 | 89% |
| **Balanced** | 128 | **98%** |
| Accurate | 256 | 99.4% |

---

## 🚀 Parallel Performance

| Operation | Speedup (8 cores) |
|-----------|------------------|
| Batch Search | **19x** |
| Batch Insert | **18x** |

---

## 🔥 v0.5.1 Optimizations

- **SIMD-accelerated HNSW** — AVX2/SSE via `simdeez_f`
- **Parallel insertion** — Rayon-based graph construction
- **CPU prefetch hints** — L2 cache warming
- **Batch WAL writes** — Single disk write per import

---

## 🧪 Methodology

- **Hardware**: 8-core CPU, 32GB RAM
- **Environment**: Rust 1.83, `--release`, `target-cpu=native`
- **Framework**: Criterion.rs
