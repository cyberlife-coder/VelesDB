---
phase: v3-02-server-binding-security
plan: 01
name: graph-unification
wave: 1
depends_on: []
autonomous: true
parallel_safe: false
---

# Plan 01: Graph Unification — Delete GraphService, Bind to Core

## Objective

Delete the server's independent `GraphService` (in-memory EdgeStore) and rewrite all graph handlers to delegate to `Collection` methods from `velesdb-core`. This eliminates 100% of reimplemented BFS/DFS logic and unifies graph data with the Collection's EdgeStore (still in-memory, but now shared with VelesQL MATCH queries).

## Context

- **Requirements:** ECO-04 (GraphService disconnected), ECO-02 (duplicate BFS/DFS)
- **Phase goal:** Server = thin HTTP layer over velesdb-core
- **Current state:** `GraphService` in `handlers/graph/service.rs` maintains its own `EdgeStore` per collection, completely disconnected from core. BFS/DFS are reimplemented (~150 lines). Graph data is lost on restart.
- **Core API available:** `Collection::add_edge()`, `get_edges_by_label()`, `traverse_bfs()`, `traverse_dfs()`, `get_node_degree()` — all in `collection/core/graph_api.rs`

## Tasks

### Task 1: Delete GraphService and Rewrite Graph Handlers

**Files:**
- `crates/velesdb-server/src/handlers/graph/service.rs` (DELETE)
- `crates/velesdb-server/src/handlers/graph/handlers.rs` (REWRITE)
- `crates/velesdb-server/src/handlers/graph/mod.rs` (UPDATE)
- `crates/velesdb-server/src/handlers/graph/stream.rs` (UPDATE)

**Action:**
1. Delete `service.rs` entirely — it reimplements EdgeStore + BFS + DFS.
2. Rewrite `handlers.rs`:
   - All graph handlers now take `State(state): State<Arc<AppState>>` instead of `State<GraphService>`.
   - `get_edges` → call `state.db.get_collection(&name)?.get_edges_by_label(&label)`.
   - `add_edge` → call `collection.add_edge(edge)`.
   - `traverse_graph` → call `collection.traverse_bfs()` or `collection.traverse_dfs()`.
   - `get_node_degree` → call `collection.get_node_degree(node_id)`.
   - Wrap all handlers in `spawn_blocking` since graph operations are CPU-bound.
   - Return 404 if collection not found (consistent with other handlers).
3. Rewrite `stream.rs`:
   - Replace `State<Arc<GraphService>>` with `State<Arc<AppState>>`.
   - Delegate to `Collection::traverse_bfs()` / `traverse_dfs()` instead of `GraphService`.
   - Wrap traversal in `spawn_blocking`.
   - Apply same edge-ID path adapter as regular traverse handler (see point 5).
   - Wire SSE route in `main.rs`: `.route("/collections/{name}/graph/traverse/stream", get(stream_traverse))`
4. Update `mod.rs`:
   - Remove `mod service;` and `pub use service::GraphService;`.
   - Remove `#![allow(dead_code)]`.
   - Update re-exports.
   - Update tests to use `Collection` directly instead of `GraphService`.
5. Map `TraversalResult` (core) → `TraversalResultItem` (server types) in handler.
   - Core's `TraversalResult.path` contains **node IDs** (includes source); server's `TraversalResultItem.path` contains **edge IDs**.
   - **Decision: Keep edge-ID paths** to avoid REST API breaking change.
   - Build adapter: for each consecutive pair of nodes in core's path, look up the edge connecting them via `collection.get_outgoing_edges()` and collect edge IDs.
   - If edge lookup fails (data race), fall back to empty path with `tracing::warn!`.

**Verify:**
```powershell
cargo check -p velesdb-server
cargo test -p velesdb-server
```

**Done when:**
- `service.rs` is deleted.
- Zero `GraphService` references in codebase.
- All graph handlers use `Collection` methods from core.
- Graph tests use `Collection` / `Database` directly.

### Task 2: Update Router and Lib Re-exports

**Files:**
- `crates/velesdb-server/src/main.rs`
- `crates/velesdb-server/src/lib.rs`
- `crates/velesdb-server/src/handlers/mod.rs`

**Action:**
1. In `main.rs`:
   - Remove `GraphService::new()` instantiation and its warning log.
   - Remove separate `graph_router` with `GraphService` state.
   - Merge graph routes into the main `api_router` with `AppState`.
   - Remove `GraphService` from imports.
2. In `lib.rs`:
   - Remove `GraphService` from re-exports.
   - Keep graph handler re-exports (they now take `AppState`).
   - Remove `GraphService` from `pub use handlers::graph::*`.
3. In `handlers/mod.rs`:
   - Remove `GraphService` from re-exports.
   - Remove `#[allow(unused_imports)]` if no longer needed.

**Verify:**
```powershell
cargo build -p velesdb-server
cargo test -p velesdb-server
grep -rn "GraphService" crates/velesdb-server/src/
```

**Done when:**
- `grep -rn "GraphService" crates/velesdb-server/src/` returns zero results.
- Graph routes use `AppState` like all other routes.
- Server builds and all tests pass.

### Task 3: Update Integration Tests

**Files:**
- `crates/velesdb-server/tests/api_integration.rs`

**Action:**
1. Search for any graph-related integration tests.
2. Update them to work without `GraphService`.
3. Add test: graph edge added via REST API is visible in the same collection's EdgeStore.
4. Add test: traversal results match core's `Collection::traverse_bfs()` output.

**Verify:**
```powershell
cargo test -p velesdb-server --test api_integration
```

**Done when:**
- All integration tests pass.
- At least one test verifies graph-core binding consistency.

## Overall Verification

```powershell
cargo check -p velesdb-server
cargo clippy -p velesdb-server -- -D warnings
cargo test -p velesdb-server
grep -rn "GraphService" crates/velesdb-server/src/
```

## Success Criteria

- [ ] `GraphService` struct and `service.rs` fully deleted
- [ ] Zero reimplemented BFS/DFS in server
- [ ] All graph handlers delegate to `Collection` methods
- [ ] Graph data accessible to VelesQL MATCH queries via shared Collection EdgeStore
- In-memory warning updated (edge persistence is still in-memory, same as core — future EPIC for disk persistence)
- [ ] All server tests pass
- [ ] Clippy clean

## Parallel Safety

- **Exclusive write files:** `handlers/graph/*`, `main.rs`, `lib.rs`, `handlers/mod.rs`
- **Shared read files:** `velesdb-core/src/collection/core/graph_api.rs`
- **Conflicts with:** Plan 02 (both write `main.rs`, `lib.rs`, `handlers/mod.rs`) → execute Plan 01 first

## Output

- **Deleted:** `handlers/graph/service.rs`
- **Modified:** `handlers/graph/handlers.rs`, `handlers/graph/mod.rs`, `handlers/graph/stream.rs`, `handlers/graph/types.rs`, `handlers/mod.rs`, `main.rs`, `lib.rs`
- **Modified:** `tests/api_integration.rs`
- **Added:** SSE streaming route for graph traversal in `main.rs`
