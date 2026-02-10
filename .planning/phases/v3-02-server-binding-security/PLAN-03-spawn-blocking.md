---
phase: v3-02-server-binding-security
plan: 03
name: spawn-blocking-cpu-handlers
wave: 1
depends_on: []
autonomous: true
parallel_safe: true
---

# Plan 03: spawn_blocking for CPU-Intensive Handlers

## Objective

Wrap ALL CPU-intensive server handlers in `tokio::task::spawn_blocking` to prevent blocking the async runtime. Remove all `#[allow(clippy::unused_async)]` annotations that mask this bug. Only truly async-only handlers (`health_check`) stay without `spawn_blocking`.

## Context

- **Requirement:** ECO-05 (Server: Handlers block async runtime)
- **Phase goal:** Runtime safety — concurrent requests must not starve each other
- **Current state:** Only `upsert_points` uses `spawn_blocking`. All other handlers (search, query, traversal, batch, etc.) run blocking core operations directly in async context. Multiple `#[allow(clippy::unused_async)]` hide the issue.
- **Pattern to follow:** `points.rs:54-78` — the existing `upsert_points` handler shows the correct pattern with proper `Ok(Ok(...))`, `Ok(Err(...))`, `Err(...)` triple matching.

## Tasks

### Task 1: Wrap Search Handlers

**Files:**
- `crates/velesdb-server/src/handlers/search.rs`

**Action:**
1. Wrap the blocking core calls in `spawn_blocking` for:
   - `search` — `collection.search()` / `search_with_ef()` / `search_with_filter()`
   - `batch_search` — `collection.search_batch_with_filters()`
   - `multi_query_search` — `collection.multi_query_search()`
   - `text_search` — `collection.text_search()` / `text_search_with_filter()`
   - `hybrid_search` — `collection.hybrid_search()` / `hybrid_search_with_filter()`
2. Remove all `#[allow(clippy::unused_async)]` annotations (5 occurrences).
3. Handle the `JoinError` from `spawn_blocking` (task panic) → return 500.
4. Pattern: clone/move necessary data into the closure. `collection` is `Arc`-based internally so it can be moved into the closure.

**What to avoid:**
- Do NOT clone vectors unnecessarily — move them into the closure.
- Do NOT forget to handle `JoinError` (task panic case).
- Do NOT add `spawn_blocking` to the filter deserialization (serde is fast, only wrap the core call).

**Verify:**
```powershell
cargo check -p velesdb-server
cargo clippy -p velesdb-server -- -D warnings
cargo test -p velesdb-server
```

**Done when:**
- All 5 search handlers use `spawn_blocking`.
- Zero `#[allow(clippy::unused_async)]` in `search.rs`.
- Clippy clean.

### Task 2: Wrap Query and Match Handlers

**Files:**
- `crates/velesdb-server/src/handlers/query.rs`
- `crates/velesdb-server/src/handlers/match_query.rs`

**Action:**
1. In `query.rs`:
   - Wrap `query` handler: the `velesql::Parser::parse()` call is fast (keep in async), but `state.db.execute_query()` and `collection.execute_aggregate()` are blocking → wrap those in `spawn_blocking`.
   - Wrap `explain` handler: parsing is fast but `state.db.get_collection()` involves locking. The explain handler is mostly compute-light, but for consistency wrap the body in `spawn_blocking`.
   - Remove `#[allow(clippy::unused_async)]` (2 occurrences).
   - Remove `#[allow(clippy::too_many_lines)]` — extract explain plan builder into a helper function (addresses clean code issue #8).
2. In `match_query.rs`:
   - Wrap `match_query` handler: `collection.execute_match()` and `execute_match_with_similarity()` are blocking.
   - Move the parse + validate logic before `spawn_blocking`, only wrap the execution.

**Verify:**
```powershell
cargo check -p velesdb-server
cargo test -p velesdb-server
```

**Done when:**
- `query` and `explain` use `spawn_blocking` for core calls.
- `match_query` uses `spawn_blocking` for execution.
- Zero `unused_async` allows in both files.

### Task 3: Wrap Collection, Point, and Index Handlers

**Files:**
- `crates/velesdb-server/src/handlers/collections.rs`
- `crates/velesdb-server/src/handlers/points.rs`
- `crates/velesdb-server/src/handlers/indexes.rs`

**Action:**
1. In `collections.rs`:
   - `create_collection` — `db.create_collection_with_options()` is blocking → wrap.
   - `delete_collection` — `db.delete_collection()` involves I/O → wrap.
   - `flush_collection` — `collection.flush()` is I/O-bound → wrap.
   - `list_collections`, `get_collection`, `is_empty` — these are lightweight reads but still acquire locks. Wrap for consistency and safety.
2. In `points.rs`:
   - `upsert_points` — ALREADY uses `spawn_blocking` ✅. No change.
   - `get_point` — `collection.get()` involves lock → wrap.
   - `delete_point` — `collection.delete()` involves I/O → wrap. Remove `#[allow(clippy::unused_async)]`.
3. In `indexes.rs`:
   - `create_index` — `collection.create_property_index()` is blocking → wrap.
   - `list_indexes` — `collection.list_indexes()` acquires lock → wrap.
   - `delete_index` — `collection.drop_index()` is blocking → wrap.

**Verify:**
```powershell
cargo check -p velesdb-server
cargo clippy -p velesdb-server -- -D warnings
cargo test -p velesdb-server
```

**Done when:**
- All collection, point, and index handlers (except `upsert_points` which is already done) use `spawn_blocking`.
- Zero `#[allow(clippy::unused_async)]` in these files.
- All tests pass.

## Overall Verification

```powershell
cargo check -p velesdb-server
cargo clippy -p velesdb-server -- -D warnings
cargo test -p velesdb-server
# Verify no unused_async allows remain
grep -rn "unused_async" crates/velesdb-server/src/handlers/
```

## Success Criteria

- [ ] ALL CPU-intensive handlers wrapped in `spawn_blocking`
- [ ] Only `health_check` remains a pure async handler (no blocking work)
- [ ] Zero `#[allow(clippy::unused_async)]` in handler files
- [ ] `JoinError` (task panic) → 500 Internal Server Error for all wrapped handlers
- [ ] Clippy clean — no `unused_async` warnings
- [ ] All existing tests pass

## Parallel Safety

- **Exclusive write files:** `handlers/search.rs`, `handlers/query.rs`, `handlers/match_query.rs`, `handlers/collections.rs`, `handlers/points.rs`, `handlers/indexes.rs`
- **Shared read files:** None
- **Conflicts with:** None (no overlap with Plans 01 or 02 handler files — Plan 01 handles graph handlers, Plan 02 handles auth)

## Output

- **Modified:** `handlers/search.rs`, `handlers/query.rs`, `handlers/match_query.rs`, `handlers/collections.rs`, `handlers/points.rs`, `handlers/indexes.rs`
