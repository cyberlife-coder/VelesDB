# VelesQL Conformance Cases (Contract v2.1.0)

Last updated: 2026-02-18

Canonical reference cases for parser + REST contract assertions.

## Valid Cases

| ID | Query / Request | Expected |
|----|------------------|----------|
| V001 | `SELECT * FROM docs LIMIT 5` | `200`, `results[]`, `meta.velesql_contract_version` |
| V002 | `SELECT * FROM docs WHERE vector NEAR $v LIMIT 10` with params | `200`, search results |
| V003 | `SELECT * FROM docs WHERE category = 'tech' AND MATCH (d:Doc)-[:REL]->(x) LIMIT 10` | `200`, filtered results |
| V004 | `/query` with `MATCH (d:Doc) RETURN d LIMIT 1` + body `collection` | `200` |
| V005 | `/collections/{name}/match` with `MATCH (a)-[:REL]->(b) RETURN a,b` | `200`, `count`, `meta.velesql_contract_version` |

## Invalid Cases

| ID | Query / Request | Expected status | Expected error |
|----|------------------|----------------|----------------|
| I001 | `/query` with top-level `MATCH (...)` and missing body `collection` | `422` | `error.code = VELESQL_MISSING_COLLECTION` |
| I002 | `/query` with unknown collection in `FROM` | `404` | `error.code = VELESQL_COLLECTION_NOT_FOUND` |
| I003 | `/query` with parse error (`SELEC * FROM docs`) | `400` | parser payload (`error.type/message/position/query`) |
| I004 | `/query` with semantic/runtime mismatch | `422` | `error.code = VELESQL_EXECUTION_ERROR` |
| I005 | `/query` invalid aggregation | `422` | `error.code = VELESQL_AGGREGATION_ERROR` |

## Error Shape Assertions

For standardized VelesQL semantic/runtime errors:

```json
{
  "error": {
    "code": "VELESQL_*",
    "message": "human-readable message",
    "hint": "actionable hint",
    "details": {}
  }
}
```

For parser syntax errors:

```json
{
  "error": {
    "type": "SyntaxError",
    "message": "parse details",
    "position": 5,
    "query": "SELEC * FROM ..."
  }
}
```
