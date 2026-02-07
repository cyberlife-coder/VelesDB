# VelesDB Core — Project State

**Project:** VelesDB Core Refactoring Milestone  
**Current Phase:** 7 — SIMD Tolerance Hardening & DistanceEngine Integration (Planned)  
**Session Started:** 2026-02-06  

---

## Project Reference

### Core Value
The codebase becomes faster, cleaner, more maintainable, and production-ready without breaking existing functionality or public APIs.

### Key Decisions
- **Zero breaking changes** — All public APIs remain stable
- **Martin Fowler patterns** — Industry-standard refactoring approach
- **Rust Book reference** — Authoritative best practices
- **All quality gates enforced** — Prevent tech debt accumulation
- **500-line module limit** — AGENTS.md guideline for maintainability

### Constraints
- Rust 1.83+ only, maintain existing crate structure
- All quality gates must pass: fmt, clippy, deny, test
- Benchmarks must not regress (ideally improve)
- All unsafe code must have documented invariants

---

## Current Position

### Phase Status
| Phase | Name | Status | Blockers |
|-------|------|--------|----------|
| 1 | Foundation Fixes | ✅ Complete | None |
| 2 | Unsafe Code & Testing | ✅ Complete | None |
| 3 | Architecture & Graph | ✅ Complete | None |
| 4 | Complexity & Errors | ✅ Complete | None |
| 5 | Cleanup & Performance | ✅ Complete | None |
| 6 | Documentation & Polish | ⏳ Pending | Phase 5 |
| 7 | SIMD Tolerance & Engine | ✅ Complete | None |

### Current Focus
**Phase 7 complete (2/2 plans) — SIMD tolerance & DistanceEngine integration**

**Wave 1 (sequential):**
- 07-01: Widen SIMD property test tolerances (TEST-08) — ✅ Complete (64fe7739)
- 07-02: Wire DistanceEngine into HNSW hot loop (PERF-04) — ✅ Complete (3b77cdcc)

### Next Action
Phase 6 (Documentation & Polish): DOCS-03, DOCS-04, PERF-02, PERF-03

Progress: ████████████████ 96%

---

## Requirements Progress

 ### Completion Summary
- **Completed:** 25/28 (89%)
- **In Progress:** 0/28
- **Pending:** 3/28

### By Category

 #### Rust Best Practices (RUST)
- [x] RUST-01 — Numeric cast fixes (Plan 01-01 complete)
- [x] RUST-02 — Clippy allow cleanup (Plan 01-02 complete)
- [x] RUST-03 — Tracing migration (Plan 01-03 complete)
- [x] RUST-04 — SAFETY comments (Plan 02-01 in-scope closure)
- [x] RUST-05 — must_use attributes (Plan 02-01 in-scope closure)

 #### Code Quality (QUAL)
- [x] QUAL-01 — Module extraction (all files <500 lines: SIMD 1604→105, 20+ files split in Phase 4)
- [x] QUAL-02 — Deduplication (HNSW serde dedup, shared validation, tail_unroll macros)
- [x] QUAL-03 — Complexity reduction (17 query submodules, clippy pedantic 476→0, all functions <25 CC)
- [x] QUAL-04 — Naming clarity (addressed in 04-07 pedantic remediation)

#### Bug Fixes (BUG)
- [x] BUG-01 — Cast overflow risks (Plan 01-01 complete)
- [x] BUG-02 — Incorrect comments (audited in Phase 2 + simd audit session)
- [x] BUG-03 — Parser fragility (targeted hotspot closure in Plan 02-02)
- [x] BUG-04 — HNSW lock ordering (runtime checker + counters in Plan 03-03)

#### Cleanup (CLEAN)
- [x] CLEAN-01 — Dead code (0 dead_code warnings, 0 #[allow(dead_code)] annotations)
- [x] CLEAN-02 — Unused deps (10 deps removed across 7 crates, cargo machete clean)
- [x] CLEAN-03 — Feature flags (orphaned portable-simd removed, all flags documented)

#### Documentation (DOCS)
- [x] DOCS-01 — Panic to error (4 panic sites converted in 04-01, production hardening in 04-08)
- [x] DOCS-02 — Error context (3 enriched error variants + 64 bare-string errors fixed in 04-08)
- [ ] DOCS-03 — Public API docs
- [ ] DOCS-04 — Outdated docs

#### Testing (TEST)
- [x] TEST-01 — SIMD property tests (Plan 02-03 complete)
- [x] TEST-02 — Concurrent resize tests (Plan 03-04 complete)
- [x] TEST-03 — GPU error tests (Plan 04-09 complete)
- [x] TEST-04 — WAL recovery edge cases (26 tests: partial writes, corruption, crash recovery)

#### Performance (PERF)
- [x] PERF-01 — SIMD dispatch (DistanceEngine with cached fn pointers, 13% faster at 1536d cosine)
- [ ] PERF-02 — Async I/O
- [ ] PERF-03 — Format allocations
- [x] PERF-04 — Wire DistanceEngine into HNSW hot loop (Plan 07-02 complete)

#### Testing (TEST) — continued
- [x] TEST-08 — Widen SIMD property test tolerances (Plan 07-01 complete)

---

## Quality Gates Status

| Gate | Status | Notes |
|------|--------|-------|
| cargo fmt | ✅ Pass | All files formatted |
| cargo clippy | ✅ Pass | Production code clean (lib targets pass -D warnings) |
| cargo deny | ⏳ Pending | Security audit pending |
| cargo test | ✅ Pass | 21 new tests added, all passing |
| Benchmarks | ⏳ Unknown | Baseline needed |

---

## Accumulated Context

### Known Pain Points (from PROJECT.md)
1. `simd_native.rs` ~2400 lines — needs modularization
2. Global clippy allows masking potential bugs
3. Complex HNSW lock ordering — fragile to modification
4. Multiple BUG-XXX comments in VelesQL parser
5. Production code using `eprintln!` instead of tracing
6. Numeric cast patterns using `as` instead of `try_from()`

### High-Risk Files (from AGENTS.md)
- `src/lib.rs` — API entry point; `/impact-analysis` required
- `collection/core/mod.rs` — Core logic; exhaustive tests needed
- `storage/mmap.rs` — Persistent data; compatibility concern
- `index/hnsw/native/graph.rs` — Performance-critical; benchmarks needed

### SAFETY Comment Template (from AGENTS.md)
```rust
// SAFETY: [Invariant principal maintenu]
// - [Condition 1]: [Explication]
// - [Condition 2]: [Explication]
// Reason: [Pourquoi unsafe est nécessaire]
unsafe { ... }
```

---

## Decisions Log

| Date | Decision | Rationale | Phase |
|------|----------|-----------|-------|
| 2026-02-06 | Zero breaking changes | Existing users rely on current APIs | All |
| 2026-02-06 | Martin Fowler patterns | Industry-standard refactoring | All |
| 2026-02-06 | Rust Book reference | Authoritative best practices | All |
| 2026-02-06 | 500-line module limit | AGENTS.md maintainability guideline | 3 |
| 2026-02-06 | 6-phase structure | Natural delivery boundaries | Roadmap |
| 2026-02-06 | Error::Overflow variant added | Support try_from() conversions with VELES-023 | 1 |
| 2026-02-06 | Existing codebase compliant | High-risk files already have SAFETY comments/annotations | 1 |
| 2026-02-06 | Use tracing::warn! for recoverable failures | Collection loading failures don't stop operation | 1 |
| 2026-02-06 | Structured logging format | key=value pairs enable log aggregation/search | 1 |
| 2026-02-06 | Keep println! in test code | Appropriate for benchmark/performance output | 1 |
| 2026-02-06 | Use workspace.lints.clippy | Centralized lint configuration across 8 crates | 1 |
| 2026-02-06 | SAFETY-style justification for allows | Document invariants for each numeric cast suppression | 1 |
| 2026-02-06 | Module-level allows preferred | Targeted suppression vs global blanket allows | 1 |
| 2026-02-06 | BUG-02 scope bounded to adjacent parser hotspots | Avoid broad comment churn while closing BUG-03 targeted sites | 2 |
| 2026-02-06 | Correlation dedup regression uses quoted dotted identifier | Current grammar path that exercises extraction/dedup assertions reliably | 2 |
| 2026-02-06 | Inventory-first unsafe closure | Track and close every unsafe-bearing in-scope file with evidence fields | 2 |
| 2026-02-06 | must_use audit with rationale ledger | Avoid blanket annotations while enforcing return-value discipline | 2 |
| 2026-02-07 | Persist proptest failures for integration tests | Ensures reproducible SIMD counterexamples without source-root lookup ambiguity | 2 |
| 2026-02-07 | Per-metric tolerance envelopes | Keeps SIMD scalar-equivalence assertions stable across ISA/accumulation-order differences | 2 |
| 2026-02-07 | AGENTS-template SAFETY comments mandatory | Header + condition bullets + Reason line required for all unsafe blocks | 2 |
| 2026-02-07 | Facade-first SIMD extraction | Convert simd_native.rs to directory module; ISA kernels transiently in mod.rs | 3 |
| 2026-02-07 | Dispatch as separate module | SimdLevel + detection + all public dispatch in dispatch.rs | 3 |
| 2026-02-07 | Shared serde helpers with struct wrappers | Better call-site readability than generic trait for HnswMeta/HnswMappingsData | 3 |
| 2026-02-07 | Lock-rank checker always-on in release | Thread-local stack ~10-20ns per call, acceptable for lock paths | 3 |
| 2026-02-07 | Safety counter logging gated behind debug_assertions | Tracing formatting cost non-trivial; atomic increments near-zero | 3 |
| 2026-02-07 | Hybrid parser split: clause modules + shared validation | Matches locked CONTEXT.md decision; reduces validation drift | 3 |
| 2026-02-07 | Merged 3 tasks into single atomic commit | Validation module integral to extraction; cleaner changeset | 3 |
| 2026-02-07 | Soft-delete tested at NativeHnswIndex level | remove() operates on mappings only; graph retains tombstones | 3 |
| 2026-02-07 | Loom epoch tests target semantics only | Loom doesn't support file I/O; standard tests cover full stack | 3 |

---

## Blockers & Risks

### Active Blockers
None.

### Potential Risks
| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Breaking changes | Low | High | API compatibility checks |
| Performance regression | Medium | Medium | Benchmarks per phase |
| Scope creep | Medium | Low | Strict v1 adherence |
| Test flakiness | Low | Medium | Property-based tests |

---

## Session Continuity

  ### Last Session
2026-02-07 — Plan 07-02 executed: Zero-dispatch CachedSimdDistance for HNSW
- Extended simd_native::DistanceEngine with hamming_fn/jaccard_fn (all 5 metrics)
- Created CachedSimdDistance struct implementing HNSW DistanceEngine trait
- Wired CachedSimdDistance into NativeHnswInner (replaces SimdDistance)
- 8 new tests, 2432 total pass, 313 HNSW tests zero regressions
- Commits: 5b9a05db → 3b77cdcc (5 atomic commits)

### Current Branch
feature/CORE-phase5-plan01-dependency-cleanup

### Uncommitted Changes
None (all committed)

### Notes for Next Session
1. Phase 7 complete — all plans executed
2. Phase 6 (DOCS-03, DOCS-04, PERF-02, PERF-03) still pending
3. CachedSimdDistance now active in production HNSW path

---

## Quick Reference

### Important File Paths
- `.planning/PROJECT.md` — Project definition
- `.planning/REQUIREMENTS.md` — v1/v2 requirements
- `.planning/ROADMAP.md` — Phase structure
- `.planning/STATE.md` — This file
- `AGENTS.md` — Coding standards and templates

### Key Commands
```bash
# Quality gates
cargo fmt --all
cargo clippy -- -D warnings
cargo deny check
cargo test --workspace
cargo build --release

# Impact analysis (before lib.rs changes)
/impact-analysis src/lib.rs

# Local CI validation
./scripts/local-ci.ps1
```

### Architecture
- 8 crates in workspace
- Core: `velesdb-core` with HNSW, SIMD, storage, query engine
- Bindings: WASM, Python (PyO3), Tauri plugin

---

*State file last updated: 2026-02-07*  
*Progress: 25/28 requirements (89%) — Phase 7 complete, Phase 6 pending*
