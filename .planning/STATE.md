# VelesDB Core — Project State

**Project:** VelesDB Core  
**Current Milestone:** None — ready for next milestone  
**Previous Milestone:** v1-refactoring (completed 2026-02-08, tagged `v1.4.1-refactored`)  

---

## Project Reference

### Core Value
VelesDB is a cognitive memory engine for AI agents — Vector + Graph + Symbolique in a single local-first engine.

### Codebase Status (post-refactoring)
- **3,117 tests** passing, 0 failures
- **Quality gates**: fmt ✅, clippy ✅, deny ✅, doc ✅, release build ✅
- **112 unsafe blocks** — all documented with SAFETY comments
- **SIMD**: ISA-specific submodules (AVX-512, AVX2, NEON) + DistanceEngine with cached fn pointers
- **Documentation**: `#![warn(missing_docs)]` enforced, 0 rustdoc warnings

### Constraints
- Rust 1.83+ only
- All quality gates must pass: fmt, clippy, deny, test
- All unsafe code must have documented invariants

---

## Current Position

No active milestone. Use `/gsd-new-milestone` to start one.

### v2 Requirements (from previous milestone)
- **TEST-05**: Fuzz testing expansion
- **TEST-06**: Loom concurrency testing expansion
- **TEST-07**: Benchmark regression testing in CI
- **DOCS-05**: Architecture Decision Records (ADRs)
- **DOCS-06**: Migration guide for breaking changes
- **QUAL-05**: Migrate from bincode to maintained serialization library (RUSTSEC-2025-0141)

---

## Quick Reference

### Important File Paths
- `.planning/PROJECT.md` — Project definition
- `.planning/MILESTONES.md` — Completed milestones
- `.planning/milestones/v1-refactoring/` — Previous milestone archive
- `AGENTS.md` — Coding standards and templates

### Key Commands
```powershell
cargo fmt --all
cargo clippy -- -D warnings
cargo deny check
cargo test --workspace
cargo build --release
.\scripts\local-ci.ps1
```

---

*State file last updated: 2026-02-08*  
*Status: Between milestones — v1-refactoring complete, next milestone pending*
