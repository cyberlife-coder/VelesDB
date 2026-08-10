# Core: performance numbers

All figures below were moved verbatim from `crates/velesdb-core/README.md` to
keep that file under the 400-line documentation budget. The measurement
methodology — hardware, flags, dataset construction — is in
[`docs/BENCHMARKS.md`](../BENCHMARKS.md); the reproducible kit is in
[`benchmarks/`](../../benchmarks/).

Unless stated otherwise, every number was measured on an Intel Core i9-14900KF,
64 GB DDR5, Rust 1.94.1, AVX2, `--release`, `target-cpu=native`, run
sequentially on an idle machine.

---

## Headline number (canonical, full path)

**450 µs p50** end-to-end vector search (10K vectors, 384D, WAL ON,
recall ≥ 96%). Reproduce with
`benchmarks/velesdb_benchmark.py --recall`.

## Vector operations (768D)

| Operation | Time | Throughput |
|-----------|------|------------|
| Dot product | **21.7 ns** | ~35 Gelem/s |
| Euclidean distance | **26.0 ns** | 34.1 Gelem/s |
| Cosine similarity | **33.1 ns** | 23.2 Gelem/s |
| Hamming distance | **35.8 ns** | — |
| Jaccard similarity | **35.1 ns** | — |

These are aligned with the canonical numbers in the repository root README.

## Index-only micro-benchmarks (10K vectors, 768D)

> These measure individual components in isolation — no WAL, no metadata fetch,
> hot cache. They are **not** directly comparable to the end-to-end latency
> above.

| Component micro-benchmark | Result |
|---------------------------|--------|
| HNSW search, index only | **55 µs** (k=10, Balanced mode) |
| VelesQL cache hit | **1.08 µs** (~926K QPS) |
| Sparse search, index only (top-10) | **57.6 µs** (v1.13.0, PR #621 — 16x faster than v1.12) |
| Recall@10 (Accurate mode) | **100%** |

## Key performance characteristics

- End-to-end search latency: **450 µs p50** (10K/384D, WAL ON, recall ≥ 96%) —
  the canonical full-path number.
- HNSW index-only micro-benchmark: **~55 µs** (10K/768D, k=10, Balanced).
- Insert throughput: **3.8–7x faster** than pgvector (10K–100K vectors, Docker
  benchmark v0.7.3 — see [benchmarks/](../../benchmarks/README.md)).
- Bulk import at collection level with persistence: **3.8K–6.4K vectors/sec**
  (768D).
- ColumnStore filtering: up to **130x** faster than JSON scanning at scale
  (integer equality, 100K rows); string equality up to **75x**.

## Recall by configuration (native Rust, Criterion)

| Config | Mode | `ef_search` | Recall@10 | Latency p50 | Status |
|--------|------|-------------|-----------|-------------|--------|
| 10K/128D | Balanced | 128 | **98.8%** | 57 µs | ✅ |
| 10K/128D | Accurate | 512 | **99.9%** | 130 µs | ✅ |
| 10K/128D | Perfect | 4096 | **100%** | 200 µs | ✅ |
| 10K/128D | Adaptive | 32–512 | **95%+** | ~40 µs (easy queries) | ✅ |

> Latency p50 = median over 100 queries. The 55 µs index-only micro-benchmark
> is for 10K/768D in Balanced mode — higher dimensions use SIMD more
> efficiently, so the 128D rows above are a worst case for recall measurement.
> The canonical end-to-end latency remains **450 µs p50**.

## Where the speed comes from

- **Native HNSW with explicit SIMD**: AVX-512 and AVX2 on x86_64 (runtime
  feature detection in `simd_dispatch.rs`), NEON on aarch64, scalar fallback
  everywhere else.
- **Adaptive search**: a two-phase `ef_search` that auto-escalates only for
  hard queries, ~2–4x faster on the median query than a fixed high
  `ef_search`.
- **Bulk insert**: turbo/fast batch modes, parallel HNSW indexing, graduated
  `ef_construction` (VAMANA 3-phase) and lock-free CAS entry-point promotion.
- **Graph traversal**: a CSR snapshot for zero-copy BFS/DFS, `FxHashSet`
  visited sets, and parent-pointer path reconstruction.
- **ColumnStore**: typed columnar metadata instead of JSON scanning.
- **Query plan cache**: see [Core query plan cache](./CORE_QUERY_PLAN_CACHE.md).
- **GPU (optional, `gpu` feature)**: a wgpu-backed compute pipeline for batch
  distance kernels, falling back transparently to SIMD on hosts without a
  usable GPU.

## Running the benchmarks yourself

```bash
# Any single Criterion bench declared in crates/velesdb-core/Cargo.toml
cargo bench -p velesdb-core --bench search_benchmark
cargo bench -p velesdb-core --bench hnsw_benchmark
cargo bench -p velesdb-core --bench simd_benchmark
```

The standardized SIFT1M ANN benchmark is feature-gated because its loader pulls
`flate2`, `tar`, `ureq` and `sha2` in as **regular** optional dependencies —
never enable it in a shipping build:

```bash
cargo bench -p velesdb-core --bench sift1m_recall --features bench-sift1m
```

It also downloads a ~168 MB tarball on first run.

## See also

- [velesdb-core README](../../crates/velesdb-core/README.md)
- [Tuning guide](./TUNING_GUIDE.md) — HNSW parameter tuning
- [Search modes](./SEARCH_MODES.md)
- [Quantization](./QUANTIZATION.md) — memory/recall trade-offs

---

Last updated: 2026-07-25 · Applies to: velesdb-core 5.0.0
