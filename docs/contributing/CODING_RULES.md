# VelesDB — Development Rules

Repo-specific coding rules. The canonical copies of the CI-enforced constraints
and the exact pre-push command block live in [`AGENTS.md`](../../AGENTS.md);
gate thresholds and their enforcement live in [`QUALITY_BAR.md`](../../QUALITY_BAR.md).
This file does not restate them — it carries the rules that live nowhere else.

---

## Architecture

### Crate Structure

```
VelesDB/
├── crates/
│   ├── velesdb-core/          # Core engine (storage, indexing, search)
│   ├── velesdb-server/        # Axum REST API server
│   ├── velesdb-cli/           # CLI / VelesQL REPL
│   ├── velesdb-python/        # Python bindings (PyO3)
│   ├── velesdb-wasm/          # Browser WASM (no persistence)
│   ├── velesdb-mobile/        # iOS/Android (UniFFI)
│   ├── velesdb-migrate/       # Migration tooling
│   ├── velesdb-memory/        # MCP agent-memory server (independent 0.x cadence)
│   ├── velesdb-node/          # Node.js binding of the memory wedge (napi-rs)
│   └── tauri-plugin-velesdb/  # Tauri plugin
```

### Architectural Principles

- **Separation of concerns**: each module has a single responsibility.
- **Stable API**: core is a versioned dependency of the premium crate.
- **Zero-copy**: prefer `&[u8]`, `Bytes`, `memmap2` for performance.
- **Concurrency**: `parking_lot::RwLock` throughout (never `std::sync`).
- **Error handling**: `thiserror` for typed errors. No `anyhow` in library crates.
- **Numeric casts**: `try_from` for `u64`-to-`usize` casts, never `as usize`
  (clippy::pedantic).
- **Features**: always explicit (`--features persistence,gpu,update-check`) —
  never `--all-features` (feature unification hides gating bugs).

---

## Test-Driven Development

1. **Red** — write a failing test. 2. **Green** — minimum code to pass.
3. **Refactor** — improve without breaking tests.

Structure each test as Arrange-Act-Assert, named
`test_<function>_<scenario>()`:

```rust
fn test_search_returns_top_k_results() { ... }
fn test_insert_with_invalid_dimension_fails() { ... }
fn test_delete_nonexistent_point_is_noop() { ... }
```

Cover the success path AND the error path of every public function; use
`proptest` for invariants worth holding under arbitrary input (e.g. distance
symmetry). Build shared fixtures as `#[cfg(test)]` helpers with `tempfile`.
Coverage target: > 80% (`cargo tarpaulin`).

Tests run single-threaded — they share filesystem state:

```bash
cargo test --workspace --features persistence,gpu,update-check \
  --exclude velesdb-python -- --test-threads=1
```

Anti-patterns: `#[ignore]` without a reason, order-dependent tests,
message-less `unwrap()` in tests (use `expect("context")`), vague assertions,
flaky/random tests.

---

## Naming Conventions

| Type | Convention | Example |
|------|------------|---------|
| Structs | PascalCase | `VectorIndex` |
| Traits | PascalCase | `Searchable` |
| Functions | snake_case | `find_nearest` |
| Constants | SCREAMING_SNAKE | `MAX_DIMENSIONS` |
| Modules | snake_case | `vector_storage` |

---

## Security

- `cargo audit` and `cargo deny check` must pass.
- No `unsafe` without a documented `// SAFETY:` comment.
- Validate all user input; no secrets in code.
- **Fail closed on untrusted shapes.** Any matcher, guard, or evaluator that
  interprets untrusted input — filter conditions, operator dispatch, mode/type
  parsing, wire-format tags — must treat an *unknown, absent, or malformed*
  shape as no-match / error, never as match-all. This is enforced by mechanism,
  not just convention:
  - **In-crate enums** (`#[non_exhaustive]` defined in this workspace): match
    exhaustively with **no permissive wildcard**, so a new variant breaks the
    build until it is handled. Exemplar:
    `collection::search::query::match_exec::where_eval::eval_match_condition`.
  - **Cross-crate enums** (the compiler forces a wildcard): the wildcard must be
    `_ => false` / `_ => Err(..)`, **never** `_ => true` / `_ => Ok(true)`, and
    must carry a regression test. Exemplar: `velesdb-wasm`'s `filter::evaluate_condition`
    with `filter_tests.rs`'s fail-closed cases.

---

## Performance

- **Measure before optimizing**; benchmarks use `criterion`
  (`cargo bench -p velesdb-core --bench simd_benchmark -- --noplot`),
  profiling uses `cargo flamegraph`.
- The regression baseline lives at `benchmarks/baseline.json`.

---

Release procedure: [`RELEASE.md`](RELEASE.md). Pre-push validation: the
command block in [`AGENTS.md`](../../AGENTS.md) — do not copy it here.
