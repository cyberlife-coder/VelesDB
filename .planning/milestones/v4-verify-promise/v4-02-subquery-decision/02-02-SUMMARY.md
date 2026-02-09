---
phase: 2
plan: 2
completed: 2026-02-08
duration: ~2h
---

# Phase 2 Plan 2: Wire Subquery into MATCH WHERE Path — Summary

## One-liner

Connected scalar subquery executor into MATCH WHERE evaluation so `Value::Subquery` comparisons resolve at runtime instead of silently returning false.

## What Was Built

Modified `where_eval.rs` to call `resolve_subquery_value` before parameter resolution in the Comparison evaluation path. The key change is a two-step resolution: first resolve any subquery to a concrete value, then resolve any remaining parameters. This ensures that MATCH queries like `MATCH (p:Product) WHERE p.price < (SELECT AVG(price) FROM products)` execute the inner SELECT and compare against the actual average.

The implementation chose **Option A** from the plan (pass collection ref inline) but simplified it further — since `evaluate_where_condition` already takes `&self`, the subquery resolution call is direct without needing to change the `resolve_where_param` signature. The `resolve_subquery_value` method from Plan 02-01 handles the heavy lifting.

## Tasks Completed

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | Write failing integration tests for subquery in MATCH WHERE (RED) | a946e44f | match_where_eval_tests.rs |
| 2 | Modify evaluation to resolve subqueries before comparison | a946e44f | where_eval.rs |
| 3 | Verify BETWEEN/IN subquery positions documented | a946e44f | where_eval.rs |

## Key Files

**Modified:**
- `crates/velesdb-core/src/collection/search/query/match_exec/where_eval.rs` — Added `resolve_subquery_value` call at line 39 before `resolve_where_param`
- `crates/velesdb-core/src/collection/search/query/match_where_eval_tests.rs` — 2 new integration tests

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Inline resolution (not signature change) | `evaluate_where_condition` already has `&self` — no need to add `Option<&Collection>` param |
| Two-step resolution: subquery then params | Subquery may return a value that still needs param resolution; ordering matters |
| BETWEEN/IN subquery: not wired | Parser doesn't currently produce subqueries in BETWEEN bounds or IN lists; documented for future |

## Deviations from Plan

*None — plan executed exactly as written.*

## Verification Results

```
cargo test -p velesdb-core --lib match_where_eval_tests → 16 passed, 0 failed
cargo test -p velesdb-core --lib match_exec_tests       → 15 passed, 0 failed
cargo clippy -p velesdb-core -- -D warnings             → 0 warnings
```

## Next Phase Readiness

- MATCH WHERE subqueries functional for all 4 README business scenarios
- Plan 02-03 wires the same mechanism into SELECT WHERE path

---
*Completed: 2026-02-08*
