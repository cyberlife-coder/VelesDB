# Migrating to VelesDB 3.3.0

VelesDB 3.3.0 is a **VelesQL correctness + cross-surface parity** release. It
fixes a large backlog of silent-wrong-result and clause-drop bugs across the
core engine, REST server, CLI, WASM, and the Python / TypeScript SDKs.

It is versioned **3.3.0 (MINOR)** because the vast majority of changes *fix*
previously-incorrect behavior. However, several changes are **client-observable**
— error codes, REST status codes, and query *results* change. If your code
asserts on error codes/statuses, or relied on a query returning its previous
(buggy) result, read the relevant section below.

> **TL;DR** — Nothing in the request/response *shape* breaks. What changes is:
> (1) some error **codes** and REST **statuses**, (2) some queries now return
> **correct** (different) results, and (3) some previously-accepted malformed
> queries now **error** instead of silently misbehaving.

---

## 1. Error codes (most likely to affect SDK clients)

| Surface | Before | After |
|---|---|---|
| Query-shape / bind-param rejects (e.g. missing `$v`, unsupported shape) | `VELES-009` (Config) | **`VELES-010`** (Query) |
| `USING FUSION` misconfiguration (bad RSF weight sum, single-branch, unknown key) | `V006` ("similarity() requires…") | **`V012`** ("USING FUSION is misconfigured") |
| WASM rejections | bare `Error(message)` string, no code | structured `Error` with a machine-readable **`.code`** (`VELES-004`, `VELES-034`, `VELES-010`, …) |

**TypeScript SDK:** query-shape/bind-param failures now narrow to **`QueryError`**
(code `VELES-010`) instead of `ConfigError` (`VELES-009`). If you `catch` and
branch on the error class, update those branches.

```ts
// Before: caught as ConfigError
// After:  caught as QueryError
try { await client.query(db, "... WHERE vector NEAR $missing"); }
catch (e) { if (e instanceof QueryError) { /* now lands here */ } }
```

**Action:** if you assert on `error.code`, map `VELES-009 → VELES-010` for
query-path errors and `V006 → V012` for FUSION-clause errors.

---

## 2. REST status codes (`/match` and graph mutations)

The `/match` endpoint and the graph-mutation handlers previously hand-rolled
error responses (invented codes, blanket `500`s). They now route through the
canonical error mapper:

| Case | Before | After |
|---|---|---|
| `POST /match` to a missing collection | `500` / `COLLECTION_NOT_FOUND` | **`404`** + `VELES-002` |
| `POST /match` with a missing `$param` / bad query | `500` / `EXECUTION_ERROR` | **`400`** + `VELES-010` |
| Duplicate edge (`add_edge`) | `500` (string) | **`409`** + `VELES-019` |
| Query timeout | `408` / `VELES-QUERY-TIMEOUT` | `408` + **`VELES-027`** |
| Rate-limit / circuit-breaker | `429`/`503` with `code: null` | `429`/`503` + **`VELES-027`** |

The error body shape is unchanged (`{ "error": "...", "code": "..." }`); only
the `code` values and HTTP statuses are now canonical. **Action:** if you branch
on HTTP status or `code` from `/match` or graph mutations, adopt the values
above. Clients that simply surface the message are unaffected.

---

## 3. Query results that change (behavior fixes)

These queries previously returned **wrong or unordered** results and now return
the correct ones. If you snapshot-tested or depended on the old output, re-baseline.

- **`ORDER BY` a nested/dotted payload field now sorts.** `ORDER BY meta.source`
  was a silent no-op (flat key lookup); it now orders by the nested value.
- **`ORDER BY similarity(field, $v)` now scores the named vector.** It previously
  always scored the default vector.
- **A bare built-in score variable in `ORDER BY` now ranks.**
  `ORDER BY sparse_score DESC` (and `vector_score` / `bm25_score` / `graph_score`
  / `fused_score`) was a silent no-op; it now ranks by that component score.
- **Unpopulated built-in score variables default to `0`, not the primary score.**
  In hybrid formulas, e.g. on a `NEAR`-only query `bm25_score` is now `0`, so
  `0.7*vector_score + 0.3*bm25_score` evaluates to `0.7 × vector_score` (was the
  full score).
- **`USING FUSION(strategy=…)` is now honored** on the dense-`NEAR` + text-`MATCH`
  hybrid (incl. the anchored path). `maximum` / `average` / `rsf` ran plain RRF
  before; `graph_weight` had no effect. Rankings change for non-`rrf` strategies.
- **`MATCH … ORDER BY … LIMIT` now returns the global top-K** across all traversal
  strategies (it previously returned the sorted top-K of the *first-K traversed*).
- **CLI REPL `SELECT *`** no longer includes the synthetic `score` column (it now
  matches the REST projection contract — score appears only when selected/ordered).

---

## 4. Previously-accepted queries that now error

These were silently accepted/ignored and now fail loudly at validate-time
(`VELES-010` / `V012` unless noted):

- **`USING FUSION` on a single retrieval branch** (a `similarity()`-only or pure
  `NEAR` query) — FUSION needs ≥ 2 fusable branches.
- **`USING FUSION(strategy='rsf', …)`** with `dense_w + sparse_w ≠ 1.0`, or
  negative `weighted` weights.
- **`NEAR_FUSED … USING FUSION 'weighted'|'rsf'`** — undefined over homogeneous
  query vectors.
- **Unknown fusion strategy name or fusion option key** — previously fell back to
  RRF / was discarded silently.
- **CLI `$parameter` vector queries** (e.g. `… WHERE vector NEAR $q`) now return a
  non-zero error instead of an empty success — vector params can't be supplied
  from the CLI; use literal vectors or the REST API.

## 5. New request caps

- **Upsert batch size** is capped at **100 000** points per request
  (`400` if exceeded).
- **Sparse vector non-zeros** are capped at **65 536** per vector (`400` if
  exceeded).

---

## 6. Additive — no migration needed (new capabilities)

- **Scalar subqueries** in `WHERE` / `HAVING` and `INSERT`/`UPDATE`/`DELETE`
  values: `SELECT * FROM t WHERE amount > (SELECT AVG(amount) FROM t)`.
- **Python SDK:** `SearchOptions(fusion=…)` for typed hybrid fusion,
  `Database.set_auto_reindex(name, bool)`, ordered `match_query()`, and a calibrated
  `explain()`.
- **TypeScript SDK:** the `velesql()` builder now emits parseable VelesQL, plus
  typed `db.setAutoReindex()` / `db.alterCollection()` and a `nearFused()` whose
  strategy is compile-time guarded to `rrf | average | maximum`.
- **WASM:** column projection / aliases / window functions, `ORDER BY` arithmetic,
  default `LIMIT`, aggregate `ORDER BY`, machine-readable error codes, and a
  core-aligned `EXPLAIN` vocabulary.
- **DDL:** `ALTER COLLECTION <name> SET (auto_reindex = true|false)` applies and
  persists.

---

## Upgrading

No data-format or storage migration is required — 3.3.0 reads 3.2.x databases
unchanged. Update your dependency pin (crate `velesdb-core`, `@wiscale/velesdb-sdk`
/ `@wiscale/velesdb-wasm`, PyPI `velesdb`) to `3.3.0`, then review sections 1–5
against your client code.
