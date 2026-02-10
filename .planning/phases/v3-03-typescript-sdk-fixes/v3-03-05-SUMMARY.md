---
phase: v3-03
plan: 05
completed: 2026-02-10
duration: ~10min
---

# Phase v3-03 Plan 05: SELECT Query Builder — Summary

## One-liner

Fluent `SelectBuilder` for VelesQL SELECT queries with 39 TDD tests — completes the query builder story alongside VelesQLBuilder (MATCH).

## What Was Built

A new `SelectBuilder` class providing type-safe construction of all VelesQL SELECT query variants. Follows the same immutable builder pattern as the existing `VelesQLBuilder`. The `build()` method returns `{ query, params }` for direct use with `db.query()`.

### Supported Clauses

| Clause | Methods |
|--------|---------|
| SELECT | `select()`, `selectAll()`, `selectAs()`, `selectAgg()` |
| FROM | `from()` |
| WHERE | `where()`, `andWhere()`, `orWhere()` |
| Vector | `nearVector()`, `similarity()` |
| JOIN | `join()` (INNER/LEFT/RIGHT) |
| GROUP BY | `groupBy()` |
| ORDER BY | `orderBy()` (multi-column, ASC/DESC) |
| LIMIT/OFFSET | `limit()`, `offset()` |

## Files

**Created:**
- `sdks/typescript/src/select-builder.ts` (248 lines) — SelectBuilder class + selectql() factory
- `sdks/typescript/tests/select-builder.test.ts` (319 lines) — 39 TDD tests

**Modified:**
- `sdks/typescript/src/index.ts` — Added `SelectBuilder` and `selectql` exports

## Verification Results

```
tsc --noEmit: 0 errors
vitest: 7 files, 204 tests passed (165 → 204, +39 new)
```

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Immutable builder (clone on every call) | Same pattern as VelesQLBuilder — predictable, composable |
| `build()` returns `{ query, params }` | Direct use with `db.query()` — no manual param extraction |
| `selectql()` factory function | Matches `velesql()` naming convention |
| Vector methods auto-set connector | First clause = no connector, subsequent = AND — natural flow |

---
*Completed: 2026-02-10T19:01*
