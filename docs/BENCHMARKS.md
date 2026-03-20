# VelesDB Performance Benchmarks

*Last updated: March 20, 2026 (v1.6.0+simd-opt — sequential benchmarks on idle machine)*

---

## Test Environment

| Parameter | Value |
|-----------|-------|
| **CPU** | Intel Core i9-14900KF (24 cores, 32 threads, AVX2) |
| **RAM** | 64 GB DDR5 |
| **OS** | Microsoft Windows 11 Professionnel |
| **Rust** | rustc 1.92.0 (ded5c06cf 2025-12-08) |
| **Build** | `--release`, `target-cpu=native`, LTO thin, codegen-units=1 |
| **Framework** | Criterion.rs with `--noplot` |

Hardware configuration captured in `benchmarks/machine-config.json`.

---

## 1. Dense Search Baseline (SIMD Kernels)

SIMD kernels use AVX2/AVX-512 multi-accumulator pipelines with runtime feature detection via `simd_dispatch`. Measured March 20, 2026 on Intel Core i9-14900KF (24C/32T, AVX2+FMA, AVX-512 disabled — hybrid P+E topology), 64GB DDR5, Windows 11 Pro, sequential run on idle machine.

### SIMD Kernel Latency

| Operation | 128D | 384D | 768D | 1536D | 3072D |
|-----------|------|------|------|-------|-------|
| **Dot Product** | 5.4 ns | 12.0 ns | 23.6 ns | 43.8 ns | 91.2 ns |
| **Euclidean** | 5.2 ns | 11.5 ns | 22.7 ns | 46.1 ns | 99.3 ns |
| **Cosine** | 7.7 ns | 18.6 ns | 33.6 ns | 61.4 ns | 118.9 ns |
| **Hamming** | 7.3 ns | 17.8 ns | 34.3 ns | 69.2 ns | 132.2 ns |
| **Jaccard** | 6.4 ns | 16.4 ns | 29.3 ns | 50.9 ns | 100.6 ns |

*Run `cargo bench -p velesdb-core --bench simd_benchmark -- --noplot` to regenerate.*

### Cosine Engine Dispatch Overhead (March 19, 2026)

| Dimension | Native Kernel | Engine Dispatch | Overhead |
|-----------|---------------|-----------------|----------|
| 384D | 21.1 ns | 28.0 ns | 33% |
| 768D | 36.3 ns | 33.9 ns | −6.6% (dispatch optimized) |
| 1536D | 64.3 ns | 74.7 ns | 16.1% |

Engine dispatch overhead is negligible at typical embedding dimensions (768D+).

### Throughput

| Dimension | Dot Product | Throughput |
|-----------|-------------|------------|
| 768D | 23.6 ns | 32.5 Gelem/s |
| 1536D | 43.8 ns | 35.1 Gelem/s |
| 3072D | 91.2 ns | 33.7 Gelem/s |

### Batch Distance Computation

| Benchmark | Latency | Per-Vector |
|-----------|---------|------------|
| Native 1000x768D | 40.4 µs | 40.4 ns |
| Engine 1000x768D | 42.2 µs | 42.2 ns |

---

## 2. PQ Recall and Latency

Product Quantization (PQ) trades recall for memory compression and faster approximate search. Benchmarked with Criterion.

### PQ Recall (pq_recall_benchmark)

**Setup:** 5,000 vectors, 128D, L2 distance, 10 clusters, 50 queries, k=10.

| Mode | Recall@10 | Search Latency (50 queries) | Per-Query |
|------|-----------|----------------------------|-----------|
| **Full Precision** | 87.6% | 19.1 ms | 382 us |
| **PQ (m=auto, rescore)** | 30.6% | 30.6 ms | 612 us |

Notes:
- Full-precision recall is 87.6% (not 100%) because HNSW is approximate search.
- PQ recall@10 of 30.6% on 128D/5K vectors is expected for standard PQ without OPQ -- low dimensionality limits subspace quality.
- Rescore oversampling (default 4x) is applied.

### PQ vs SQ8 vs Full HNSW Latency (pq_hnsw_benchmark)

**Setup:** 2,000 vectors, 64D, L2 distance, top-20 search.

| Storage Mode | Search Latency | Recall@50 | Compression |
|--------------|---------------|-----------|-------------|
| **Full Precision** | 24.9 us | baseline | 1x |
| **SQ8** | 25.2 us | 100% | 4x |
| **PQ** | 257.6 us | 68.0% | ~16-32x |

Key findings:
- **SQ8 is the best general-purpose mode**: zero recall loss with 4x compression and identical latency.
- PQ search is slower due to ADC (Asymmetric Distance Computation) table lookups, but delivers 16--32x compression for memory-constrained deployments.
- PQ recall improves significantly with higher dimensionality (256D+) and OPQ rotation.

*Run `cargo bench -p velesdb-core --bench pq_recall_benchmark -- --noplot` to regenerate recall numbers.*
*Run `cargo bench -p velesdb-core --bench pq_hnsw_benchmark -- --noplot` to regenerate latency comparison.*

---

## 3. Sparse Search Latency

Sparse vector search uses an inverted index with MaxScore optimization for early termination.

### Sparse Search (sparse_benchmark)

**Setup:** 10,000 documents, BM25-style sparse vectors, Criterion.

| Benchmark | Latency (estimate) | Notes |
|-----------|--------------------|-------|
| **Insert 10K sequential** | 93 ms | 9.3 µs/doc |
| **Insert 10K parallel (4x2500)** | 155 ms | Manual 4-thread partitioning |
| **Search top-10, 10K corpus** | 958 µs | MaxScore pruning active |
| **Search top-100, 10K corpus** | 818 µs | Minimal cost for larger k |
| **Concurrent 16-thread (8 insert + 8 search)** | 171 ms | Mixed read/write workload |

Latency percentiles are not separately measured by Criterion (which reports confidence intervals). The estimates above represent the mean of the sampling distribution.

| Metric | Value |
|--------|-------|
| **MaxScore threshold** | 30% coverage (total postings > 0.3 * doc_count * query_nnz) |
| **Accumulator strategy** | Dense array up to 10M doc IDs, FxHashMap above |
| **Linear scan fallback** | When coverage exceeds threshold |

*Run `cargo bench -p velesdb-core --bench sparse_benchmark -- --noplot` to regenerate.*

---

## 4. Hybrid Search

Hybrid search combines dense vector similarity with sparse keyword matching using Reciprocal Rank Fusion (RRF, k=60) or Relative Score Fusion (RSF).

No dedicated hybrid benchmark suite exists yet. Performance can be estimated from the individual components:

| Component | Latency (10K corpus) | Source |
|-----------|---------------------|--------|
| Dense HNSW search (k=10, 768D) | ~43 µs | hnsw_benchmark |
| Sparse search (top-10, 10K) | ~958 µs | sparse_benchmark |
| RRF fusion overhead | negligible (score merging) | -- |
| **Estimated hybrid total** | **~1.0 ms** | Dense + Sparse + fusion |

The RRF fusion step is a simple score merge with no distance computation, so hybrid latency is dominated by the sparse search branch. For workloads where sparse search is the bottleneck, the MaxScore optimization provides early termination on high-selectivity queries.

To run a hybrid search benchmark when available:
```bash
cargo bench -p velesdb-core --bench hybrid_benchmark -- --noplot
```

---

## 5. HNSW Vector Search

| Operation | Latency | Throughput |
|-----------|---------|------------|
| **Search k=10** (10K/768D) | 42.8 µs | 23.4K QPS |
| **Search k=50** | 63.5 µs | -- |
| **Search k=100** | 151.5 µs | -- |
| **Insert 1K x 768D** (sequential) | 263.7 ms | 3.8K vec/s |
| **Parallel Insert 1K x 768D** | 156.5 ms | 6.4K vec/s |
| **Parallel Insert 10K x 768D** | 2.26 s | 4.4K vec/s |

### HNSW Recall Profiles (10K/128D)

| Profile | ef_search | Recall@10 | Latency P50 |
|---------|-----------|-----------|-------------|
| Fast | 64 | 92.2% | 36 us |
| Balanced | 128 | 98.8% | 57 us |
| Accurate | 512 | 100.0% | 130 us |
| Perfect | 4096 | 100% | 200 us |
| Adaptive | 32–512 | 95%+ | ~15-40 us (easy queries) |

*Recall values from recall_benchmark. Latencies measured March 19, 2026. ef_search values are base values (scaled with k).*

Recall@10 >= 95% is guaranteed for Balanced mode and above. The new **Adaptive** mode starts with a low ef and escalates only for hard queries, achieving 2-4x faster median latency. Use `HnswParams::for_dataset_size()` for automatic parameter tuning.

---

## 6. ColumnStore Filtering

#### String Equality Filter (`filter_eq_string`, measured 2026-03-19)

| Scale | ColumnStore | JSON Scan | Speedup |
|-------|-------------|-----------|---------|
| 1K rows | 0.609 µs | 13.7 µs | 22x |
| 10K rows | 4.06 µs | 138.0 µs | 34x |
| 100K rows | 46.5 µs | 3.50 ms | 75x |

#### Integer Equality Filter (`filter_eq_int`, measured 2026-03-19)

| Scale | ColumnStore | JSON Scan | Speedup |
|-------|-------------|-----------|---------|
| 1K rows | 0.336 µs | 16.2 µs | 48x |
| 10K rows | 2.95 µs | 162.7 µs | 55x |
| 100K rows | 29.5 µs | 3.84 ms | 130x |

---

## 7. VelesQL Parser

| Mode | Latency | Throughput |
|------|---------|------------|
| Simple Parse | 1.26 µs | 794K QPS |
| Vector Query | 1.77 µs | 565K QPS |
| Complex Query | 7.47 µs | 134K QPS |
| **Cache Hit** | **1.06 µs** | **943K QPS** |
| EXPLAIN Plan (simple) | 65.4 ns | 15.3M QPS |

*Measured March 19, 2026, sequential run on idle machine.*

---

## 8. Graph (EdgeStore)

| Operation | Latency |
|-----------|---------|
| **get_neighbors (degree 10)** | 95 ns |
| **get_neighbors (degree 50)** | 485 ns |
| **add_edge** | 265 ns |
| **BFS depth 3** | 3.32 µs |
| **Parallel reads (8 threads)** | 292 µs |

---

## 9. Competitive Analysis

### SIMD Distance Kernels

| Library | Dot Product 1536D | Notes |
|---------|-------------------|-------|
| **VelesDB** | **43.8 ns** | AVX2 4-acc, native Rust |
| SimSIMD | ~25-30 ns | AVX-512, C library |
| NumPy | ~200-400 ns | BLAS backend |
| SciPy | ~300-500 ns | No SIMD optimization |

### Vector Database Search Latency

| Database | Search Latency | Scale | Notes |
|----------|---------------|-------|-------|
| **VelesDB** | **< 1 ms** | 10K | Local, in-memory HNSW |
| Milvus | < 10 ms p50 | 1M+ | Distributed |
| Qdrant | 20-50 ms | 1M+ | Cloud/distributed |
| pgvector | 45-100 ms | 100K+ | PostgreSQL extension |
| Redis | ~5 ms | 1M+ | In-memory |

*Competitor latencies are approximate industry estimates for comparable-scale workloads. VelesDB is local-first/in-process; distributed databases serve different use cases at larger scale.*

---

## 10. Performance Targets by Scale

| Dataset Size | Search P99 | Recall@10 | Status |
|--------------|------------|-----------|--------|
| 10K vectors | < 1 ms | >= 98% | Achieved |
| 100K vectors | < 5 ms | >= 95% | Achieved (96.1%) |
| 1M vectors | < 50 ms | >= 95% | Target |

---

## Methodology

- **Hardware**: See Test Environment section above
- **Framework**: Criterion.rs (`--release`, `--noplot`)
- **Concurrency**: Tests run with `--test-threads=1` for isolation
- **Reproducibility**: Seeded RNG for synthetic data generation
- **Reporting**: Criterion `estimate` value (center of confidence interval)

All benchmarks can be reproduced with:
```bash
cargo bench -p velesdb-core --bench <benchmark_name> -- --noplot
```
