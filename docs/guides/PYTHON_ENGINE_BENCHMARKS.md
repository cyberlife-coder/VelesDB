# Engine benchmark numbers behind the Python bindings

Moved out of [`crates/velesdb-python/README.md`](../../crates/velesdb-python/README.md)
to keep that file under the documentation line budget. These are **native Rust
engine** figures: the Python bindings call straight into the same code, but a
Python-side measurement also pays PyO3 conversion cost — for the Python-level
numbers and how to close that gap, read
[PYTHON_PERFORMANCE.md](PYTHON_PERFORMANCE.md).

VelesDB is built in Rust with explicit SIMD optimizations:

| Operation | Time (768D) | Throughput |
|-----------|-------------|------------|
| Cosine | ~33.1 ns | 23.2 Gelem/s |
| Euclidean | ~26.0 ns | 34.1 Gelem/s |
| Dot Product | ~21.7 ns | ~35 Gelem/s |
| Hamming | ~35.8 ns | -- |

## System benchmarks (native Rust engine)

| Benchmark | Result |
|-----------|--------|
| **HNSW Search index-only (10K/768D)** | **~55 µs** (k=10, Balanced mode) |
| **End-to-end p50 (10K/384D, WAL ON)** | **~450 µs** (canonical, recall ≥ 96%) |
| **Recall@10 (Accurate)** | **100%** |
| **Insert throughput vs pgvector** | **3.8-7x faster** (10K-100K vectors, internal benchmarks on i9-14900KF, not independently verified) |

> Numbers match `docs/reference/promise-contract.json` (the single source
> of truth for the README perf claims).

*Measured with Criterion.rs on i9-14900KF. See
[benchmarks/](../../benchmarks/) for methodology.*

---

Last updated: 2026-07-25 · Applies to: velesdb-core 4.0.0
