# README vs Runtime Contract Matrix

Source of truth for live server routes: `crates/velesdb-server/src/main.rs`.

Last verification: 2026-02-20.

## Route Coverage Matrix

| Route | Runtime | README |
|---|---|---|
| `/health` | yes | yes |
| `/collections` | yes | yes |
| `/collections/{name}` | yes | yes |
| `/collections/{name}/empty` | yes | yes |
| `/collections/{name}/flush` | yes | yes |
| `/collections/{name}/points` | yes | yes |
| `/collections/{name}/points/{id}` | yes | yes |
| `/collections/{name}/search` | yes | yes |
| `/collections/{name}/search/batch` | yes | yes |
| `/collections/{name}/search/multi` | yes | yes |
| `/collections/{name}/search/text` | yes | yes |
| `/collections/{name}/search/hybrid` | yes | yes |
| `/collections/{name}/indexes` | yes | yes |
| `/collections/{name}/indexes/{label}/{property}` | yes | yes |
| `/query` | yes | yes |
| `/query/explain` | yes | yes |
| `/collections/{name}/match` | yes | yes |
| `/collections/{name}/graph/edges` | yes | yes |
| `/collections/{name}/graph/traverse` | yes | yes |
| `/collections/{name}/graph/nodes/{node_id}/degree` | yes | yes |
| `/metrics` | feature-gated (`prometheus`) | yes (feature-gated section) |

## Contract Notes

- Top-level `MATCH` via `/query` requires `collection` in request body.
- Top-level `MATCH` is supported by `Collection::execute_query`; `Database::execute_query` rejects top-level `MATCH` by design.
- `JOIN` runtime supports `INNER`, `LEFT`, `RIGHT`, and `FULL`.
- `LEFT/RIGHT/FULL` JOIN paths are covered by the core runtime executor.
- `USING` currently supports a single column only.
