# Performance Remediation Report (2026-03-10)

Host:

- CPU: `Intel(R) Core(TM) i9-14900KF`
- OS: `Windows 11`
- `rustc`: `1.92.0`
- `cargo`: `1.92.0`
- crate features: `persistence`, `internal-bench`

Commands:

- `cargo bench -p velesdb-core --features internal-bench --bench simd_benchmark -- 768 --noplot`
- `cargo bench -p velesdb-core --features internal-bench --bench sparse_benchmark -- sparse_insert --noplot`
- `cargo bench -p velesdb-core --features internal-bench --bench velesql_benchmark -- velesql_cache --noplot`

Results summary:

- SIMD cosine `768D`
  - dispatch before threshold change: `~38.9 ns`
  - direct AVX2 2-acc kernel: `~35.4 ns`
  - direct AVX2 4-acc kernel: `~30.8 ns`
  - dispatch after threshold change to AVX2 4-acc at `768D`: `~33.4 ns`
- Sparse insert `10K`
  - sequential: `~86.1 ms`
  - rayon doc-granular: `~210.1 ms`
  - manual 4x2500 doc-granular: `~168.7 ms`
  - manual 4x2500 chunked: `~53.2 ms`
- `VelesQL` cache / parsing
  - canonicalize + hash: `~374.5 ns`
  - cache hit full (complex query): `~1.38 us`
  - cache hit without stats: `~1.12 us`
  - direct parse (complex query): `~8.71 us`

Decisions taken:

- Lowered the AVX2 cosine 4-acc dispatch threshold from `1024` to `768`.
- Replaced sparse insert lock-per-document pressure with a chunked batch merge path.
- Switched `QueryCache` hit path from `cache.write()` to `upgradable_read()` and moved cache stats to atomics.
- Added `internal-bench` wrappers to benchmark real scalar, dispatch, resolved, and direct kernel paths without promoting them to stable public API.

Notes:

- These values are valid for this host and toolchain only.
- They are suitable for local regression tracking and code-level decisions.
- They must not be reused as cross-product marketing claims without parity methodology.
