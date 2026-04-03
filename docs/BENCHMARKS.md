# VelesDB Performance Benchmarks

*Last updated: April 3, 2026 (v1.11.0 — includes competitive benchmarks vs Qdrant, Memgraph, ClickHouse)*

---

## Test Environment

| Parameter | Value |
|-----------|-------|
| **CPU** | Intel Core i9-14900KF (24 cores, 32 threads, AVX2) |
| **RAM** | 64 GB DDR5 |
| **OS** | Microsoft Windows 11 Professionnel |
| **Rust** | rustc 1.94.1 (e408947bf 2026-03-25) |
| **Build** | `--release`, `target-cpu=native`, LTO thin, codegen-units=1 |
| **Framework** | Criterion.rs with `--noplot` |

Hardware configuration captured in `benchmarks/machine-config.json`.

---

## 1. Dense Search Baseline (SIMD Kernels)

SIMD kernels use AVX2 multi-accumulator pipelines with runtime feature detection via `simd_dispatch`. Measured April 3, 2026 on Intel Core i9-14900KF (24C/32T, AVX2+FMA), 64GB DDR5, Rust 1.94.1, Windows 11 Pro, sequential run on idle machine.

### SIMD Kernel Latency

| Operation | 128D | 384D | 768D | 1536D | 3072D |
|-----------|------|------|------|-------|-------|
| **Dot Product** | 5.4 ns | 10.7 ns | 21.8 ns | 61.6 ns | 94.8 ns |
| **Euclidean** | 5.3 ns | 11.0 ns | 26.0 ns | 50.5 ns | 118.2 ns |
| **Cosine** | 12.5 ns | 21.4 ns | 32.4 ns | 60.9 ns | 123.5 ns |
| **Hamming** | 7.4 ns | 19.1 ns | 35.2 ns | 65.5 ns | 129.4 ns |
| **Jaccard** | 6.8 ns | 17.6 ns | 27.3 ns | 52.3 ns | 113.1 ns |

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
| 768D | 21.8 ns | 35.2 Gelem/s |
| 1536D | 61.6 ns | 24.9 Gelem/s |
| 3072D | 94.8 ns | 32.4 Gelem/s |

### Batch Distance Computation

| Benchmark | Latency | Per-Vector |
|-----------|---------|------------|
| Native 1000x768D | 42.1 µs | 42.1 ns |
| Engine 1000x768D | 42.9 µs | 42.9 ns |

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
| **Search top-10, 10K corpus** | 813 µs | MaxScore pruning active |
| **Search top-100, 10K corpus** | 824 µs | Minimal cost for larger k |
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
| Dense HNSW search (k=10, 768D) | ~47 µs | hnsw_benchmark |
| Sparse search (top-10, 10K) | ~813 µs | sparse_benchmark |
| RRF fusion overhead | negligible (score merging) | -- |
| **Estimated hybrid total** | **~0.86 ms** | Dense + Sparse + fusion |

The RRF fusion step is a simple score merge with no distance computation, so hybrid latency is dominated by the sparse search branch. For workloads where sparse search is the bottleneck, the MaxScore optimization provides early termination on high-selectivity queries.

To run a hybrid search benchmark when available:
```bash
cargo bench -p velesdb-core --bench hybrid_benchmark -- --noplot
```

---

## 5. HNSW Vector Search

| Operation | Latency | Throughput |
|-----------|---------|------------|
| **Search k=10** (10K/768D) | 47.0 µs | 21.3K QPS |
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

### Search Optimization Notes (v1.7.2)

- **Partial sort** — `search_layer` now uses `select_nth_unstable_by` to avoid full-sorting all `ef` candidates when only the top-k are needed. Complexity drops from O(ef log ef) to O(ef + k log k). Benefit is proportional to the ef/k ratio (e.g., ef=128 with k=10 avoids sorting ~92% of candidates).
- **Batch insert fast-path** — Pure-insert workloads (all new IDs) skip the `DashMap::entry()` write lock overhead introduced by v1.7.0 upsert semantics. Read-lock `contains_key()` pre-check routes new IDs to the cheaper `register()` path.

### Upsert Path Optimization (v1.7.2)

Three changes eliminated lock contention in `Collection::upsert()`:

1. **Write-to-read lock** — `HnswIndex::insert` (in `trait_impl.rs`) now acquires `self.inner.read()` instead of `self.inner.write()`. `NativeHnswInner::insert` takes `&self` and manages its own internal synchronization (per-node locks, atomic entry point). The previous write lock serialized all inserts and blocked concurrent searches.
2. **3-phase pipeline** — `upsert_storage_and_index()` (in `crud.rs`) restructured into: (a) batch storage with 1 fsync per store, (b) per-point secondary updates without holding storage locks, (c) batch HNSW insert via `bulk_index_or_defer()`. This replaces per-point `insert_or_defer()` which acquired and released the HNSW lock N times.
3. **Batch I/O** — Vectors and payloads are written via `store_batch()` (1 WAL write + 1 flush each) instead of N individual `store()` calls with N fsyncs.

Local measurement (i9-14900KF, 10K vectors, 384D): upsert throughput ~808 vec/s before, ~16,151 vec/s after. The upsert/bulk ratio dropped from ~19x to ~1x.

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
| **Cache Hit** | **1.08 µs** | **926K QPS** |
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

### Graph Traversal V2 — CSR Snapshot (Issue #491)

*Measured April 2026 with Criterion on the same test environment.*

| Operation | Latency | Notes |
|-----------|---------|-------|
| **BFS CSR 1K nodes (deg 5, depth 3)** | 3.2 µs | Zero-copy slice access |
| **BFS CSR 10K nodes** | 2.8 µs | Cache-optimal CSR layout |
| **BFS CSR 100K nodes** | 2.8 µs | Scales perfectly — same latency |
| **BFS dense 10K (deg 20)** | 4.6 µs | 4x more neighbors, 1.6x slower |
| **Predicate pushdown 1/5 labels** | 290 ns | 12x faster than unfiltered |
| **Predicate pushdown 2/5 labels** | 721 ns | Linear with label count |
| **No filter baseline** | 3.4 µs | Reference |
| **CSR build 1K nodes** | 262 µs | One-shot construction |
| **CSR build 10K nodes** | 5.8 ms | O(N+E) |
| **add_edge (lazy CSR)** | 442 ns/edge | No O(N+E) rebuild per mutation |

### Filtered Search — Bitmap Pre-filter V2 (Issue #487)

| Selectivity | Latency | Strategy |
|-------------|---------|----------|
| **1% (rare)** | 32 µs | Full-scan brute-force |
| **10% (uncommon)** | 65 µs | HNSW + bitmap pre-filter |
| **50% (common)** | 302 µs | Post-filter fallback (>30% threshold) |
| **Unfiltered baseline** | 83 µs | Reference |

### Bulk Insert V2 (Issue #488)

| Operation | Latency | Throughput |
|-----------|---------|------------|
| **upsert_bulk standard** | 77 ms / 10K | 130K vec/s |
| **AsyncIndexBuilder enqueue+drain** | 108 µs / 10K | 90M vec/s (buffer only) |
| **Buffer brute-force search (10K)** | 213 µs | — |

---

## 9. Competitive Analysis (April 3, 2026)

*All measurements via `bench_full_audit.py` and `bench_graph_quick.py` on the same machine.*
*Competitors: Qdrant 1.17.1, Memgraph 3.9.0, ClickHouse 26.3.2.3 (all Docker, localhost).*

### Vector Search — VelesDB vs Qdrant (SIFT1M, 1M × 128D)

| Metric | VelesDB 1.11.0 | Qdrant 1.17.1 | Ratio |
|--------|----------------|---------------|-------|
| **kNN@10 p50** | **348 µs** | 6.8 ms | VelesDB **19.7x faster** |
| **kNN@100 p50** | **1.9 ms** | 6.9 ms | VelesDB **3.6x faster** |
| **Insert 1M** | 19.0K vec/s | ~15.5K vec/s | VelesDB **1.2x faster** |
| **Recall@10** | 0.992 | 0.998 | −0.6% |
| **Recall@100** | 0.995 | 0.996 | −0.1% |

### Graph Traversal — VelesDB vs Memgraph (5K nodes, 55K edges)

| Query | VelesDB p50 | Memgraph p50 | Ratio | Results |
|-------|-------------|-------------|-------|---------|
| **BFS 1-hop** | **2 µs** | 441 µs | VelesDB **189x faster** | 10 = 10 |
| **BFS 2-hop** | **23 µs** | 2.2 ms | VelesDB **97x faster** | 110 = 110 |
| **BFS 3-hop (limit 200)** | **44 µs** | 2.2 ms | VelesDB **50x faster** | 200 = 200 |
| **Multi KNOWS→WORKS_AT** | **27 µs** | 525 µs | VelesDB **19x faster** | 10 = 10 |
| **Edge loading** | 1.03M edges/s | — | — | — |

### Columnar Queries — VelesDB vs ClickHouse (1M rows, ClickBench)

*Measured via `bench_clickbench.py` with metadata-only queries (no forced vector NEAR).*

| Query | VelesDB p50 | ClickHouse p50 | Ratio | Results |
|-------|-------------|---------------|-------|---------|
| **Q37 dashboard (4 predicates)** | **5.3 ms** | 5.4 ms | **Parity** | 100 = 100 |
| **Q38 dashboard + Title** | 5.8 ms | 4.9 ms | CH 1.2x | 100 = 100 |
| **Q41 traffic source** | 5.3 ms | 4.5 ms | CH 1.2x | 100 = 100 |
| **Q21 URL LIKE '%google%'** | 38.6 ms | 8.3 ms | CH 4.6x | 3 vs 98* |
| **Q39 links (IsLink!=0)** | 21.0 ms | 4.8 ms | CH 4.4x | 100 = 100 |
| **Qx mobile (IsMobile=1)** | 15.9 ms | 5.1 ms | CH 3.1x | 100 = 100 |
| **Q20 point lookup** | 4.2 s | 2.6 ms | CH 1616x | 74 = 74 |

*\*Q21: BM25 text index returns fewer results than LIKE pattern match on URL strings.*

### Improvement vs v1.10.0

| Metric | v1.10.0 | v1.11.0 | Change |
|--------|---------|---------|--------|
| vs Qdrant search | 17.7x faster | **19.7x faster** | Improved |
| vs Qdrant insert | **23x slower** | **1.2x faster** | **Reversed** |
| vs Memgraph BFS 1-hop | **100x slower** | **189x faster** | **Reversed** |
| vs Memgraph BFS 3-hop | **25,000x slower** | **50x faster** | **Reversed** |
| vs ClickHouse Q37 | **345x slower** | **Parity** | **Reversed** |
| vs ClickHouse Q39 | **200x slower** | **4.4x slower** | **45x improved** |

### Known Limitations

| Issue | Current | Root Cause | Planned Fix |
|-------|---------|-----------|-------------|
| Q20 point lookup (1616x) | Full scan 1M rows | No hash index for high-cardinality Eq | Hash index on secondary indexes |
| Q21 LIKE result count (3 vs 98) | BM25 tokenization mismatch | URL strings tokenize differently | Trigram index for LIKE patterns |
| Bulk load with many indexes | Slow at 1M+ with >3 indexes | Per-point B-tree insert | Deferred index build (implemented) |

### SIMD Distance Kernels

| Library | Dot Product 1536D | Notes |
|---------|-------------------|-------|
| **VelesDB** | **43.8 ns** | AVX2 4-acc, native Rust |
| SimSIMD | ~25-30 ns | AVX-512, C library |
| NumPy | ~200-400 ns | BLAS backend |
| SciPy | ~300-500 ns | No SIMD optimization |

### Industry Context

VelesDB is optimized for **local-first / in-process** deployment with sub-millisecond latencies at 10K-100K scale. Cloud and distributed vector databases (Qdrant, Milvus, Weaviate, Pinecone) target different deployment models and scale profiles (1M+ vectors, multi-node clusters). Direct latency comparisons are not meaningful across these different architectures.

For reproducible VelesDB benchmarks, run:
```bash
cargo bench -p velesdb-core --bench hnsw_benchmark --features internal-bench -- --noplot
```

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
