---
phase: 03-pq-integration
plan: 02
subsystem: velesql
tags: [pest, parser, ast, train-quantizer, velesql]

# Dependency graph
requires:
  - phase: 02-pq-core-engine
    provides: ProductQuantizer implementation that TRAIN QUANTIZER will train
provides:
  - TRAIN QUANTIZER grammar rule in VelesQL pest grammar
  - TrainStatement AST node with collection and params HashMap
  - parse_train_stmt parser implementation
  - Query.is_train() and Query.new_train() helpers
affects: [03-pq-integration]

# Tech tracking
tech-stack:
  added: []
  patterns: [train-stmt-dispatch-pattern, with-clause-to-hashmap-extraction]

key-files:
  created:
    - crates/velesdb-core/src/velesql/ast/train.rs
    - crates/velesdb-core/src/velesql/parser/train.rs
    - crates/velesdb-core/src/velesql/train_tests.rs
  modified:
    - crates/velesdb-core/src/velesql/grammar.pest
    - crates/velesdb-core/src/velesql/ast/mod.rs
    - crates/velesdb-core/src/velesql/parser/mod.rs
    - crates/velesdb-core/src/velesql/parser/select/mod.rs
    - crates/velesdb-core/src/velesql/mod.rs

key-decisions:
  - "Reused existing with_clause grammar rule for TRAIN params (no new grammar for key=value pairs)"
  - "WITH params extracted into HashMap<String, WithValue> for flexible runtime param access"
  - "Combined Task 1+2 into single commit (grammar+AST+parser tightly coupled for TDD cycle)"

patterns-established:
  - "TRAIN statement pattern: grammar rule -> AST struct -> parser -> dispatch in parse_query"
  - "Extension point: new statement types follow train_stmt pattern (grammar alt + AST + parser module)"

requirements-completed: [PQ-05]

# Metrics
duration: 19min
completed: 2026-03-06
---

# Phase 3 Plan 02: VelesQL TRAIN QUANTIZER Parse Layer Summary

**TRAIN QUANTIZER ON <collection> WITH (m=8, k=256) grammar, AST, parser with 13 tests covering positive/negative/edge cases**

## Performance

- **Duration:** 19 min
- **Started:** 2026-03-06T14:35:56Z
- **Completed:** 2026-03-06T14:55:10Z
- **Tasks:** 2
- **Files modified:** 17

## Accomplishments
- VelesQL grammar extended with `train_stmt` rule accepting TRAIN QUANTIZER ON <collection> WITH (params)
- TrainStatement AST struct storing collection name and params as HashMap<String, WithValue>
- Full parse pipeline from string to typed AST via parse_train_stmt
- 13 tests: basic parsing, all param types, boolean params, case-insensitivity, semicolons, missing-keyword errors, is_train/is_select checks

## Task Commits

Both tasks combined into a single atomic commit (grammar + AST + parser + dispatch + tests):

1. **Task 1+2: Grammar, AST, parser, dispatch, tests** - `0b2122dd` (feat)

**Plan metadata:** pending

## Files Created/Modified
- `crates/velesdb-core/src/velesql/grammar.pest` - Added train_stmt rule and query alternative
- `crates/velesdb-core/src/velesql/ast/train.rs` - TrainStatement struct (collection + params)
- `crates/velesdb-core/src/velesql/ast/mod.rs` - Query.train field, is_train(), new_train()
- `crates/velesdb-core/src/velesql/parser/train.rs` - parse_train_stmt implementation
- `crates/velesdb-core/src/velesql/parser/mod.rs` - Added train module declaration
- `crates/velesdb-core/src/velesql/parser/select/mod.rs` - Rule::train_stmt dispatch arm
- `crates/velesdb-core/src/velesql/mod.rs` - TrainStatement re-export + train_tests module
- `crates/velesdb-core/src/velesql/train_tests.rs` - 13 parser tests
- `crates/velesdb-core/src/velesql/parser/select/clause_compound.rs` - train: None in Query struct
- `crates/velesdb-core/src/velesql/ast_tests.rs` - train: None in Query struct
- `crates/velesdb-core/src/velesql/validation_tests.rs` - train: None in Query struct
- `crates/velesdb-core/src/velesql/validation_parity_tests.rs` - train: None in Query struct

## Decisions Made
- Reused existing `with_clause` grammar rule for TRAIN params instead of defining new grammar -- identical syntax, no duplication
- WITH params extracted to HashMap<String, WithValue> for flexible runtime access by executor
- Combined Task 1 and Task 2 into single commit since grammar/AST/parser/dispatch form an atomic unit for the TDD cycle

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed pre-existing clippy errors in config.rs**
- **Found during:** Task 1 (commit attempt)
- **Issue:** Pre-existing uncommitted changes from plan 03-01 in config.rs had clippy errors (unnecessary_wraps on default_oversampling, derivable_impls on QuantizationType Default)
- **Fix:** Added #[allow(clippy::unnecessary_wraps)] on default_oversampling, derived Default with #[default] on None variant
- **Files modified:** crates/velesdb-core/src/config.rs
- **Verification:** cargo clippy passes clean
- **Committed in:** 0b2122dd (part of task commit)

**2. [Rule 3 - Blocking] Included pre-existing 03-01 config/error/lib changes**
- **Found during:** Task 1 (commit attempt)
- **Issue:** Pre-existing uncommitted changes to config.rs, config_tests.rs, error.rs, error_tests.rs, lib.rs from plan 03-01 were in working tree, blocking clippy in pre-commit hook
- **Fix:** Included these files in the commit to maintain a clean workspace state
- **Files modified:** config.rs, config_tests.rs, error.rs, error_tests.rs, lib.rs
- **Verification:** Full test suite (2699 tests) passes
- **Committed in:** 0b2122dd

---

**Total deviations:** 2 auto-fixed (2 blocking)
**Impact on plan:** Both fixes were necessary to commit. Pre-existing 03-01 changes were in the working tree and blocking the pre-commit hook. No scope creep.

## Issues Encountered
- Git stash/pop operation reverted our edits to existing files (only new untracked files survived). Required re-applying all modifications. Root cause: stash pop conflicted with pre-existing config.rs changes.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- TRAIN QUANTIZER parse layer complete, ready for Plan 03 (executor wiring)
- Plan 03 will wire parse_train_stmt output to the actual ProductQuantizer::train() call
- TrainStatement.params HashMap provides all needed config (m, k, type, oversampling, force)

---
*Phase: 03-pq-integration*
*Completed: 2026-03-06*
