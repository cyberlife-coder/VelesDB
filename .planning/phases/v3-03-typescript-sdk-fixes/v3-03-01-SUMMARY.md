---
phase: v3-03
plan: 01
completed: 2026-02-09
duration: ~15min
---

# Phase v3-03 Plan 01: REST Backend Contract, MATCH Support & Safety Fixes — Summary

## One-liner

Fix all REST backend response contract mismatches, add `matchQuery()` for MATCH endpoint, implement smart query routing with aggregation handling, and eliminate all type safety issues.

## What Was Built

The TypeScript SDK REST backend (`rest.ts`) had several response contract mismatches with the VelesDB server: `search()` expected a flat array but the server wraps results in `{ results: [...] }`, `listCollections()` expected `Collection[]` directly but the server returns `{ collections: [...] }` with snake_case fields, and `delete()` defaulted to `false` on empty HTTP success bodies instead of `true`.

A critical gap was closed: the server exposes `POST /collections/{name}/match` for MATCH graph traversal queries, but the SDK had no method to call it. The `VelesQLBuilder` could generate MATCH syntax but its output was unusable. Now `matchQuery()` provides direct access, and `query()` automatically detects MATCH queries and routes them to the correct endpoint.

Additionally, `query()` now handles aggregation responses (server returns `{ result: ... }` singular vs `{ results: [...] }` plural), preventing silent empty results on aggregate queries. A promise-based init lock prevents concurrent callers from firing duplicate health checks. All `as any` casts and unsafe `!` non-null assertions were eliminated.

## Tasks Completed

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | Fix response unwrapping (T-01, T-02, delete default) | `0b21d4e9` | `rest.ts`, `rest-backend.test.ts` |
| 2 | Promise-based init lock (BEG-07) + query() JSDoc (T-03) | `691e6a0d` | `rest.ts`, `rest-backend.test.ts` |
| 3 | Add matchQuery() — MATCH endpoint support | `44209ea8` | `rest.ts`, `rest-backend.test.ts` |
| 4 | Smart MATCH routing in query() + aggregation handling | `12bcbddb` | `rest.ts`, `rest-backend.test.ts`, `types.ts` |
| 5 | Type safety sweep | `549ac24b` | `rest.ts` |

## Key Files

**Modified:**
- `sdks/typescript/src/backends/rest.ts` — Response unwrapping, init lock, matchQuery(), smart routing, aggregation handling, type safety
- `sdks/typescript/tests/rest-backend.test.ts` — 9 new tests (31 total, up from 22)
- `sdks/typescript/src/types.ts` — QueryOptions extended with vector/threshold for MATCH pass-through

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| MATCH types defined locally in `rest.ts` | Plan 02 owns `types.ts` exports; local server-contract interfaces avoid conflicts |
| Aggregation detected via `'result' in rawData` | Server uses singular `result` for aggregations vs plural `results` for SELECT |
| `as ServerAggregationResponse` cast kept despite SonarQube warning | TypeScript `in` operator doesn't fully discriminate union types; cast is necessary |
| `rest.ts` at 856 lines — not refactored | Out of scope for this plan; noted for future extraction |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Critical] Missing DistanceMetric/StorageMode imports**
- Found during: Task 1
- Issue: `listCollections()` field mapping used `as DistanceMetric` / `as StorageMode` but types weren't imported
- Fix: Added `DistanceMetric` and `StorageMode` to the type import block
- Files: `rest.ts`
- Commit: `0b21d4e9`

**2. [Rule 2 - Critical] Float32Array handling in matchQuery()**
- Found during: Task 3
- Issue: `options.vector` could be `Float32Array` which doesn't serialize to JSON correctly
- Fix: Added `instanceof Float32Array ? Array.from(...) : ...` conversion
- Files: `rest.ts`
- Commit: `44209ea8`

## Verification Results

```
$ npx tsc --noEmit
(exit 0 — zero type errors)

$ npx vitest run
Test Files  6 passed (6)
     Tests  151 passed (151)
  Duration  351ms
```

## Next Phase Readiness

- Plan 02 (types.ts exports + VelesQLBuilder MATCH methods) can proceed — no conflicts
- Plan 03 (test coverage) can build on the 9 new tests added here
- `rest.ts` at 856 lines should be considered for extraction in a future plan

---
*Completed: 2026-02-09 21:55 UTC+1*
