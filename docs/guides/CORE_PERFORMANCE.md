# Core: performance numbers

All figures below were moved verbatim from `crates/velesdb-core/README.md` to
keep that file under the 400-line documentation budget. The measurement
methodology — hardware, flags, dataset construction — is in
[`docs/BENCHMARKS.md`](../BENCHMARKS.md); the reproducible kit is in
[`benchmarks/`](../../benchmarks/).

Unless stated otherwise, every latency was measured on an Intel Core i9-14900KF,
64 GB DDR5, Rust 1.94.1, AVX2, `--release`, `target-cpu=native`, run
sequentially on an idle machine; the recall figures were re-measured on an
Apple M5 Pro.

---

## Headline number (canonical, full path)

**450 µs p50** end-to-end vector search (10K vectors, 384D, WAL ON,
recall ≥ 96%), measured 2026-03-27 on 1.7.2, when Balanced ran at ef 128
([report](../../benchmarks/report_1.7.2_2026-03-27.json)). Reproduce with
`benchmarks/velesdb_benchmark.py --recall`.

## Vector operations (768D)

| Operation | Time | Run (i9-14900KF, version) |
|-----------|------|---------------------------|
| Dot product | **21.7 ns** | 2026-03-27, 1.7.2 |
| Euclidean distance | **26.0 ns** | 2026-04-03, 1.11.0 |
| Cosine similarity | **33.1 ns** | 2026-03-24, 1.7.0 |
| Hamming distance | **35.8 ns** | 2026-03-24, 1.7.0 |
| Jaccard similarity | **35.1 ns** | 2026-03-24, 1.7.0 |

The rows come from three runs on the same machine, one per date: compare two
kernels only within one run. The 2026-04-03 run timed all five together:
[BENCHMARKS §1](../BENCHMARKS.md#simd-kernel-latency).

## Index-only micro-benchmarks (10K vectors, 768D)

> These measure individual components in isolation — no WAL, no metadata fetch,
> hot cache. They are **not** directly comparable to the end-to-end latency
> above.

| Component micro-benchmark | Result |
|---------------------------|--------|
| HNSW search, index only | **55 µs** (k=10, Balanced mode; i9-14900KF, 2026-03-27 — 53.2 µs in a quiet Apple M5 Pro re-run, 2026-07-20) |
| VelesQL cache hit | **1.08 µs** (~926K QPS) |
| Sparse search, index only (top-10) | **57.6 µs** (v1.13.0, PR #621 — 16x faster than v1.12) |
| Recall@10 (Accurate mode) | **100%** (10K/128D, `recall_benchmark`) |

## Key performance characteristics

- End-to-end search latency: **450 µs p50** (10K/384D, WAL ON, recall ≥ 96%) —
  the canonical full-path number, measured 2026-03-27 on 1.7.2.
- HNSW index-only micro-benchmark: **~55 µs** (10K/768D, k=10, Balanced).
- Insert throughput vs pgvector: no ratio is claimed. The Docker comparison kit
  is in [benchmarks/](../../benchmarks/README.md), and no run of it is recorded
  for a current version.
- Bulk import at collection level with persistence: measured at 384D only —
  [Upsert Path Optimization](../BENCHMARKS.md#upsert-path-optimization-v172);
  no recorded run measures it at 768D.
- ColumnStore filtering: up to **130x** faster than JSON scanning at scale
  (integer equality, 100K rows); string equality up to **75x**.

## Recall by configuration (native Rust, Criterion)

| Config | Mode | `ef_search` | Recall@10 | Status |
|--------|------|-------------|-----------|--------|
| 10K/128D | Balanced | 160 | **99.8%** | ✅ |
| 10K/128D | Accurate | 512 | **100.0%** | ✅ |
| 10K/128D | Perfect | exhaustive | **100%** | ✅ |
| 10K/128D | Adaptive | 32, then 64 if hard | — | not measured |

> Recall re-measured 2026-09-10 on 6.0.0 at the current presets
> (`recall_benchmark`, an index built with `HnswParams::max_recall`). No
> recorded run measures latency at these presets, nor the Adaptive row (#2266);
> see [BENCHMARKS §5](../BENCHMARKS.md#hnsw-recall-profiles-10k128d).
>
> Latency p50 = median over 100 queries. The 55 µs index-only micro-benchmark
> is for 10K/768D in Balanced mode — higher dimensions use SIMD more
> efficiently, so the 128D rows above are a worst case for recall measurement.
> The canonical end-to-end latency remains **450 µs p50** (2026-03-27, 1.7.2).

## Where the speed comes from

- **Native HNSW with explicit SIMD**: AVX-512 and AVX2 on x86_64 (runtime
  feature detection in `simd_dispatch.rs`), NEON on aarch64, scalar fallback
  everywhere else.
- **Adaptive search**: a two-phase `ef_search` that escalates only for hard
  queries, so easy ones stop at a low `ef_search` (the gain is not measured
  yet, #2266).
- **Bulk insert**: turbo/fast batch modes, parallel HNSW indexing, graduated
  `ef_construction` (VAMANA 3-phase), and lock-free entry-point reads (a
  small lock serializes its rare promotions).
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

Last updated: 2026-09-10 · Applies to: velesdb-core 6.0.0
