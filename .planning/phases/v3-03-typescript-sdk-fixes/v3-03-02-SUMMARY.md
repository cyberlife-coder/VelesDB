---
phase: v3-03
plan: 02
title: Types Export, Backend Interface & VelesQLBuilder MATCH
status: completed
started: 2026-02-10
completed: 2026-02-10
---

# Phase v3-03 Plan 02 — Summary

## Objective

Move MATCH types from local `rest.ts` definitions to shared `types.ts`, add `matchQuery()` to the backend interface and client, and enhance `VelesQLBuilder` with similarity convenience methods.

## Commits

| # | Hash | Description |
|---|------|-------------|
| 1 | `43f30dec` | Move MATCH types from rest.ts to shared types.ts |
| 2 | `12c61ff2` | Add matchQuery() to IVelesDBBackend + VelesDB client |
| 3 | `4d3a3857` | Add similarity() and orderBySimilarity() to VelesQLBuilder |
| 4 | `828bbe60` | Export SimilarityOptions from index.ts |

## Changes

### types.ts
- **Added** `MatchQueryOptions`, `MatchQueryResultItem`, `MatchQueryResponse` (moved from rest.ts)
- **Added** `matchQuery()` to `IVelesDBBackend` interface

### backends/rest.ts
- **Removed** local MATCH type definitions (now imported from types.ts)
- `matchQuery()` already implemented in Plan 01 — now satisfies interface contract

### backends/wasm.ts
- **Added** `matchQuery()` stub (throws `NOT_SUPPORTED`)

### client.ts
- **Added** `matchQuery()` with validation, JSDoc, and code examples

### query-builder.ts
- **Added** `SimilarityOptions` interface (threshold, custom field name)
- **Added** `similarity(alias, param, vector, options?)` — generates `WHERE similarity()` clause
- **Added** `orderBySimilarity(direction?)` — shorthand for ORDER BY similarity()

### index.ts
- **Added** `SimilarityOptions` to type exports

## Test Results

- **158 tests pass** (6 test files), up from 151
- **7 new tests**: similarity() (5) + orderBySimilarity() (2)
- **0 type errors** (`tsc --noEmit` clean)

## Findings Addressed

| Finding | Status | Action |
|---------|--------|--------|
| T-03 | ✅ Resolved | `query()` collection param documented (Plan 01 JSDoc); `matchQuery()` now uses collection in URL properly; types promoted to shared exports |

## Deferred / Future Work

| Item | Reason |
|------|--------|
| `rest.ts` file splitting (960 lines) | Separate refactor plan — working correctly as-is |
| `client.ts` file splitting (720 lines) | Same — functional, can be split in future cleanup phase |

## Phase v3-03 Status

| Plan | Status | Scope |
|------|--------|-------|
| Plan 01 | ✅ Complete | REST backend fixes (T-01, T-02, BEG-07) |
| Plan 02 | ✅ Complete | Types export, interface, VelesQLBuilder |

**Phase v3-03 is COMPLETE.** All TypeScript SDK findings (T-01, T-02, T-03, BEG-07) are addressed.
