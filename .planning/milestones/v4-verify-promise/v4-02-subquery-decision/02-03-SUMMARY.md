---
phase: 2
plan: 3
completed: 2026-02-08
duration: ~2h
---

# Phase 2 Plan 3: Wire Subquery into SELECT WHERE Path + Quality Gates — Summary

## One-liner

Connected subquery pre-resolution into `execute_query` SELECT path and added defense-in-depth warnings in filter conversion, with full quality gate validation.

## What Was Built

Added the `resolve_subqueries_in_condition` call into `execute_query()` (mod.rs line 132-137) that runs after WHERE clause extraction but before any filter conversion. This ensures all `Value::Subquery` nodes are replaced with concrete scalar values before the stateless `From<velesql::Condition> for filter::Condition` trait runs.

Also added defense-in-depth in `filter/conversion.rs` — any `Value::Subquery` that somehow reaches conversion (meaning `resolve_subqueries_in_condition` was not called) now logs a `tracing::warn!` before falling back to `Value::Null`. This replaces the previous silent `Value::Null` conversion that was the root cause of VP-002.

Two integration tests verify the end-to-end SELECT + subquery flow: one for successful filtering (`price < AVG(price)` returning 2 of 5 rows) and one for NULL subquery result (0 rows returned, not all rows).

## Tasks Completed

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | Write failing SELECT WHERE + subquery tests (RED) | 7e87d446 | subquery_tests.rs |
| 2 | Implement `resolve_subqueries_in_condition` pre-resolution step | 7e87d446 | subquery.rs |
| 3 | Wire pre-resolution into `execute_query` | 7e87d446 | mod.rs |
| 4 | Defense-in-depth in filter conversion + full quality gates | 7e87d446 | conversion.rs |

## Key Files

**Modified:**
- `crates/velesdb-core/src/collection/search/query/mod.rs` — Pre-resolution step at line 132-137
- `crates/velesdb-core/src/collection/search/query/subquery.rs` — `resolve_subqueries_in_condition` method
- `crates/velesdb-core/src/collection/search/query/subquery_tests.rs` — 2 new SELECT integration tests
- `crates/velesdb-core/src/filter/conversion.rs` — Defense-in-depth `tracing::warn!` for unresolved subqueries (Comparison + IN positions)

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Pre-resolve before filter conversion | `From` trait is stateless — cannot execute subqueries. Must resolve first. |
| Defense-in-depth warn (not error) | Graceful degradation: log warning + return Null rather than panic |
| Two conversion.rs locations patched | Both Comparison value and IN list values can theoretically contain subqueries |

## Deviations from Plan

*None — plan executed exactly as written.*

## Verification Results

```
cargo fmt --all --check           → Exit 0
cargo clippy -p velesdb-core -- -D warnings → 0 warnings
cargo test -p velesdb-core --lib  → 2489 passed, 0 failed
cargo deny check                  → advisories ok, bans ok, licenses ok, sources ok
```

## Phase 2 Complete

All 3 plans executed successfully:
- **02-01**: Core scalar subquery executor (foundation)
- **02-02**: Wired into MATCH WHERE path
- **02-03**: Wired into SELECT WHERE path + quality gates

**Total tests added**: 13 (10 in subquery_tests + 2 in match_where_eval_tests + parser tests)
**Total commits**: 4 (3 feature + 1 docs)
**Quality gates**: 5/5 passing

---
*Completed: 2026-02-08*
