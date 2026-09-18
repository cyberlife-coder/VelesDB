# Engine benchmark numbers behind the Python bindings

Moved out of [`crates/velesdb-python/README.md`](../../crates/velesdb-python/README.md)
to keep that file under the documentation line budget. These are **native Rust
engine** figures: the Python bindings call straight into the same code, but a
Python-side measurement also pays PyO3 conversion cost — for the Python-level
numbers and how to close that gap, read
[PYTHON_PERFORMANCE.md](PYTHON_PERFORMANCE.md).

VelesDB is built in Rust with explicit SIMD optimizations:

| Operation | Time (768D) |
|-----------|-------------|
| Cosine | ~33.1 ns |
| Euclidean | ~26.0 ns |
| Dot Product | ~21.7 ns |
| Hamming | ~35.8 ns |

## System benchmarks (native Rust engine)

| Benchmark | Result |
|-----------|--------|
| **HNSW Search index-only (10K/768D)** | **~55 µs** (k=10, Balanced mode; i9-14900KF, 2026-03-27) |
| **End-to-end p50 (10K/384D, WAL ON)** | **~450 µs** (canonical, recall ≥ 96%; measured 2026-03-27 on 1.7.2) |
| **Recall@10 (Accurate)** | **100%** (10K/128D, `recall_benchmark`) |
| **Insert throughput vs pgvector** | No ratio claimed: no run of the Docker comparison kit (`benchmarks/`) is recorded for a current version |

> Numbers match `docs/reference/promise-contract.json` (the single source
> of truth for the README perf claims).

*The rows come from different runs: the SIMD, index-only HNSW and end-to-end
figures from the i9-14900KF, the recall from an Apple M5 Pro (2026-09-10). See
[benchmarks/](../../benchmarks/) for methodology.*

---

Last updated: 2026-09-10 · Applies to: velesdb-core 6.0.0
