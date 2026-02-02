# 📊 VelesDB Performance Benchmarks

*Last updated: February 1, 2026 (v1.4.1 - SIMD Tiered Dispatch EPIC-052/077)*

---

## 🚀 SIMD Performance Results (Post-EPIC-052)

### Hardware Configuration
- **CPU**: Intel Core i9-14900K (24 cores, 32 threads, AVX2 native)
- **RAM**: 64GB DDR5
- **GPU**: NVIDIA RTX 4090 (for GPU benchmarks)
- **OS**: Windows 11 (Power Mode: "Performances élevées")
- **Rust**: 1.85, `--release`, `target-cpu=native`
- **Tests**: 2411 passing, 82.30% coverage

### SIMD Kernel Benchmarks (LTO thin, codegen-units=1)

| Operation | 128D | 384D | **768D** | **1536D** | **3072D** |
|-----------|------|------|----------|-----------|-----------|
| **dot_product** | 4.05ns | 9.71ns | **18.68ns** | **32.91ns** | **70.73ns** |
| **euclidean** | 8.59ns | 11.56ns | **20.88ns** | 43.80ns | 81.69ns |
| **cosine** | 7.87ns | 19.67ns | **37.26ns** | 58.09ns | 110.13ns |
| **hamming** | 6.25ns | 9.78ns | **18.99ns** | 38.35ns | 82.01ns |
| **jaccard** | 5.00ns | 11.61ns | **22.81ns** | 47.72ns | 93.63ns |

### 📈 Throughput Analysis

| Dimension | Dot Product | Throughput |
|-----------|-------------|------------|
| 768D | 18.68ns | **41.1 Gelem/s** |
| 1536D | 32.91ns | **46.6 Gelem/s** |
| 3072D | 70.73ns | **43.4 Gelem/s** |

### 🎯 Key Achievements

#### ✅ Major Performance Gains (EPIC-052/077)
- **Dot Product**: 18.5ns @ 768D → **41.6 Gelem/s**
- **Cosine tiered dispatch**: 2-acc (64-1023D) + 4-acc (>1024D) pour éviter register pressure
- **Jaccard**: 22.8ns @ 768D (avant 28.1ns)
- **Hamming**: 19.0ns @ 768D (avant 36.2ns)

---

## 🔄 HNSW Insert Performance

| Operation | Vectors | Time | Throughput |
|-----------|---------|------|------------|
| **Sequential Insert** | 1,000 × 768D | 614ms | **1,628 vec/s** |
| **Parallel Insert** | 1,000 × 768D | 443ms | **2,259 vec/s** |

**Parallel insert** provides **38% speedup** over sequential.

---

## 🌐 Competitive Analysis (State of the Art 2025)

### SIMD Distance Kernels

| Library | Dot Product 1536D | Notes |
|---------|-------------------|-------|
| **VelesDB** | **32ns** | AVX2 4-acc, native Rust |
| SimSIMD | ~25-30ns | AVX-512, C library |
| NumPy | ~200-400ns | BLAS backend |
| SciPy | ~300-500ns | No SIMD optimization |

**VelesDB** is **competitive with SimSIMD** and **10-15x faster than NumPy/SciPy**.

### Vector Database Search Latency

| Database | Search Latency | Scale | Notes |
|----------|---------------|-------|-------|
| **VelesDB** | **< 1ms** | 10K | Local, in-memory HNSW |
| Milvus | < 10ms p50 | 1M+ | Distributed |
| Qdrant | 20-50ms | 1M+ | Cloud/distributed |
| pgvector | 45-100ms | 100K+ | PostgreSQL extension |
| Redis | ~5ms | 1M+ | In-memory |

**VelesDB excels for local/embedded use cases** with sub-millisecond latency.

### Insert Throughput

| Database | Insert Rate | Notes |
|----------|-------------|-------|
| **VelesDB** | **2,259 vec/s** | Single machine, parallel |
| Milvus | Highest indexing | Distributed, batch |
| Qdrant | ~1,000 vec/s | Single node |

---

## 🎯 VelesDB Positioning

### ✅ Where VelesDB Excels
1. **Local-first / Edge**: Sub-ms latency, no network overhead
2. **Embedded**: 15MB binary, zero dependencies
3. **SIMD Performance**: Competitive with state-of-the-art
4. **Privacy**: Data never leaves device

### 📈 Optimization Opportunities
1. **Batch Insert**: Implement batch indexing for higher throughput
2. **AVX-512**: Enable on supported hardware (i9-14900K has AVX2 only)
3. **Quantization**: int8/int4 vectors for memory efficiency
4. **GPU Acceleration**: CUDA/WebGPU for large-scale search

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

## ⚡ SIMD Performance Summary (i9-14900K AVX2 4-acc)

| Operation | 384D | 768D | 1536D | vs v1.4.0 |
|-----------|------|------|-------|-----------|
| **Dot Product** | **9.7ns** | **18.7ns** | **32.9ns** | **Baseline** |
| **Euclidean** | 13.4ns | 20.9ns | 43.8ns | **Improved** |
| **Cosine** | 19.7ns | 37.3ns | 58.1ns | **-13%** ✅ |

### Stratégie Adaptative (EPIC-PERF-003) - Optimisée Feb 2026

Le dispatch s'adapte automatiquement au CPU détecté avec des seuils optimisés basés sur la recherche state-of-the-art:

| CPU Détecté | Implémentation | Seuils | Gain typique |
|-------------|----------------|--------|--------------|
| **AVX-512** (Xeon, serveurs) | 512-bit 4-acc | >= 512 éléments | 15-25% |
| **AVX2** (Core 12th/13th/14th gen, Ryzen) | 256-bit 4-acc | >= 256 | 15-37% |
| **AVX2** | 256-bit 2-acc | 64-255 | Baseline |
| **AVX2 petits vecteurs** | 256-bit 1-acc | **16-63** | **Meilleur ratio overhead/perf** |
| **AVX2 tiny** | Scalar | **< 16** | Évite overhead SIMD |
| **ARM NEON** | 128-bit 1-acc | >= 4 | Baseline |

**Optimisations implémentées:**
- **Tail unrolling**: Remainder déroulé (4→2→1 éléments) pour éviter les boucles
- **Warmup AVX-512**: 3 itérations avant mesure pour stabiliser la fréquence CPU
- **Dispatch optimisé**: Scalar < 16 éléments (évite overhead SIMD setup)

### EPIC-073 SIMD Pipeline Optimizations

| Feature | Description | Performance |
|---------|-------------|-------------|
| **Multi-level Prefetch** | L1/L2/L3 cache hints | 10-30% cold cache improvement |
| **Jaccard 4-way ILP** | Instruction-level parallelism | **2.3x** faster than baseline |
| **Binary Jaccard POPCNT** | Hardware popcount | **10x** faster for u64 packed |
| **Batch Dot Product** | M×N matrix computation | Amortized overhead |
| **Batch Top-K** | Multi-query similarity | Cache reuse optimization |

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
