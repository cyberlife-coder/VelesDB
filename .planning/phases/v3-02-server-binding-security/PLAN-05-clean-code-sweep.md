---
phase: v3-02-server-binding-security
plan: 05
name: clean-code-sweep
wave: 2
depends_on: [01, 03]
autonomous: true
parallel_safe: true
---

# Plan 05: Clean Code Sweep — Clippy Allows, Tests, Refactoring

## Objective

Remove blanket clippy allows from `lib.rs`, extract inline tests to separate files, fix dead code annotations, and apply craftsman-level code quality throughout the server crate. This plan addresses code quality issues discovered during the phase audit that are not directly fixed by Plans 01-04.

## Context

- **Motivation:** Clean code craftsman approach — fix everything wrong, even if not directly related to the phase requirements.
- **Current state:** 13 blanket clippy allows in `lib.rs`, inline tests in handler files, dead code annotations, and various lint suppressions.
- **Depends on:** Plans 01 and 03 — those plans rewrite significant handler code, so clean code sweep runs after to avoid merge conflicts and to address any new issues introduced.

## Tasks

### Task 1: Remove Blanket Clippy Allows from lib.rs

**Files:**
- `crates/velesdb-server/src/lib.rs`

**Action:**
1. Remove all 13 blanket `#![allow(...)]` at the top of `lib.rs`:
   ```rust
   #![allow(clippy::pedantic)]
   #![allow(clippy::nursery)]
   #![allow(clippy::doc_markdown)]
   // ... etc.
   ```
2. Run `cargo clippy -p velesdb-server -- -D warnings -W clippy::pedantic` to see all warnings.
3. Fix genuine issues found by clippy pedantic:
   - `doc_markdown` — fix doc comments with unescaped items like `VelesDB`.
   - `uninlined_format_args` — use `format!("{variable}")` instead of `format!("{}", variable)`.
   - `manual_let_else` — convert `match ... { Some(x) => x, None => return }` to `let ... else`.
   - `cast_possible_truncation` — add `#[allow]` with `// Reason:` comment ONLY where justified.
   - `trivially_copy_pass_by_ref` — fix small types passed by reference.
   - `map_unwrap_or` — replace with `map_or`.
   - `needless_for_each` — replace with `for` loops.
4. Add **targeted** `#[allow(...)]` with `// Reason:` comments ONLY where fixing would hurt readability.
5. Keep `#![allow(clippy::enum_glob_use)]` if glob imports are used for ergonomics — add `// Reason:` comment.

**What to avoid:**
- Do NOT blindly suppress all warnings — fix them.
- Do NOT break public API signatures while fixing.
- Do NOT spend more than a targeted `#[allow]` on items that are genuinely fine (e.g., `match_same_arms` in intentional patterns).

**Verify:**
```powershell
cargo clippy -p velesdb-server -- -D warnings
```

**Done when:**
- Zero blanket `#![allow(clippy::pedantic)]` or `#![allow(clippy::nursery)]` in `lib.rs`.
- All remaining `#[allow]` have `// Reason:` comments.
- Clippy clean with `-D warnings`.

### Task 2: Extract Inline Tests to Separate Files

**Files:**
- `crates/velesdb-server/src/handlers/query.rs` (REMOVE tests section)
- `crates/velesdb-server/src/handlers/match_query.rs` (REMOVE tests section)
- `crates/velesdb-server/src/handlers/graph/mod.rs` (REMOVE tests section)
- `crates/velesdb-server/src/handlers/graph/stream.rs` (REMOVE tests section)
- `crates/velesdb-server/src/handlers/metrics.rs` (REMOVE tests section)
- `crates/velesdb-server/tests/query_handler_tests.rs` (NEW)
- `crates/velesdb-server/tests/graph_handler_tests.rs` (NEW or update existing)

**Action:**
1. Move `query.rs` tests (lines 393-438: `detect_query_type` tests) to `tests/query_handler_tests.rs`.
   - These tests use `velesql::Parser` directly — they can be standalone.
2. Move `match_query.rs` tests (lines 197-251: serialization tests) to `tests/` or keep in `match_query.rs` if they test private types.
   - If types are `pub`, move to integration test. If not, keep as unit test but ensure file stays clean.
3. Move `graph/mod.rs` tests (lines 27-220) — these test `GraphService` which will be deleted by Plan 01. After Plan 01, write new tests for the core-bound graph handlers.
4. Move `stream.rs` tests (lines 119-155: serialization tests) to `tests/graph_handler_tests.rs`.
5. Move `metrics.rs` test (lines 72-83) — trivial, can keep inline or move.
6. Ensure `lib.rs` tests (lines 143-381) stay — they test OpenAPI spec generation and type serialization, which is appropriate for unit tests.

**Verify:**
```powershell
cargo test -p velesdb-server
```

**Done when:**
- Handler `.rs` files contain zero `#[cfg(test)] mod tests` sections (except `lib.rs`).
- All tests pass from their new locations.
- No test coverage lost.

### Task 3: Fix Dead Code and Miscellaneous Issues

**Files:**
- `crates/velesdb-server/src/handlers/query.rs`
- `crates/velesdb-server/src/handlers/metrics.rs`
- `crates/velesdb-server/src/handlers/graph/stream.rs`

**Action:**
1. `detect_query_type` in `query.rs:361`:
   - Currently `#[allow(dead_code)]` with comment "will be used in unified handler".
   - Either: wire it into the `query` handler to set `QueryType` in response, OR remove the `#[allow(dead_code)]` and let it be tested publicly.
   - Preferred: use it in the query handler to populate a `query_type` field in response (addresses EPIC-052 US-006 partial).
2. `metrics.rs:11`:
   - Remove `#![allow(dead_code)]` — functions are `pub` and used when `prometheus` feature is enabled.
   - Add `#[cfg(feature = "prometheus")]` to the functions if not already present (they are behind `#[cfg(feature = "prometheus")]` at module level via `mod.rs:22-23`).
3. `stream.rs:87,100`:
   - The `as u64` casts on `as_millis()` are practically safe (elapsed time in ms won't overflow u64 for 584M years).
   - Add `// Reason: elapsed time in milliseconds cannot practically overflow u64` comment above each cast.
   - Or use `#[allow(clippy::cast_possible_truncation)]` with the Reason comment.

**Verify:**
```powershell
cargo clippy -p velesdb-server -- -D warnings
cargo test -p velesdb-server
```

**Done when:**
- Zero unjustified `#[allow(dead_code)]` in server crate.
- All remaining `#[allow]` annotations have `// Reason:` comments.
- Clippy clean.

## Overall Verification

```powershell
cargo fmt --all --check
cargo clippy -p velesdb-server -- -D warnings
cargo test -p velesdb-server
# Verify no blanket allows remain
grep -n "allow(clippy::pedantic)" crates/velesdb-server/src/lib.rs
grep -n "allow(clippy::nursery)" crates/velesdb-server/src/lib.rs
```

## Success Criteria

- [ ] Zero blanket `#![allow(clippy::pedantic)]` or `#![allow(clippy::nursery)]` in `lib.rs`
- [ ] All `#[allow]` annotations have `// Reason:` comments
- [ ] Inline tests extracted to `tests/` directory (handler files clean)
- [ ] Zero unjustified `#[allow(dead_code)]`
- [ ] Clippy clean with `-D warnings`
- [ ] All tests pass (zero coverage loss)
- [ ] `cargo fmt --all --check` passes

## Parallel Safety

- **Exclusive write files:** `lib.rs`, `handlers/query.rs`, `handlers/match_query.rs`, `handlers/graph/mod.rs`, `handlers/graph/stream.rs`, `handlers/metrics.rs`, `tests/query_handler_tests.rs` (new), `tests/graph_handler_tests.rs` (new)
- **Shared read files:** None
- **Conflicts with:** Plan 04 (may both touch `main.rs`) → coordinate changes. This plan focuses on clippy/tests, Plan 04 on rate limiting.

## Output

- **Created:** `tests/query_handler_tests.rs`, `tests/graph_handler_tests.rs`
- **Modified:** `lib.rs`, `handlers/query.rs`, `handlers/match_query.rs`, `handlers/graph/mod.rs`, `handlers/graph/stream.rs`, `handlers/metrics.rs`
