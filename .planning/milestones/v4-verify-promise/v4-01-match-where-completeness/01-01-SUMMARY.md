---
phase: v4-01
plan: 1
completed: 2026-02-08
duration: ~25min
---

# Phase v4-01 Plan 01: Wire ORDER BY into MATCH Execution Pipeline — Summary

## One-liner

Wire existing `order_match_results` into all MATCH execution paths so ORDER BY clauses are actually applied, with explicit ORDER BY taking precedence over default similarity sort.

## What Was Built

The `order_match_results` method existed in `similarity.rs` with unit tests passing, but was never called from the MATCH execution pipeline — making ORDER BY a silent no-op for all MATCH queries. This plan wired it in.

A new `apply_match_order_by` helper iterates over `ReturnClause.order_by` items in reverse order (SQL multi-column sort semantics via stable sort) and delegates to `order_match_results` for each item. This helper is called from all 3 return paths in `execute_match` (no-relationship, multi-hop, single-hop) after aggregation but before returning results. For `execute_match_with_similarity`, explicit ORDER BY now takes precedence over the default metric-aware similarity sort.

An E2E test (`test_bs4_order_by_timestamp`) verifies that ORDER BY on an intermediate node's projected property (`conv.timestamp DESC`) correctly reorders multi-hop MATCH results across 4 distinct timestamp values.

## Tasks Completed

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | Add `apply_match_order_by` helper | `4dcd275a` | `similarity.rs` |
| 2 | Wire ORDER BY into `execute_match` (3 paths) | `4dcd275a` | `mod.rs` |
| 3 | Fix ORDER BY in `execute_match_with_similarity` | `4dcd275a` | `similarity.rs` |
| 4 | Add E2E test + update `test_bs4_agent_memory` | `4dcd275a` | `match_simple.rs` |

## Key Files

**Modified:**
- `crates/velesdb-core/src/collection/search/query/match_exec/similarity.rs` — Added `apply_match_order_by` helper, changed similarity sort to respect explicit ORDER BY
- `crates/velesdb-core/src/collection/search/query/match_exec/mod.rs` — Wired `apply_match_order_by` into all 3 return paths
- `crates/velesdb-core/tests/readme_scenarios/match_simple.rs` — Added `test_bs4_order_by_timestamp`, updated `test_bs4_agent_memory` to use ORDER BY

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Single commit for all 4 tasks | Pre-commit hook rejects dead code, so helper + wiring must be atomic |
| `&mut [MatchResult]` not `&mut Vec` | Clippy `ptr_arg` lint — slice is sufficient |
| Explicit ORDER BY overrides default similarity sort | User intent should always take precedence over implicit behavior |
| Reverse iteration for multi-column ORDER BY | SQL semantics: last ORDER BY item = primary sort key with stable sort |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Clippy `ptr_arg` lint on `apply_match_order_by` signature**
- Found during: Task 1
- Issue: `&mut Vec<MatchResult>` triggers clippy `ptr_arg` warning
- Fix: Changed to `&mut [MatchResult]`
- Files: `similarity.rs`
- Commit: `4dcd275a`

**2. [Rule 3 - Blocker] Multi-hop path `results` not declared `mut`**
- Found during: Task 2
- Issue: `let results = self.execute_multi_hop_chain(...)` needed `let mut` for `&mut results`
- Fix: Changed to `let mut results`
- Files: `mod.rs`
- Commit: `4dcd275a`

**3. [Rule 1 - Bug] `cargo fmt` reformatted closure in test**
- Found during: Task 4
- Issue: Multi-line closure formatted as single line by rustfmt
- Fix: Ran `cargo fmt --all`
- Files: `match_simple.rs`
- Commit: `4dcd275a`

## Verification Results

```
cargo test --test readme_scenarios match_simple -- 7 passed, 0 failed
cargo test --test readme_scenarios hero_query -- 4 passed, 0 failed
cargo clippy -p velesdb-core -- -D warnings — clean
Pre-commit hook: fmt ✅, clippy ✅, tests ✅ (all workspace), secrets ✅
```

## Next Phase Readiness

- VP-006 (ORDER BY in MATCH) is now fully wired — `order_match_results` is live code
- Plan 01-02 (Temporal resolution in MATCH WHERE, VP-003) can proceed in parallel
- Phase 4 E2E tests (04-05, 04-06) that depend on ORDER BY in MATCH are unblocked

---
*Completed: 2026-02-08T21:30+01:00*
