# AGENTS.md — velesdb-memory

Independent 0.x crate — agentic memory layer. Root [`AGENTS.md`](../../AGENTS.md)
is the contract; this file holds what only applies here.

- **0.x semver: MINOR is the breaking component.** Bump minor for any change
  that breaks an out-of-tree implementor or caller (trait method without a
  default, signature change, renamed export); patch otherwise. A BREAKING
  CHANGELOG entry leads with the migration path.
  `scripts/check-version-sync.py` must stay green.
- **Feature matrix is a contract.** Local verification mirrors CI's blocking
  `memory-feature-matrix` job — four shapes plus each feature in isolation
  with `--all-targets` (compiles tests/examples a test-only build misses,
  #2202):
  ```bash
  cargo check -p velesdb-memory --no-default-features
  cargo check -p velesdb-memory --no-default-features --features context
  cargo check -p velesdb-memory
  cargo check -p velesdb-memory --no-default-features --features mcp,persistence
  for feat in mcp http context persistence embedder-http extractor-http ollama extract gpu; do
    cargo check -p velesdb-memory --no-default-features --features "$feat" --all-targets
  done
  ```
- **No std clock in wasm-reachable code.** `SystemTime::now()`/`Instant`
  abort on wasm32 — cfg-gate (pattern: `now_nanos()` in
  `src/context/memory_bridge.rs`).
- **Upgrade is load-bearing.** `memory-upgrade-guard` replays
  upgrade/rollback/kill-mid-write on a real store and gates the release —
  touching storage format or defaults needs that job's attention.
- **Provenance is written, not inferred.** Bindings record who wrote a store
  (model, dims, writer) on open — fixtures asserting `recorded: false` were
  the pre-provenance contract.
