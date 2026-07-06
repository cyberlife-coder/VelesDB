# VelesQL REST Contract

Canonical contract for VelesQL server endpoints and payloads.

- Contract version: `3.7.0`
- Last updated: `2026-07-06`

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
- Collection is resolved from `FROM <collection>` first, then optional body `collection`.

Success response shape:

```json
{
  "result": [{ "category": "tech", "count": 42 }],
  "timing_ms": 1.12,
  "meta": {
    "velesql_contract_version": "3.3.0",
    "count": 1
  }
}
```

Rules:

- `collection` is optional for `SELECT ... FROM <collection> ...`.
- `collection` is mandatory for top-level `MATCH (...) ...` sent to `/query`.
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
  sparse-only and `NOT similarity()` shapes). Residual shapes (predicates
  under `OR`/`NOT`, `similarity()` cascades, BM25 text-`MATCH` fusion, hybrid
  dense+sparse fusion) keep the windowed execution: a ranked fetch filters an
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
    "velesql_contract_version": "3.3.0",
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
    "velesql_contract_version": "3.3.0",
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
    "velesql_contract_version": "3.3.0"
  }
}
```

## Standard Error Model (VelesQL)

Semantic/runtime errors for VelesQL endpoints use:

```json
{
  "error": {
    "code": "VELESQL_MISSING_COLLECTION",
    "message": "MATCH query via /query requires `collection` in request body",
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
- `VELESQL_EXPLAIN_ANALYZE_ERROR`

`/collections/{name}/match` no longer emits the bespoke `*_ERROR`
strings (`COLLECTION_NOT_FOUND` / `PARSE_ERROR` / `EXECUTION_ERROR` / …).
It now returns the canonical `{ "error", "code" }` body with a
`VELES-XXX` code (e.g. `VELES-002` for a missing collection, `VELES-010`
for a parse error or unbound parameter, `VELES-019` for a duplicate
edge), and the HTTP status is derived from the error variant. Guard-rail
rejections (rate limit `429`, circuit breaker `503`) and query timeouts
(`408`) carry the `VELES-027` code.

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
- Text predicate: `<field> MATCH 'text'`

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
