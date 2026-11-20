# VelesQL REST Contract

Canonical contract for VelesQL server endpoints and payloads.

- Document revision: `3.8.1`
- Contract version emitted at runtime: `3.0.0` — the value of
  `VELESQL_CONTRACT_VERSION` (`crates/velesdb-core/src/api_types/mod.rs`),
  returned as `meta.velesql_contract_version` in every response. The document
  revision tracks edits to this file; only the runtime constant appears on the
  wire.
- Last updated: `2026-10-30`

This document is the normative REST contract baseline for VelesQL.
When behavior differs between docs and runtime, runtime must be fixed or this
document must be updated in the same PR.

## Endpoints

### `POST /query`

Unified endpoint for `SELECT` and top-level `MATCH` queries.

Request body:

```json
{
  "query": "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10",
  "params": { "v": [0.1, 0.2, 0.3] },
  "collection": "docs"
}
```

Aggregation queries are accepted on `/query` for backward compatibility, but
`/aggregate` is the explicit endpoint for aggregation workloads.

### `POST /aggregate`

Aggregation-only endpoint for `GROUP BY`/`HAVING`/aggregate queries.

Request body:

```json
{
  "query": "SELECT category, COUNT(*) FROM docs GROUP BY category",
  "params": {},
  "collection": "docs"
}
```

Rules:

- Query must be aggregation-shaped (`GROUP BY` or aggregate functions in `SELECT`).
- Non-aggregation queries return `422` with `VELESQL_AGGREGATION_ERROR`.
- The collection is the statement's `FROM <collection>`, which the grammar
  requires of every `SELECT`; the body `collection` and `params._collection`
  are not read.

Success response shape:

```json
{
  "result": [{ "category": "tech", "count": 42 }],
  "timing_ms": 1.12,
  "meta": {
    "velesql_contract_version": "3.0.0",
    "count": 1
  }
}
```

Rules:

- `collection` is optional for `SELECT ... FROM <collection> ...`.
- A top-level `MATCH (...) ...` sent to `/query` names its collection with
  `collection` in the body or `_collection` in `params`: the body fills
  `_collection` when absent, and an explicit `_collection` wins. With neither
  (an empty body `collection` is none), it is refused with
  `VELESQL_MISSING_COLLECTION`. An explicit `_collection` that is not a
  non-empty string is refused with `VELESQL_PARAMETER_ERROR`
  (`details.parameter = "_collection"`, core's `VELES-038`). This is core's
  one rule (`velesql::collection_target` / `with_collection_param`), which the
  Tauri plugin's `query` and the WASM executor apply too; only a top-level
  `MATCH` reads `_collection`, every other statement its own `FROM` or table.
- For graph-only execution, `/collections/{name}/match` remains supported.
- `SELECT ... WHERE ... AND MATCH (...)` is supported and does not require
  `collection` in body when `FROM <collection>` is present.
- The MATCH anchor (first node of the pattern) must carry an alias; when the
  `FROM`/`JOIN` clauses declare aliases, the anchor alias either is one of
  them (explicit binding) or — when no pattern alias matches a declared
  alias — binds implicitly to the FROM rows, guarded by G1/G2/G3 (see the
  spec's "Anchor rule (V011)"). Violations fail validation with error code
  `V011`. When `FROM` has no alias, any anchor alias is accepted.
- Execution note: graph predicates that are AND-required by the WHERE clause
  take the GraphFirst anchored fetch — the pattern is evaluated first and
  retrieval is exhaustive within its anchor set (NEAR, metadata-only,
  sparse-only and `NOT similarity()` shapes; a NEAR fused with a text
  `MATCH` ranks both branches within the anchors its metadata conditions
  match). Residual shapes (predicates under `OR`/`NOT`, `similarity()`
  cascades, a text `MATCH` without a `NEAR`, hybrid dense+sparse fusion)
  keep the windowed execution: a ranked fetch filters an
  over-fetched window of `max(LIMIT, min(LIMIT × 10, 10 000))` candidates
  (rows ranked beyond it are not returned); unranked shapes scan up to
  100 000 points in storage order.

Success response shape:

```json
{
  "results": [{ "id": 1, "score": 0.98, "payload": { "title": "Doc" } }],
  "timing_ms": 1.42,
  "took_ms": 1,
  "rows_returned": 1,
  "meta": {
    "velesql_contract_version": "3.0.0",
    "count": 1
  }
}
```

### DDL and Mutation Statements via `/query`

DDL statements (`CREATE COLLECTION`, `DROP COLLECTION`, `CREATE INDEX`, `DROP INDEX`,
`ANALYZE`, `TRUNCATE`, `ALTER COLLECTION`) and graph/delete mutation statements
(`INSERT EDGE`, `DELETE EDGE`, `DELETE FROM`, `INSERT NODE`, `SELECT EDGES`) are
submitted through the same `POST /query` endpoint.

Request body (DDL example):

```json
{
  "query": "CREATE COLLECTION documents (dimension = 768, metric = 'cosine') WITH (storage = 'sq8')",
  "params": {},
  "collection": ""
}
```

Request body (graph mutation example — `collection` is optional, extracted from SQL):

```json
{
  "query": "INSERT EDGE INTO knowledge (source = 1, target = 2, label = 'AUTHORED_BY') WITH PROPERTIES (year = 2026)",
  "params": {}
}
```

DDL success response shape (standard `QueryResponse` with zero rows):

```json
{
  "results": [],
  "timing_ms": 2.31,
  "took_ms": 2,
  "rows_returned": 0,
  "meta": {
    "velesql_contract_version": "3.0.0",
    "count": 0
  }
}
```

DDL error response shape uses the standard VelesQL error model (see below).

Rules:

- DDL and graph/delete mutation statements always route to `/query`, never to `/aggregate`.
- DDL statements (`CREATE COLLECTION`, `DROP COLLECTION`, `CREATE INDEX`, `DROP INDEX`, `ANALYZE`, `TRUNCATE`, `ALTER COLLECTION`) do not require a `collection` field in the body (the collection name is embedded in the SQL statement).
- `DROP COLLECTION IF EXISTS` returns success even if the collection does not exist.
- Graph/delete mutation statements (`INSERT EDGE`, `DELETE EDGE`, `DELETE FROM`, `INSERT NODE`, `SELECT EDGES`) extract the collection name from the SQL statement; the `collection` field in the request body is ignored.
- `INSERT INTO` and `UPDATE` statements flow through the standard query path and return result rows in the `results` array.
- `UPDATE` and a `DELETE FROM` whose `WHERE` names no ids touch the rows a `SELECT` with the same `WHERE` reads — on a graph, every stored node, with an embedding or not — and write each back by what it stores; on a graph with embeddings an `INSERT` without `vector` writes the node's payload, a new node without an embedding and an existing one keeping its own, and `UPDATE … SET vector` attaches one (`VELES-004` for another dimension, an empty vector included), while a collection that stores no vectors refuses the column (`VELES-014`). `INSERT`, `UPSERT` and `UPDATE` return the rows they leave stored, as a `SELECT` reads them back; in core the read follows the write — one per row written, in statement order: a row already TTL-expired or deleted by a concurrent write in between is left out, an id written twice is returned twice as finally stored, and no returned row carries sparse vectors. A DML `WHERE` holding a `similarity()`, `NEAR`, graph or full-text `MATCH` predicate is refused with `VELES-010` before its parameters are bound, so the shape error wins over an unbound `$name` (`VELES-038`) — one rule, `velesql::check_dml_where`, in core and in the WASM executor. See `VELESQL_SPEC.md`, "Which rows DML reads and writes".

### `POST /collections/{name}/match`

Collection-scoped endpoint for graph `MATCH` queries.

Success response shape:

```json
{
  "results": [
    {
      "bindings": { "a": 1, "b": 2 },
      "score": 0.91,
      "depth": 1,
      "projected": { "a.name": "Alice" }
    }
  ],
  "took_ms": 4,
  "count": 1,
  "meta": {
    "velesql_contract_version": "3.0.0"
  }
}
```

## Standard Error Model (VelesQL)

Semantic/runtime errors for VelesQL endpoints use:

```json
{
  "error": {
    "code": "VELESQL_MISSING_COLLECTION",
    "message": "MATCH query via /query requires `collection` in request body or `_collection` in params",
    "hint": "Add `collection` to the /query JSON body or use /collections/{name}/match",
    "details": {
      "field": "collection",
      "endpoint": "/query",
      "query_type": "MATCH"
    }
  }
}
```

Current codes:

- `VELESQL_MISSING_COLLECTION`
- `VELESQL_COLLECTION_NOT_FOUND`
- `VELESQL_EXECUTION_ERROR`
- `VELESQL_AGGREGATION_ERROR`
- `VELESQL_VALIDATION_ERROR`
- `VELESQL_MUTATION_ERROR`
- `VELESQL_EXPLAIN_ERROR` — `/query/explain` without `analyze`: the read gate
  refused the plan, or the statement failed the planner's checks
- `VELESQL_EXPLAIN_ANALYZE_ERROR` — `/query/explain` with `analyze: true`:
  executing the statement failed
- `VELESQL_PARAMETER_ERROR`

A statement that fails during execution answers `422` when the failure is
the client's (`Error::is_caller_error`: a query error, an unsupported shape,
a guard-rail ceiling the query crossed, a `WITH (timeout_ms = N)` budget
tighter than the server's limit running out, …) and the engine's own status
otherwise — `500` for a storage, I/O or internal fault, `503` for a timeout
under the server's limit, the memory budget, an open circuit breaker or a
locked database — keeping the `VELESQL_*` code. A statement the database's
observer refuses (`Error::AccessDenied`, VELES-037) is the exception among
the client's errors: it is not malformed, so it answers `403`, as on every
other endpoint. A `$name` parameter the request does not bind in `params`, or
binds to a value its position cannot take (`Error::QueryParameter`,
VELES-038), answers `422` with `VELESQL_PARAMETER_ERROR` on every endpoint, in
place of the endpoint's own code, and names the parameter in
`details.parameter` (`{"parameter": "v"}`), so a client tells "supply the
parameter" from "fix the statement" without reading the message. `WITH
(timeout_ms = N)` can shorten the server's limit, never lengthen it, and `0`
keeps it.

`/collections/{name}/match` no longer emits the bespoke `*_ERROR`
strings (`COLLECTION_NOT_FOUND` / `PARSE_ERROR` / `EXECUTION_ERROR` / …).
It now returns the canonical `{ "error", "code" }` body with a
`VELES-XXX` code (e.g. `VELES-002` for a missing collection, `VELES-010`
for a parse error, `VELES-038` for an unbound or mistyped parameter,
`VELES-019` for a duplicate edge), and the HTTP status is derived from the error variant. Guard-rail
stops carry the `VELES-027` code, with the status of the limit crossed: the
rate limit `429`; a depth, cardinality or capacity ceiling `400`; the circuit
breaker, the memory budget and a timeout under the server's limit `503`. An
`ORDER BY` MATCH cannot sort by is refused before traversal with `VELES-010`
carrying `V015`, as `/query` refuses it with `VELESQL_VALIDATION_ERROR`. The
request's `vector` and every `similarity()` query vector of the clause are
checked against the collection before traversal too (core's
`velesql::QueryVectorTarget`, the check `/query` applies to every `NEAR`,
`NEAR_FUSED` and `similarity()` query vector of a statement): `VELES-015` for a
graph without embeddings, which stores no vectors, then `VELES-004` for another
dimension — a walk that would find no row does not turn either into an empty
result. A metadata collection is no `/match` target at all — it has no graph to
traverse — and answers `VELES-002` (`404`) before the query is read, as a
missing collection does.

Syntax errors still use parser-specific payload (`QueryErrorResponse` with `type/message/position/query`).

## Resource Limits (Guard-rails)

These limits are part of the runtime contract and apply to every VelesQL query:

| Limit | Value | Behavior on breach |
|-------|-------|--------------------|
| Query length | configurable `max_query_length` | Rejected **before parsing** (pre-scan), parse error. |
| Bracket / `NOT` nesting depth | 64 | Rejected **before parsing** (pre-scan), parse error. Prevents a parser stack-overflow DoS. |
| GROUP BY groups | server ceiling 1,000,000 (default 10,000) | `WITH (max_groups = N)` may lower the budget but `N` is clamped down to the ceiling; exceeding the effective budget returns an aggregation error. |
| `NOT similarity()` scan | 5,000,000 vectors | Rejected with guidance (no index acceleration for `NOT similarity()`). |

Both pre-scan rejections are evaluated before `pest` is invoked, so they fire
regardless of how the query nests.

## Syntax Profiles (Frozen)

These syntax profiles are frozen for this contract version:

- Top-level query: `SELECT ...` or `MATCH (...) ...`
- Hybrid WHERE predicate: `SELECT ... FROM <collection> WHERE ... AND MATCH (...)`
- Text predicate: `<field> MATCH 'text'` — holds for a row that is a BM25
  hit for the query, wherever the WHERE is evaluated (the column is not
  consulted). The one AND-required `MATCH` ranks a SELECT (alone by BM25;
  beside a `NEAR`, as a hybrid boost); any other is a membership test.

Reference grammar:

- `docs/VELESQL_SPEC.md` (canonical, v3.6)

## Stable vs Experimental

| Capability | Contract state | Runtime state |
|------------|----------------|---------------|
| `SELECT ... FROM ... WHERE ...` | Stable | Stable |
| `SELECT ... WHERE ... AND MATCH (...)` | Stable | Stable |
| top-level `MATCH (...) RETURN ...` | Stable | Stable |
| top-level `MATCH` via `/query` with body `collection` | Stable | Stable |
| aggregation via `/aggregate` | Stable | Stable |
| `JOIN ... ON` | Stable | Stable |
| `JOIN ... USING (...)` | Stable | Stable for single-column USING |
| `LEFT/RIGHT/FULL JOIN` | Stable | Stable |
| `GROUP BY` / `HAVING` | Stable | Stable |
| `UNION/INTERSECT/EXCEPT` | Stable | Stable |
| `CREATE COLLECTION` | Stable | Stable |
| `DROP COLLECTION [IF EXISTS]` | Stable | Stable |
| `CREATE INDEX ON` | Stable | Stable |
| `DROP INDEX ON` | Stable | Stable |
| `ANALYZE` | Stable | Stable |
| `TRUNCATE` | Stable | Stable |
| `ALTER COLLECTION ... SET` | Stable | Stable |
| `INSERT INTO` | Stable | Stable |
| `UPSERT INTO` | Stable | Stable |
| `UPDATE ... SET` | Stable | Stable |
| `INSERT EDGE INTO` | Stable | Stable |
| `DELETE FROM` | Stable | Stable |
| `DELETE EDGE ... FROM` | Stable | Stable |
| `SELECT EDGES FROM` | Stable | Stable |
| `INSERT NODE INTO` | Stable | Stable |
| `SHOW COLLECTIONS` | Stable | Stable |
| `DESCRIBE COLLECTION` | Stable | Stable |
| `EXPLAIN` | Stable | Stable |
| `FLUSH` | Stable | Stable |

## Validation Matrix

Contract test cases are listed in:

- `conformance/velesql_parser_cases.json` (parser conformance, v3.6)
- `conformance/velesql_contract_cases.json` (runtime contract)
- `conformance/velesql_executor_cases.json` (executor result rows/counts/ordering, goldens derived from `velesdb-core`)

Each invalid case maps to an expected HTTP status and an expected error shape.

## Feature Execution Status

| Feature | Parser | Executor |
|---------|--------|----------|
| `JOIN ... ON` | Supported | Supported (inner join) |
| `JOIN ... USING (...)` | Supported | Supported for single-column USING |
| `LEFT/RIGHT/FULL JOIN` | Supported | Supported |
| `GROUP BY`, `HAVING` | Supported | Supported |
| `ORDER BY similarity()` | Supported | Supported |
| `UNION/INTERSECT/EXCEPT` | Supported | Supported |
| `CREATE COLLECTION` | Supported | Supported |
| `DROP COLLECTION [IF EXISTS]` | Supported | Supported |
| `CREATE INDEX ON` | Supported | Supported |
| `DROP INDEX ON` | Supported | Supported |
| `ANALYZE` | Supported | Supported |
| `TRUNCATE` | Supported | Supported |
| `ALTER COLLECTION ... SET` | Supported | Supported |
| `INSERT INTO` | Supported | Supported |
| `UPSERT INTO` | Supported | Supported |
| `UPDATE ... SET` | Supported | Supported |
| `INSERT EDGE INTO` | Supported | Supported |
| `DELETE FROM` | Supported | Supported |
| `DELETE EDGE ... FROM` | Supported | Supported |
| `SELECT EDGES FROM` | Supported | Supported |
| `INSERT NODE INTO` | Supported | Supported |
| `SHOW COLLECTIONS` | Supported | Supported |
| `DESCRIBE COLLECTION` | Supported | Supported |
| `EXPLAIN` | Supported | Supported |
| `FLUSH` | Supported | Supported |

## Compatibility Notes

- Existing clients reading `timing_ms` + `rows_returned` continue to work.
- New clients should prefer `meta.velesql_contract_version` for contract-aware handling.
