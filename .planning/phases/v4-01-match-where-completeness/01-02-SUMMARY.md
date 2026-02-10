---
phase: v4-01
plan: 2
completed: 2026-02-08
duration: ~15min
---

# Phase v4-01 Plan 02: Wire Temporal Resolution into MATCH WHERE Comparison — Summary

## One-liner

Fix `Value::Temporal` silent rejection in MATCH WHERE by resolving temporal expressions to epoch seconds before comparison, matching the SELECT path behavior.

## What Was Built

Added a `Value::Temporal` arm to `resolve_where_param` in `where_eval.rs` that converts temporal expressions (`NOW()`, `NOW() - INTERVAL '7 days'`, etc.) to `Value::Integer(epoch_seconds)` before comparison. This mirrors the existing SELECT path in `filter/conversion.rs:19-21` which already performed this conversion.

Previously, `resolve_where_param` had only a `Value::Parameter` arm and a catch-all `other => Ok(other.clone())`, meaning `Value::Temporal` was passed through unchanged. Then `evaluate_comparison` hit its `_ => Ok(false)` for `(Number, Temporal)` pairs — **silently rejecting all temporal comparisons in MATCH WHERE**.

Two new tests prove the fix works: `NOW()` resolves to a current timestamp (all test nodes in the past → 0 results), and `NOW() - INTERVAL '999999 days'` resolves to an ancient date (all 4 test nodes match). The binding-aware path (`evaluate_where_with_bindings`) was confirmed to already call `resolve_where_param` at line 361, so Task 1's fix automatically covers multi-hop MATCH queries too.

## Tasks Completed

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | Resolve Value::Temporal in resolve_where_param | 4d06fbfd | where_eval.rs |
| 2 | Add unit tests for temporal comparison | 4d06fbfd | match_where_eval_tests.rs |
| 3 | Verify binding-aware path (no code change) | 4d06fbfd | where_eval.rs (read-only) |

## Key Files

**Modified:**
- `crates/velesdb-core/src/collection/search/query/match_exec/where_eval.rs` — Added `Value::Temporal(t) => Ok(Value::Integer(t.to_epoch_seconds()))` arm in `resolve_where_param`
- `crates/velesdb-core/src/collection/search/query/match_where_eval_tests.rs` — Added 2 temporal resolution tests + imports

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Convert to `Value::Integer` (not `Value::Float`) | Epoch seconds are integers; matches SELECT path behavior in `filter/conversion.rs` |
| Single fix point in `resolve_where_param` | Both `evaluate_where_condition` and `evaluate_where_with_bindings` call this function, so one fix covers both paths |

## Deviations from Plan

None — plan executed exactly as written.

## Verification Results

```
cargo test -p velesdb-core --lib -- match_where_eval_tests
  19 passed; 0 failed (includes 3 temporal tests)

cargo test --test readme_scenarios match_simple
  7 passed; 0 failed

cargo clippy -p velesdb-core -- -D warnings
  0 warnings (clean)
```

## Next Phase Readiness

- **Phase 1 complete** — Both plans (01-01 ORDER BY, 01-02 Temporal) are done
- VP-001 ✅, VP-003 ✅, VP-006 ✅ — All Phase 1 requirements resolved
- Phase 4 Wave 2 continues (plans 04-05, 04-06 pending)

---
*Completed: 2026-02-08T21:34+01:00*
