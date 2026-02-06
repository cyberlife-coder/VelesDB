# VelesDB Core Refactoring Roadmap

**Version:** 1.0  
**Created:** 2026-02-06  
**Depth:** Comprehensive  
**Total Phases:** 6  
**Total Requirements:** 26 v1 requirements  

---

## Overview

This roadmap delivers a comprehensive refactoring of the VelesDB Core codebase, transforming it from a functional but complex system into a production-ready, maintainable, and performant vector database. The refactoring follows Martin Fowler patterns and Rust best practices while maintaining zero breaking changes to public APIs.

Each phase delivers a coherent, verifiable capability that builds upon previous phases. Progress is measured by observable outcomes, not just tasks completed.

---

## Phase 1: Foundation Fixes

**Goal:** Establish Rust best practices foundation by eliminating unsafe casts, global lint suppressions, and improper logging in library code.

**Estimated Complexity:** Medium (200-400 lines changed across 10-15 files)  
**Estimated Duration:** 2-3 sessions  

### Requirements

| ID | Requirement | Priority |
|----|-------------|----------|
| RUST-01 | Replace `as` casts with `try_from()` or explicit bounds checks | High |
| RUST-02 | Remove global `#[allow]` clippy attributes, use targeted allows | High |
| RUST-03 | Replace `eprintln!`/`println!` with `tracing` macros | Medium |
| BUG-01 | Fix numeric cast overflow/truncation risks | High |

### Success Criteria

1. **Zero unsafe numeric conversions:** All `as` casts on user-provided data use `try_from()` with proper error handling or have explicit `#[allow]` with justification comment
2. **Clean clippy configuration:** Global `#[allow]` attributes removed from `lib.rs`, all suppressions are function-level with SAFETY-style justification
3. **Professional logging:** Library code uses only `tracing::info!`, `debug!`, `warn!`, `error!` — no `println!` or `eprintln!` in production paths
4. **Bounds-checked arithmetic:** All numeric operations that could overflow have explicit bounds checks with explanatory comments
5. **CI gates pass:** `cargo clippy -- -D warnings` succeeds with zero warnings from new code

### Key Files

- `crates/velesdb-core/src/lib.rs` (global allows at lines 61-65, eprintln at 437)
- All files with numeric conversions (audit during implementation)

### Plans

**Plans:** 5 plans in 1 wave (all parallel)

- [x] `01-01-PLAN.md` — Numeric Cast Audit & Fixes (RUST-01, BUG-01)
- [x] `01-02-PLAN.md` — Clippy Configuration Cleanup (RUST-02)
- [x] `01-03-PLAN.md` — Tracing Migration (RUST-03)
- [x] `01-04-PLAN.md` — Fix Clippy Cast Errors (Gap Closure)
- [x] `01-05-PLAN.md` — Add SAFETY Comments & Integration Tests (deferred to Phase 2)

---

## Phase 2: Unsafe Code Audit & Testing Foundation

**Goal:** Make unsafe code auditable and verifiable by adding comprehensive SAFETY documentation and establishing property-based testing for SIMD correctness.

**Estimated Complexity:** High (100+ unsafe blocks, ~600 lines of new tests)  
**Estimated Duration:** 3-4 sessions  

### Dependencies

- Phase 1 (RUST-02 for SAFETY comment style consistency)

### Requirements

| ID | Requirement | Priority |
|----|-------------|----------|
| RUST-04 | Add SAFETY comments to all unsafe blocks | High |
| RUST-05 | Apply `#[must_use]` to appropriate functions | Medium |
| BUG-02 | Fix incorrect comments that don't match code | High |
| BUG-03 | Resolve VelesQL parser fragility (BUG-XXX markers) | High |
| TEST-01 | Add property-based tests for SIMD equivalence | High |

### Success Criteria

1. **Documented invariants:** Every unsafe block has a `// SAFETY:` comment following AGENTS.md template with invariant documentation
2. **Parser stability:** All BUG-XXX markers in VelesQL parser resolved with permanent fixes or documented workarounds
3. **Comment accuracy:** All code comments audited and verified to match actual behavior; misleading comments corrected
4. **SIMD verification:** Property-based tests (proptest/quickcheck) verify SIMD implementations produce identical results to scalar fallbacks for all distance metrics
5. **API discipline:** `#[must_use]` applied to all public functions where ignoring return values would indicate a bug

### Key Files

- `simd_native.rs` (~100 unsafe blocks)
- `simd_neon.rs` (unsafe blocks)
- `storage/guard.rs` (unsafe blocks)
- `velesql/parser/select.rs` (BUG-XXX at 414, 685)
- `velesql/parser/values.rs` (BUG-XXX at 377, 384)

### Plans

**Plans:** 3 plans in 2 waves

Plans:
- [ ] `02-01-PLAN.md` — Inventory-led unsafe closure across core modules with scoped BUG-02 audit and broad `#[must_use]` coverage
- [ ] `02-02-PLAN.md` — VelesQL parser fragility fixes with assertion-style regressions at targeted BUG sites
- [ ] `02-03-PLAN.md` — Property-based SIMD equivalence test foundation with reproducible tolerance policy

---

## Phase 3: Architecture Extraction & Graph Safety

**Goal:** Improve maintainability by extracting oversized modules into coherent sub-modules and strengthen HNSW concurrent access safety.

**Estimated Complexity:** Very High (~2400 lines restructured, new module hierarchy)  
**Estimated Duration:** 4-5 sessions  

### Dependencies

- Phase 2 (TEST-01 foundation for testing refactored SIMD)

### Requirements

| ID | Requirement | Priority |
|----|-------------|----------|
| QUAL-01 | Extract sub-modules from files >500 lines | High |
| QUAL-02 | Remove code duplication across modules | High |
| BUG-04 | Strengthen HNSW lock ordering documentation | High |
| TEST-02 | Add concurrent resize operation tests | Medium |

### Success Criteria

1. **Modular SIMD:** `simd_native.rs` (~2400 lines) extracted into `simd/avx512.rs`, `simd/avx2.rs`, `simd/sse.rs`, etc. with clear module boundaries
2. **Modular HNSW:** `index/hnsw/native/graph.rs` (~800 lines) split into logical submodules (graph operations, neighbor management, etc.)
3. **Modular Parser:** `velesql/parser/select.rs` (~1000 lines) decomposed into parser submodules by SQL clause type
4. **No module bloat:** Zero source files exceed 500 lines (except test files and auto-generated code)
5. **Lock ordering documented:** HNSW lock ordering invariant documented at `index/hnsw/native/graph.rs:585-636` with runtime checker in debug builds
6. **Concurrent safety tested:** Integration tests verify `VectorSliceGuard` behavior during mmap resize operations
7. **Code deduplication:** Distance calculation patterns, error handling boilerplate, and serialization patterns consolidated into shared utilities

### Key Files

- `simd_native.rs` → new `simd/` subdirectory structure
- `index/hnsw/native/graph.rs` → graph operations submodule
- `velesql/parser/select.rs` → parser submodules
- `storage/guard.rs` (concurrent resize tests)

---

## Phase 4: Complexity Reduction & Error Handling

**Goal:** Improve code clarity and robustness by simplifying complex functions and converting inappropriate panics to proper errors.

**Estimated Complexity:** Medium-High (20-30 functions refactored, error types redesigned)  
**Estimated Duration:** 3-4 sessions  

### Dependencies

- Phase 3 (module extraction provides cleaner boundaries for refactoring)

### Requirements

| ID | Requirement | Priority |
|----|-------------|----------|
| QUAL-03 | Reduce cognitive complexity to <25 | Medium |
| QUAL-04 | Improve naming clarity and consistency | Medium |
| DOCS-01 | Convert panics to proper errors | High |
| DOCS-02 | Add error context and chain information | Medium |
| TEST-03 | Add GPU error handling tests | Medium |

### Success Criteria

1. **Complexity compliance:** All functions pass `.clippy.toml` cognitive complexity threshold (25); no cognitive complexity warnings
2. **Panic elimination:** Panics in `column_store/mod.rs:87-109` and `storage/guard.rs:84-90` converted to proper `Result` types with descriptive errors
3. **Rich error context:** Errors use `anyhow::Context` and `thiserror` to provide clear debugging information including error chains
4. **Clear naming:** All abbreviated variable names expanded, unclear function names clarified (e.g., `calc` → `calculate_euclidean_distance`)
5. **GPU resilience:** Tests verify graceful handling when GPU is unavailable or operations fail; no panics on GPU errors
6. **Clippy clean:** `cargo clippy -- -W clippy::cognitive_complexity` reports no violations

### Key Files

- Functions flagged by clippy cognitive complexity (audit during implementation)
- `column_store/mod.rs` (panic removal)
- `storage/guard.rs` (panic removal)
- `gpu.rs` (error handling tests)

---

## Phase 5: Cleanup & Performance Optimization

**Goal:** Remove technical debt through dead code elimination and optimize critical hot paths for measurable performance gains.

**Estimated Complexity:** Medium (code removal + targeted optimizations)  
**Estimated Duration:** 2-3 sessions  

### Dependencies

- Phase 4 (cleaner code enables accurate dead code detection)

### Requirements

| ID | Requirement | Priority |
|----|-------------|----------|
| CLEAN-01 | Remove unreachable code | Medium |
| CLEAN-02 | Remove unused dependencies | Low |
| CLEAN-03 | Clean up feature flags | Low |
| TEST-04 | Add WAL recovery edge case tests | High |
| PERF-01 | Optimize SIMD dispatch | Medium |

### Success Criteria

1. **Zero dead code:** `cargo clippy -- -W dead_code` reports no warnings; all unreachable code identified and removed
2. **Dependency hygiene:** `cargo machete` reports zero unused dependencies; all declared deps actively used
3. **Feature clarity:** All feature flags documented with clear purpose; unnecessary conditional compilation removed
4. **WAL resilience:** WAL recovery tests cover partial writes, corruption scenarios, and crash recovery edge cases
5. **SIMD optimization:** SIMD dispatch in `simd_native.rs:1339-1400` optimized to cache function pointers in `DistanceEngine`, reducing branch misprediction
6. **Benchmark parity:** All benchmarks maintain or improve performance vs. baseline (no regressions)

### Key Files

- All source files (dead code audit)
- `Cargo.toml` files (dependency audit)
- `storage/mmap.rs` (WAL recovery tests)
- `simd_native.rs:1339-1400` (dispatch optimization)

---

## Phase 6: Documentation & Final Polish

**Goal:** Complete the refactoring milestone with comprehensive public API documentation and final performance optimizations.

**Estimated Complexity:** Medium (documentation + async refactoring)  
**Estimated Duration:** 2-3 sessions  

### Dependencies

- All previous phases (documentation requires stable APIs)

### Requirements

| ID | Requirement | Priority |
|----|-------------|----------|
| DOCS-03 | Document all public APIs with rustdoc examples | High |
| DOCS-04 | Fix misleading or outdated documentation | Medium |
| PERF-02 | Move blocking I/O to spawn_blocking | Low |
| PERF-03 | Eliminate format allocations in hot paths | Medium |

### Success Criteria

1. **Complete API docs:** Every public function in `lib.rs`, collection APIs, and search APIs has rustdoc with usage example
2. **Accurate documentation:** README, AGENTS.md, and all rustdocs reviewed and updated; outdated information corrected
3. **Async I/O safety:** Blocking operations in `storage/mmap.rs:158-195` (`mmap.flush()`, `set_len()`) moved to `spawn_blocking` to prevent async runtime blocking
4. **Allocation reduction:** Format allocations eliminated from hot paths in `index/trigram/simd.rs` using stack buffers or string interning
5. **Quality gates pass:** All quality gates pass: `cargo fmt`, `clippy -D warnings`, `deny check`, `test --workspace`
6. **Milestone complete:** All 26 v1 requirements satisfied; codebase is production-ready

### Key Files

- `src/lib.rs` (public API documentation)
- `README.md`, `AGENTS.md` (outdated docs)
- `storage/mmap.rs` (async I/O)
- `index/trigram/simd.rs` (allocation optimization)

---

## Progress Tracker

| Phase | Status | Progress | Requirements | Success Criteria Met |
|-------|--------|----------|--------------|---------------------|
| 1 - Foundation Fixes | ✅ Complete | 100% | 4/26 | 5/5 |
| 2 - Unsafe Code & Testing | ⏳ Pending | 0% | 5/26 | 0/5 |
| 3 - Architecture & Graph | ⏳ Pending | 0% | 4/26 | 0/7 |
| 4 - Complexity & Errors | ⏳ Pending | 0% | 5/26 | 0/6 |
| 5 - Cleanup & Performance | ⏳ Pending | 0% | 5/26 | 0/6 |
| 6 - Documentation & Polish | ⏳ Pending | 0% | 4/26 | 0/6 |

**Overall Progress:** 4/26 requirements (15%)

---

## Quality Gates

All phases must pass these gates before completion:

- [ ] `cargo fmt --all` — Code formatting
- [ ] `cargo clippy -- -D warnings` — Zero warnings
- [ ] `cargo deny check` — Security audit clean
- [ ] `cargo test --workspace` — All tests pass
- [ ] `cargo build --release` — Release build succeeds
- [ ] Benchmarks maintain or improve performance

---

## Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Breaking changes introduced | Strict API compatibility checks; `/impact-analysis` before lib.rs changes |
| Performance regression | Benchmarks before/after each phase; abort if regression >5% |
| Test failures | Comprehensive tests added in Phase 2 before major refactoring |
| Scope creep | Strict adherence to v1 requirements; v2 items deferred |
| Unsafe code bugs | SAFETY documentation + property-based testing in Phase 2 |

---

## Post-Milestone

After Phase 6 completion:

1. **Tag release:** `git tag v0.x.0-refactored`
2. **Update CHANGELOG:** Document all refactoring improvements
3. **Run final benchmarks:** Generate performance comparison report
4. **Schedule v2:** Review v2 requirements (TEST-05 through QUAL-05) for next milestone

---

*Roadmap created: 2026-02-06*  
*Last updated: 2026-02-06*  
*Next review: After Phase 2 completion*
