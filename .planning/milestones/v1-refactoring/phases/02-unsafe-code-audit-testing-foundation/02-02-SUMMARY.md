---
phase: 02-unsafe-code-audit-testing-foundation
plan: 02
subsystem: testing
tags: [velesql, parser, regression-tests, clippy]

# Dependency graph
requires:
  - phase: 01-foundation-fixes
    provides: clippy baseline and parser safety/comment conventions
provides:
  - assertion-backed parser regressions for aggregate wildcard, HAVING operators, and correlated-subquery handling
  - removal of stale BUG markers in targeted parser hotspots
affects: [02-03-PLAN, phase-03-parser-modularization]

# Tech tracking
tech-stack:
  added: []
  patterns: ["Parser hotspot fixes are paired with exact named regression tests"]

key-files:
  created: []
  modified:
    - crates/velesdb-core/src/velesql/parser/select.rs
    - crates/velesdb-core/src/velesql/parser/values.rs
    - crates/velesdb-core/src/velesql/pr_review_bugfix_tests.rs

key-decisions:
  - "Keep BUG-02 scope narrow: only comments adjacent to targeted BUG-03 parser sites were updated"
  - "Correlated-subquery regression uses quoted dotted identifiers to exercise extraction and dedup behavior"

patterns-established:
  - "Replace historical bug labels with behavior/invariant comments"
  - "Use fully qualified test names in targeted parser validation commands"

# Metrics
duration: 9 min
completed: 2026-02-06
---

# Phase 2 Plan 02: Parser Fragility Hotspots Summary

**VelesQL parser wildcard aggregate rules, HAVING logical operators, and correlated-subquery extraction are now protected by direct assertion regressions at the targeted fragile sites.**

## Performance

- **Duration:** 9 min
- **Started:** 2026-02-06T20:58:35Z
- **Completed:** 2026-02-06T21:08:19Z
- **Tasks:** 3
- **Files modified:** 3

## Accomplishments
- Removed targeted `BUG-` markers from `select.rs` and `values.rs` while retaining behavior-focused invariants.
- Expanded `pr_review_bugfix_tests.rs` with explicit operator assertions (`AND`/`OR`) and correlated-subquery dedup/string-literal guards.
- Passed parser-focused validation: `pr_review_bugfix_tests`, `parser::`, and `cargo clippy -p velesdb-core -- -D warnings`.

## Task Commits

Each task was committed atomically:

1. **Task 1: Fix targeted parser BUG sites with explicit scope boundary** - `16f3231a` (fix)
2. **Task 2: Add assertion-style regressions for each targeted hotspot** - `125f2f51` (test)
3. **Task 3: Run parser regression + lint quality gates** - No code changes (verification-only task)

## Files Created/Modified
- `crates/velesdb-core/src/velesql/parser/select.rs` - Replaced stale bug-label comments with current aggregate/HAVING invariants.
- `crates/velesdb-core/src/velesql/parser/values.rs` - Replaced stale bug-label comments with correlation behavior notes.
- `crates/velesdb-core/src/velesql/pr_review_bugfix_tests.rs` - Added stronger assertion regressions for parser hotspots.

## Decisions Made
- Scoped BUG-02 strictly to comments adjacent to edited BUG-03 parser logic in `select.rs` and `values.rs`.
- Used quoted dotted identifiers in subquery regressions to exercise correlation extraction/dedup through the current grammar.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
- Initial task verification command filters matched no tests when using unqualified test names; resolved by running fully qualified test paths (no behavior change).

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Parser fragility hotspots targeted in this plan are covered and stable.
- Ready for `02-03-PLAN.md` (SIMD property-based test foundation).

---
*Phase: 02-unsafe-code-audit-testing-foundation*
*Completed: 2026-02-06*

## Self-Check: PASSED
