---
phase: 2
plan: 1
completed: 2026-02-08
duration: ~3h
---

# Phase 2 Plan 1: Core Scalar Subquery Executor — Summary

## One-liner

Scalar subquery execution engine that runs inner SELECT statements and returns concrete values, with support for uncorrelated, correlated, and aggregation subqueries.

## What Was Built

Created the subquery execution engine (`subquery.rs`) that takes a `Subquery` AST node, executes the inner `SelectStatement` against the collection, and returns a scalar `Value`. This is the foundation that both MATCH WHERE and SELECT WHERE paths use (wired in Plans 02-02 and 02-03).

The executor handles two paths: aggregation subqueries (e.g., `SELECT AVG(price) FROM products`) that route through `execute_aggregate`, and regular SELECT subqueries that route through `execute_query` with LIMIT 1 optimization. Correlated subqueries inject outer row values into the params map using the `correlations` metadata from the parser.

A `resolve_subquery_value` helper provides the clean entry point for callers, and `resolve_subqueries_in_condition` recursively walks condition trees to pre-resolve all `Value::Subquery` nodes before stateless filter conversion.

## Tasks Completed

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | Write failing tests for scalar subquery execution (RED) | 10665464 | subquery_tests.rs |
| 2 | Implement `execute_scalar_subquery` on Collection | 10665464 | subquery.rs, mod.rs |
| 3 | Add `resolve_subquery_value` + `resolve_subqueries_in_condition` helpers | 10665464 | subquery.rs |

## Key Files

**Created:**
- `crates/velesdb-core/src/collection/search/query/subquery.rs` — Core executor (274 lines)
- `crates/velesdb-core/src/collection/search/query/subquery_tests.rs` — Tests (504 lines)

**Modified:**
- `crates/velesdb-core/src/collection/search/query/mod.rs` — Register `mod subquery` + `mod subquery_tests`

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Pre-resolution pattern (walk tree before filter conversion) | `From<Condition>` trait is stateless — cannot call Collection methods. Industry standard per Materialize/PostgreSQL. |
| Scalar subquery = first row, first column | SQL standard contract for scalar subqueries |
| NULL for empty results | SQL standard — explicit, not silent data loss |
| Correlated via param injection | Simple and correct for embedded DB. Decorrelation (join rewrite) is a future optimization. |
| LIMIT 1 cap for scalar SELECT subqueries | Optimization — only first row matters for scalar context |
| Aggregation vs SELECT routing | `execute_aggregate` returns JSON objects; regular `execute_query` returns SearchResults — different extraction logic needed |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Critical functionality] Added `resolve_subqueries_in_condition`**
- Found during: Task 3
- Issue: Plan only asked for `resolve_value` but the SELECT WHERE path needs recursive condition tree walking
- Fix: Added `resolve_subqueries_in_condition` that handles Comparison, In, Between, And, Or, Not, Group
- Files: `subquery.rs`

## Verification Results

```
cargo fmt --all --check           → Exit 0
cargo clippy -p velesdb-core -- -D warnings → 0 warnings
cargo test -p velesdb-core --lib subquery_tests → 11 passed, 0 failed
cargo deny check                  → advisories ok, bans ok, licenses ok, sources ok
```

## Research Validation

Web research confirms VelesDB's approach aligns with industry patterns:
- **Pre-resolution before stateless conversion** — Same pattern as Materialize, Alibaba Cloud optimizer
- **Nested loop for correlated scalars** — PostgreSQL uses same approach (per CYBERTEC blog)
- **Future optimization**: Decorrelation (convert correlated subqueries to joins) for performance at scale

## Next Phase Readiness

- Plan 02-02 can wire into MATCH WHERE path using `resolve_subquery_value`
- Plan 02-03 can wire into SELECT WHERE path using `resolve_subqueries_in_condition`

---
*Completed: 2026-02-08*
