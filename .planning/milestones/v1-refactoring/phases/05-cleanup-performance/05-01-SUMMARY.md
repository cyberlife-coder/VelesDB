---
phase: 5
plan: 1
completed: 2026-02-07
duration: ~25 minutes
---

# Phase 5 Plan 1: Dependency Hygiene & Dead Code Cleanup — Summary

## One-liner

Removed 10 unused dependencies across 7 crates, eliminated orphaned `portable-simd` feature flag, and documented all remaining feature flags with comprehensive comments.

## What Was Built

This plan performed a thorough audit of the entire VelesDB workspace for dependency hygiene, dead code, and feature flag clarity. Using `cargo machete` as the primary scanner, each flagged dependency was manually verified with grep searches before removal. Two false positives (sqlx behind feature gate, tokio as uniffi runtime) were documented with `[package.metadata.cargo-machete]` ignore entries.

The dead code audit found zero `#[allow(dead_code)]` annotations and zero dead code warnings — confirming the codebase is already clean from prior phases. The feature flag audit identified one orphaned flag (`portable-simd` from EPIC-054 evaluation) with zero `cfg` references in source, which was removed. All 4 remaining features now have clear documentation in `Cargo.toml`.

## Tasks Completed

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | Remove unused dependencies across 7 crates | `49523df8` | 7 Cargo.toml files |
| 2 | Audit #[allow(dead_code)] annotations | N/A (already clean) | 0 files |
| 3 | Feature flag audit and documentation | `00affad6` | crates/velesdb-core/Cargo.toml |

## Key Files

**Modified:**
- `crates/velesdb-cli/Cargo.toml` — removed thiserror
- `crates/velesdb-core/Cargo.toml` — removed anyhow, arc-swap, bytes, crossbeam-channel; removed portable-simd feature; added feature docs
- `crates/velesdb-migrate/Cargo.toml` — removed futures; added sqlx to machete ignore
- `crates/velesdb-mobile/Cargo.toml` — added tokio to machete ignore
- `crates/velesdb-server/Cargo.toml` — removed config, thiserror
- `crates/velesdb-wasm/Cargo.toml` — removed half
- `demos/tauri-rag-app/src-tauri/Cargo.toml` — removed thiserror

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Keep futures in velesdb-server | Used in handlers/graph/stream.rs (initial grep missed it) |
| Keep sqlx in velesdb-migrate (ignore) | Optional dep behind `postgres` feature flag — cargo-machete can't detect |
| Keep tokio in velesdb-mobile (ignore) | Runtime dependency for uniffi's async support — not directly imported |
| Remove portable-simd feature | Zero `cfg(feature = "portable-simd")` references in source — orphaned EPIC-054 eval |
| Remove bytes from velesdb-core | `vector_to_bytes`/`bytes_to_vector` are internal functions, not the `bytes` crate |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug Fix] futures dependency incorrectly removed from velesdb-server**
- Found during: Task 1 verification (`cargo check --workspace`)
- Issue: `futures` was flagged by cargo-machete in velesdb-server, but it IS used in `handlers/graph/stream.rs`
- Fix: Restored `futures = "0.3"` immediately after build failure
- Files: `crates/velesdb-server/Cargo.toml`
- Commit: included in `49523df8`

**2. [Rule 2 - Critical] cargo-machete report differs from plan predictions**
- Found during: Task 1
- Issue: Plan predicted certain deps (from earlier scan), but current machete found different set (anyhow, bytes, arc-swap added; some removed)
- Fix: Used current machete output as ground truth, verified each with grep
- Files: All Cargo.toml files
- Commit: `49523df8`

## Verification Results

```
cargo machete: 0 unused dependencies ✅
cargo clippy --workspace -- -D warnings: 0 errors ✅
cargo test -p velesdb-core --lib: 2382 passed, 0 failed ✅
cargo check --workspace: success ✅
cargo check --package velesdb-core --no-default-features: success ✅
cargo check --package velesdb-wasm: success ✅
```

## Next Phase Readiness

- CLEAN-01 (dead code): Already satisfied — 0 dead code warnings
- CLEAN-02 (unused deps): Satisfied — 0 unused dependencies
- CLEAN-03 (feature flags): Satisfied — all flags documented, orphaned flag removed
- Ready for Plan 05-02 (WAL recovery tests) or Plan 05-03 (SIMD dispatch optimization)

---
*Completed: 2026-02-07T21:52Z*
