# Requirements: VelesDB Core Refactoring Milestone

**Defined:** 2026-02-06
**Core Value:** The codebase becomes faster, cleaner, more maintainable, and production-ready without breaking existing functionality

## v1 Requirements

### Rust Best Practices (RUST)

- [x] **RUST-01**: Replace all `as` casts with `try_from()` or explicit bounds checks with justification
  - Priority: High
  - Files to audit: All numeric conversion points
  - Success: Zero `as` casts on user-provided data without bounds checking

- [x] **RUST-02**: Remove global `#[allow]` clippy attributes from `lib.rs`, use targeted allows with justification comments
  - Priority: High
  - Files: `crates/velesdb-core/src/lib.rs:61-65`
  - Success: All clippy allows are function-level with SAFETY-style justification

- [x] **RUST-03**: Replace all `eprintln!`/`println!` in library code with proper `tracing` macros
  - Priority: Medium
  - Files: `crates/velesdb-core/src/lib.rs:437` and others
  - Success: Library code uses only `tracing::info!`, `debug!`, `warn!`, `error!`

- [x] **RUST-04**: Add comprehensive SAFETY comments to all unsafe blocks following AGENTS.md template
  - Priority: High
  - Files: `simd_native.rs` (100+ blocks), `simd_neon.rs`, `storage/guard.rs`, etc.
  - Success: Every unsafe block has `// SAFETY:` with invariant documentation
  - Verification: `python scripts/verify_unsafe_safety_template.py --inventory ... --strict` shows 0 violations across all 15 inventory files

- [x] **RUST-05**: Apply `#[must_use]` to all functions returning values that should not be ignored
  - Priority: Medium
  - Files: Public API functions in core crate
  - Success: Compiler warns on ignored return values where appropriate
  - Coverage: 100+ `#[must_use]` annotations across 12 audited modules

### Code Quality & Refactoring Patterns (QUAL)

- [ ] **QUAL-01**: Extract sub-modules from files exceeding 500 lines
  - Priority: High
  - Files:
    - `simd_native.rs` (~2400 lines) → `simd/avx512.rs`, `simd/avx2.rs`, `simd/sse.rs`, etc.
    - `index/hnsw/native/graph.rs` (~800 lines) → graph operations submodule
    - `velesql/parser/select.rs` (~1000 lines) → parser submodules
  - Success: No file >500 lines except test files

- [ ] **QUAL-02**: Remove code duplication across modules
  - Priority: High
  - Areas to audit:
    - Distance calculation patterns (scalar vs SIMD fallbacks)
    - Error handling boilerplate
    - Serialization/deserialization patterns
  - Success: Duplicate logic extracted to shared utilities

- [ ] **QUAL-03**: Reduce cognitive complexity of complex functions to <25
  - Priority: Medium
  - Files: Functions flagged by clippy cognitive complexity
  - Success: All functions pass `.clippy.toml` threshold (25)

- [ ] **QUAL-04**: Improve naming clarity and consistency
  - Priority: Medium
  - Focus: Abbreviated variable names, unclear function names
  - Success: All public APIs have clear, descriptive names

### Bug Fixes & Code Smells (BUG)

- [x] **BUG-01**: Fix numeric cast overflow/truncation risks
  - Priority: High
  - Files: All `as` casts identified in RUST-01 audit
  - Success: No silent truncation; explicit bounds checks with comments

- [x] **BUG-02**: Fix incorrect comments that don't match code behavior
  - Priority: High
  - Approach: Audit all comments during refactoring, verify against implementation
  - Success: Comments accurately describe code behavior
  - Scope: Unsafe-adjacent comments updated with SAFETY template; parser comments corrected

- [x] **BUG-03**: Resolve VelesQL parser fragility (address BUG-XXX markers)
  - Priority: High
  - Files:
    - `velesql/parser/select.rs:414,685`
    - `velesql/parser/values.rs:377,384`
  - Success: All BUG-XXX comments resolved or documented with permanent fixes
  - Verification: 12/13 parser regression tests pass; 1 pre-existing limitation documented

- [ ] **BUG-04**: Strengthen HNSW lock ordering documentation and validation
  - Priority: High
  - Files: `index/hnsw/native/graph.rs:585-636`
  - Success: Lock ordering invariant documented; runtime checker in debug builds

### Dead Code & Dependencies (CLEAN)

- [ ] **CLEAN-01**: Identify and remove unreachable code
  - Priority: Medium
  - Approach: Clippy dead_code warnings + manual audit
  - Success: Zero dead code warnings from clippy

- [ ] **CLEAN-02**: Audit and remove unused dependencies
  - Priority: Low
  - Tool: `cargo machete`
  - Success: All declared dependencies are actively used

- [ ] **CLEAN-03**: Clean up feature flags and conditional compilation
  - Priority: Low
  - Success: Feature flags are well-documented and necessary

### Error Handling & Documentation (DOCS)

- [ ] **DOCS-01**: Convert panics to proper errors where appropriate
  - Priority: High
  - Files:
    - `column_store/mod.rs:87-109` (PK configuration panic)
    - `storage/guard.rs:84-90` (epoch mismatch panic)
  - Success: Panics only for unrecoverable errors; recoverable errors return Result

- [ ] **DOCS-02**: Add missing error context and chain information
  - Priority: Medium
  - Approach: Use `anyhow::Context` and `thiserror` for rich errors
  - Success: Errors provide clear context for debugging

- [ ] **DOCS-03**: Document all public APIs with rustdoc examples
  - Priority: High
  - Focus: `lib.rs` public exports, collection APIs, search APIs
  - Success: Every public function has rustdoc with usage example

- [ ] **DOCS-04**: Fix misleading or outdated documentation
  - Priority: Medium
  - Success: README, AGENTS.md, and rustdocs are current and accurate

### Testing & Quality Assurance (TEST)

- [x] **TEST-01**: Add property-based tests for SIMD vs scalar equivalence
  - Priority: High
  - Files: All `simd_*.rs` files
  - Success: QuickCheck/proptest ensures SIMD matches scalar results
  - Verification: 6/6 proptest cases pass (dot, L2, cosine, hamming, jaccard); 66/66 native tests pass

- [ ] **TEST-02**: Add integration tests for concurrent resize operations
  - Priority: Medium
  - Files: `storage/guard.rs`
  - Success: Tests verify VectorSliceGuard during mmap resize

- [ ] **TEST-03**: Add tests for GPU error handling paths
  - Priority: Medium
  - Files: `gpu.rs`
  - Success: GPU unavailable/failure paths tested

- [ ] **TEST-04**: Add WAL recovery edge case tests
  - Priority: High
  - Files: `storage/mmap.rs`
  - Success: Partial writes, corruption scenarios tested

### Performance Optimization (PERF)

- [ ] **PERF-01**: Optimize SIMD dispatch to reduce branch misprediction
  - Priority: Medium
  - Files: `simd_native.rs:1339-1400`
  - Success: Function pointer cached in DistanceEngine

- [ ] **PERF-02**: Move blocking I/O to spawn_blocking for async contexts
  - Priority: Low
  - Files: `storage/mmap.rs:158-195`
  - Success: `mmap.flush()` and `set_len()` don't block async runtime

- [ ] **PERF-03**: Eliminate format allocations in hot paths
  - Priority: Medium
  - Files: `index/trigram/simd.rs`
  - Success: Stack buffers or string interning for trigram extraction

- [ ] **PERF-04**: Wire DistanceEngine into HNSW hot loop replacing per-call SimdDistance dispatch
  - Priority: Medium (P3)
  - Files: `index/hnsw/native/graph/mod.rs`, `search.rs`, `insert.rs`, `neighbors.rs`
  - Success: NativeHnsw uses cached fn pointers; ~5-15% search latency improvement

### Testing & Quality Assurance (TEST) — continued

- [ ] **TEST-08**: Widen SIMD property test tolerances for dot product and jaccard
  - Priority: High (P2)
  - Files: `tests/simd_property_tests.rs`
  - Success: Both flaky tests pass consistently across 10 consecutive runs; tolerances justified with `// Reason:` comments

## v2 Requirements

### Testing Improvements

- **TEST-05**: Fuzz testing expansion beyond current targets
- **TEST-06**: Loom concurrency testing expansion for all lock-heavy modules
- **TEST-07**: Benchmark regression testing in CI

### Documentation

- **DOCS-05**: Architecture Decision Records (ADRs) for major design choices
- **DOCS-06**: Migration guide for breaking changes (when they eventually happen)

### Code Quality

- **QUAL-05**: Migrate from bincode to maintained serialization library (RUSTSEC-2025-0141)

## Out of Scope

| Feature | Reason |
|---------|--------|
| New features/functionality | This is refactoring-only milestone |
| Breaking API changes | Must maintain backward compatibility |
| Major architectural rewrites | Incremental improvements only |
| GPU benchmark completion | Requires separate feature milestone |
| CART index leaf splitting | Incomplete feature, separate milestone |
| Database migration tools | Focus on code, not data migration |
| GTK3 security advisories | External dependency, affects CLI only |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| RUST-01 | Phase 1 | Complete |
| RUST-02 | Phase 1 | Complete |
| RUST-03 | Phase 1 | Complete |
| RUST-04 | Phase 2 | Complete |
| RUST-05 | Phase 2 | Complete |
| QUAL-01 | Phase 3 | Pending |
| QUAL-02 | Phase 3 | Pending |
| QUAL-03 | Phase 4 | Pending |
| QUAL-04 | Phase 4 | Pending |
| BUG-01 | Phase 1 | Complete |
| BUG-02 | Phase 2 | Complete |
| BUG-03 | Phase 2 | Complete |
| BUG-04 | Phase 3 | Pending |
| CLEAN-01 | Phase 5 | Pending |
| CLEAN-02 | Phase 5 | Pending |
| CLEAN-03 | Phase 5 | Pending |
| DOCS-01 | Phase 4 | Pending |
| DOCS-02 | Phase 4 | Pending |
| DOCS-03 | Phase 6 | Pending |
| DOCS-04 | Phase 6 | Pending |
| TEST-01 | Phase 2 | Complete |
| TEST-02 | Phase 3 | Pending |
| TEST-03 | Phase 4 | Pending |
| TEST-04 | Phase 5 | Pending |
| PERF-01 | Phase 5 | Pending |
| PERF-02 | Phase 6 | Pending |
| PERF-03 | Phase 6 | Pending |
| PERF-04 | Phase 7 | Pending |
| TEST-08 | Phase 7 | Pending |

**Coverage:**
- v1 requirements: 28 total
- Mapped to phases: 28
- Unmapped: 0 ✓

---
*Requirements defined: 2026-02-06*
*Last updated: 2026-02-07 after Phase 7 addition*
*ROADMAP.md: All 28 v1 requirements mapped to 7 phases*
