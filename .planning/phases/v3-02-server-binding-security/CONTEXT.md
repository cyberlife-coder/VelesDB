# Phase v3-02: Server Binding & Security — Audit Context

## Phase Goal

Server becomes a thin HTTP layer over `velesdb-core`. Add authentication and runtime safety.

**Requirements:** ECO-03, ECO-04, ECO-05, ECO-14

---

## Audit Findings

### 🚨 ECO-03: No Authentication/Authorization

- **Zero auth** anywhere in the server — all endpoints publicly accessible.
- Need: API key middleware (`Authorization: Bearer <key>` + `VELESDB_API_KEY` env var).
- Optional: disabled if env var not set (dev mode).

### 🚨 ECO-04: GraphService Disconnected from Core

- `GraphService` in `handlers/graph/service.rs` maintains its own in-memory `EdgeStore` per collection.
- Completely **separate** from the collection's EdgeStore managed by `velesdb-core`.
- Graph data added via REST API is **NOT visible** to VelesQL MATCH queries.
- Data is lost on server restart (warned in `main.rs:66-72`).

### 🚨 ECO-02 (server part): Duplicate BFS/DFS

- `service.rs:92-216` reimplements BFS and DFS from scratch.
- Core's `Collection` already exposes `traverse_bfs()` and `traverse_dfs()` in `graph_api.rs`.
- Core also provides generic `graph::traversal::bfs()` / `dfs()` via `GraphTraversal` trait.
- Server MUST delegate to core — zero reimplemented traversal logic.

### 🐛 ECO-05: Handlers Block Async Runtime

- **Only** `upsert_points` correctly uses `spawn_blocking` (`points.rs:56`).
- ALL other CPU-intensive handlers run blocking work directly in async context:
  - `search`, `batch_search`, `text_search`, `hybrid_search`, `multi_query_search`
  - `query`, `explain`, `match_query`
  - `create_collection`, `delete_collection`, `flush_collection`
  - `create_index`, `list_indexes`, `delete_index`
  - `get_point`, `delete_point`
  - `traverse_graph`, `add_edge`, `get_edges`, `get_node_degree`
- Multiple `#[allow(clippy::unused_async)]` annotations mask this issue.

### ⚠️ ECO-14: No Rate Limiting

- No rate limiting middleware of any kind.
- Need: tower-based rate limiting, configurable per IP, default 100 req/s.

---

## Clean Code Issues (Craftsman)

| # | File | Issue | Severity |
|---|------|-------|----------|
| 1 | `lib.rs:1-13` | 13 blanket clippy allows suppressing ALL quality | ⚠️ |
| 2 | `graph/mod.rs:9` | `#![allow(dead_code)]` blanket | ⚠️ |
| 3 | `metrics.rs:11` | `#![allow(dead_code)]` blanket | ⚠️ |
| 4 | `handlers/mod.rs:38` | `#[allow(unused_imports)]` on legitimate imports | 🔧 |
| 5 | `search.rs` (5 places) | `#[allow(clippy::unused_async)]` masking ECO-05 | 🐛 |
| 6 | `query.rs:29,147` | Same `unused_async` issue | 🐛 |
| 7 | `points.rs:145` | `delete_point` has `unused_async` + runs blocking sync | 🐛 |
| 8 | `query.rs:135` | `#[allow(clippy::too_many_lines)]` — explain needs refactoring | ⚠️ |
| 9 | `query.rs:360` | `#[allow(dead_code)]` on `detect_query_type` | 🔧 |
| 10 | Multiple files | Tests inline in handler files (convention: separate files) | 🔧 |
| 11 | `main.rs:141` | `CorsLayer::permissive()` — security concern | ⚠️ |
| 12 | `stream.rs:87,100` | `as u64` casts on `as_millis()` without safety comment | 🔧 |

---

## Core Graph API Available (graph_api.rs)

The server can bind directly to these `Collection` methods:

| Method | Signature | Notes |
|--------|-----------|-------|
| `add_edge` | `(&self, edge: GraphEdge) -> Result<()>` | Persistent via EdgeStore |
| `get_edges_by_label` | `(&self, label: &str) -> Vec<GraphEdge>` | Uses label index |
| `get_outgoing_edges` | `(&self, node_id: u64) -> Vec<GraphEdge>` | |
| `get_incoming_edges` | `(&self, node_id: u64) -> Vec<GraphEdge>` | |
| `traverse_bfs` | `(&self, source, max_depth, rel_types, limit) -> Result<Vec<TraversalResult>>` | |
| `traverse_dfs` | `(&self, source, max_depth, rel_types, limit) -> Result<Vec<TraversalResult>>` | |
| `get_node_degree` | `(&self, node_id: u64) -> (usize, usize)` | (in, out) |
| `remove_edge` | `(&self, edge_id: u64) -> bool` | |
| `edge_count` | `(&self) -> usize` | |

---

## Plan Decomposition

| Plan | Name | Wave | Requirements | Depends |
|------|------|------|-------------|---------|
| 01 | Graph Unification | 1 | ECO-04, ECO-02 | — |
| 02 | Authentication Middleware | 1 | ECO-03 | — |
| 03 | spawn_blocking for CPU Handlers | 1 | ECO-05 | — |
| 04 | Rate Limiting + Security | 2 | ECO-14 | 02 |
| 05 | Clean Code Sweep | 2 | — | 01, 03 |

### Wave Parallelism

- **Wave 1:** Plans 01, 02, 03 — independent file sets (see isolation below)
- **Wave 2:** Plans 04, 05 — depend on Wave 1

### File Isolation (Wave 1)

| File | Plan 01 (Graph) | Plan 02 (Auth) | Plan 03 (spawn_blocking) |
|------|-----------------|----------------|--------------------------|
| `handlers/graph/*` | ✏️ WRITE | — | — |
| `handlers/auth.rs` (new) | — | ✏️ WRITE | — |
| `handlers/mod.rs` | ✏️ WRITE | ✏️ WRITE | — |
| `handlers/search.rs` | — | — | ✏️ WRITE |
| `handlers/query.rs` | — | — | ✏️ WRITE |
| `handlers/collections.rs` | — | — | ✏️ WRITE |
| `handlers/points.rs` | — | — | ✏️ WRITE |
| `handlers/indexes.rs` | — | — | ✏️ WRITE |
| `handlers/match_query.rs` | — | — | ✏️ WRITE |
| `main.rs` | ✏️ WRITE | ✏️ WRITE | — |
| `lib.rs` | ✏️ WRITE | ✏️ WRITE | — |
| `Cargo.toml` | — | ✏️ WRITE | — |

**Conflict:** Plans 01 & 02 both write `main.rs`, `lib.rs`, `handlers/mod.rs`.
**Resolution:** Plan 01 executes first (largest architectural change), then Plan 02 layers auth on top.
Plans 02 & 03 have zero file overlap → parallel safe.
