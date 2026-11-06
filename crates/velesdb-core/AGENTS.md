# AGENTS.md — velesdb-core

The engine. Root [`AGENTS.md`](../../AGENTS.md) is the contract; this file holds
what only applies inside this crate.

- **Test layout:** unit tests live beside their module as `*_tests.rs`;
  integration/BDD suites in `crates/velesdb-core/tests/`. New tests follow the
  placement of the suite they extend — do not invent a third location.
- **Tests are single-threaded** (`--test-threads=1`) — they share filesystem
  state under `tempfile`. A test that fails only under parallelism is broken,
  not flaky.
- **Search path is recall-gated.** Touching `index/hnsw/`, `simd_native/`,
  `quantization/`, or `fusion/` requires `cargo test -p velesdb-core
  --features persistence test_recall -- --test-threads=1` — recall@10 ≥ 0.95.
- **Filesystem paths are validated at the sink.** Collection and index names
  reach `Path::join` only through `Database::collection_dir` /
  `sparse_file_prefix` — never join a name directly, even when validation
  already ran upstream (the join is the boundary the analyzer and the next
  caller check).
- **Premium boundary:** `velesdb-core` must never reference a premium crate,
  type, or symbol. The policy port is `src/observer/` (`DatabaseObserver`);
  enforcement lives out-of-tree.
- **Feature-gated items** must carry `#[cfg(feature = ...)]` at the item — a
  test importing a gated item needs a matching `#![cfg]` or the isolated
  feature lint pass fails.
- **Doc claims are contracted.** Any measured figure (×, %, MB) in a public
  doc or module doc must be registered in
  `docs/reference/promise-contract.json` with provenance; figures live in one
  authoritative location, secondary comments reference the issue, never
  restate the number.
