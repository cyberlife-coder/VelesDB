# VelesQL REST Contract

Canonical contract for VelesQL server endpoints and payloads.

- Contract version: `2.1.0`
- Last updated: `2026-02-18`

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

Rules:

- `collection` is optional for `SELECT ... FROM <collection> ...`.
- `collection` is mandatory for top-level `MATCH (...) ...` sent to `/query`.
- For graph-only execution, `/collections/{name}/match` remains supported.

Success response shape:

```json
{
  "results": [{ "id": 1, "score": 0.98, "payload": { "title": "Doc" } }],
  "timing_ms": 1.42,
  "took_ms": 1,
  "rows_returned": 1,
  "meta": {
    "velesql_contract_version": "2.1.0",
    "count": 1
  }
}
```

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
    "velesql_contract_version": "2.1.0"
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

`/collections/{name}/match` keeps compatibility fields (`error`, `code`) and now also returns `hint` and optional `details`.

Syntax errors still use parser-specific payload (`QueryErrorResponse` with `type/message/position/query`).

## Feature Execution Status

| Feature | Parser | Executor |
|---------|--------|----------|
| `JOIN ... ON` | Supported | Supported (inner join) |
| `JOIN ... USING (...)` | Supported | Not supported |
| `LEFT/RIGHT/FULL JOIN` | Parsed in spec variants | Not supported in runtime |
| `GROUP BY`, `HAVING` | Supported | Supported |
| `ORDER BY similarity()` | Supported | Supported |
| `UNION/INTERSECT/EXCEPT` | Supported | Supported |

## Compatibility Notes

- Existing clients reading `timing_ms` + `rows_returned` continue to work.
- New clients should prefer `meta.velesql_contract_version` for contract-aware handling.
