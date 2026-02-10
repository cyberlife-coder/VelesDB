---
phase: v3-01
plan: 02
name: Core In-Memory API Surface for WASM
status: complete
completed: 2026-02-09
---

# Plan 02 Summary: Core In-Memory API Surface for WASM

## What Was Done

All 3 tasks completed:

### Task 1: Extract Graph Types to Non-Persistence Module ✅

Created `crates/velesdb-core/src/graph/` module (NOT behind `persistence` feature):
- `types.rs` — `GraphNode`, `GraphEdge` with full API (CRUD, properties, vectors, serde)
- `edge_store.rs` — `InMemoryEdgeStore` with node+edge CRUD, bidirectional indexing, label indexes
- `mod.rs` — Module facade with re-exports

Added `Error::NodeExists(u64)` variant (VELES-028) to error.rs for the edge store.

### Task 2: Extract BFS/DFS Traversal to Non-Persistence Module ✅

Created `crates/velesdb-core/src/graph/traversal.rs`:
- `GraphTraversal` trait — generic interface for any graph store
- `TraversalStep` — step result with node_id, depth, path
- `TraversalConfig` — max_depth, limit, rel_types filter
- `bfs()` — BFS traversal with cycle detection, depth limiting, rel_type filter
- `dfs()` — DFS traversal with same capabilities
- `impl GraphTraversal for InMemoryEdgeStore`

### Task 3: Expose JSON-to-Filter Conversion Utility ✅

Created `crates/velesdb-core/src/filter/json_filter.rs`:
- `json_to_condition()` — converts JSON filter objects to core `Condition`
- Supports all operators: eq, neq, gt, gte, lt, lte, in, contains, is_null, is_not_null, like, ilike, and, or, not
- Nested conditions supported

## Tests Created

- `graph/types_tests.rs` — 11 tests (node/edge CRUD, serde, validation)
- `graph/edge_store_tests.rs` — 15 tests (CRUD, cascade delete, indexing, degrees)
- `graph/traversal_tests.rs` — 14 tests (BFS/DFS: linear, diamond, cyclic, depth, limit, rel_type filter)
- `filter/json_filter_tests.rs` — 20 tests (all operators, nested, integration, edge cases)

**Total: 60 new tests, all passing.**

## Verification

- `cargo check --package velesdb-core --no-default-features` ✅
- `cargo check --package velesdb-core` ✅
- `cargo check --package velesdb-wasm` ✅
- `cargo clippy --package velesdb-core -- -D warnings` ✅
- `cargo test --workspace` ✅ (2,652+ tests, 0 failures)
- `cargo fmt --all --check` ✅

## Files Created

- `crates/velesdb-core/src/graph/mod.rs`
- `crates/velesdb-core/src/graph/types.rs`
- `crates/velesdb-core/src/graph/edge_store.rs`
- `crates/velesdb-core/src/graph/traversal.rs`
- `crates/velesdb-core/src/graph/types_tests.rs`
- `crates/velesdb-core/src/graph/edge_store_tests.rs`
- `crates/velesdb-core/src/graph/traversal_tests.rs`
- `crates/velesdb-core/src/filter/json_filter.rs`
- `crates/velesdb-core/src/filter/json_filter_tests.rs`

## Files Modified

- `crates/velesdb-core/src/lib.rs` — Added `pub mod graph;`
- `crates/velesdb-core/src/filter/mod.rs` — Added `pub mod json_filter;`
- `crates/velesdb-core/src/error.rs` — Added `NodeExists(u64)` variant

## Success Criteria Met

- [x] `graph` module exists at `velesdb-core/src/graph/` (NOT persistence-gated)
- [x] `InMemoryEdgeStore` with full CRUD + indexes
- [x] BFS/DFS traversal via `GraphTraversal` trait
- [x] `json_to_condition()` converts JSON filter to core `Condition`
- [x] `cargo check --no-default-features` passes for velesdb-core
- [x] `cargo check --package velesdb-wasm` passes
- [x] All existing tests pass (zero regressions)
