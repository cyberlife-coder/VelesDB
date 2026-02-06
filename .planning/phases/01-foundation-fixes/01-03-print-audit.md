# Print Statement Audit Report

**Plan:** 01-03 - Tracing Migration (RUST-03)  
**Date:** 2026-02-06  
**Auditor:** gsd-executor

## Summary

Total print statements found: 1 in library code (production paths)  
Test code print statements: 15 (excluded from migration)

## Library Code Print Statements (Requires Migration)

| # | File | Line | Statement | Context | Recommended Level |
|---|------|------|-----------|---------|-------------------|
| 1 | `crates/velesdb-core/src/lib.rs` | 379 | `eprintln!("Warning: Failed to load collection: {err}");` | Collection loading failure in `load_collections()` | `tracing::warn!` |

**Rationale:** This is a warning about a recoverable failure during collection loading. The operation continues despite the failure (the error is logged but not propagated). This is appropriate for `warn!` level.

## Test Code Print Statements (Excluded - Allowed)

| File | Lines | Count | Reason for Exclusion |
|------|-------|-------|---------------------|
| `cache/lockfree_tests.rs` | 185-188 | 4 | Test file - benchmark output |
| `cache/lru_optimization_tests.rs` | 47-224 | 10 | Test file - performance metrics |
| `cache/performance_tests.rs` | 139-278 | 4 | Test file - throughput reports |
| `gpu/gpu_backend_tests.rs` | 22-24 | 2 | Test file - GPU availability |
| `index/trigram/gpu.rs` | 223, 225 | 2 | Inside `#[cfg(test)]` module |
| `index/trigram/simd.rs` | 361, 439 | 2 | Inside `#[cfg(test)]` module |
| `simd_dispatch_tests.rs` | 148-149 | 2 | Test file - feature detection |

**Total test code print statements:** 26  
**Total library code print statements:** 1 (to be migrated)

## Tracing Level Guidelines Applied

- `error!`: Actual errors that need immediate attention - not applicable
- `warn!`: Warning conditions, recoverable issues - **lib.rs:379** ✓
- `info!`: Important operational events - none found
- `debug!`: Detailed debugging information - none found
- `trace!`: Very detailed tracing - none found

## Files Already Using Tracing

✅ `crates/velesdb-core/src/storage/mmap.rs` - Already uses `tracing::error!`  
✅ `crates/velesdb-core/src/index/hnsw/native/graph.rs` - No print statements found

## Migration Plan

1. **lib.rs:379** - Replace `eprintln!` with `tracing::warn!`
   - Add structured logging: `tracing::warn!(error = %err, "Failed to load collection")`
   - Ensure `tracing` import is present (already used elsewhere in the file)

## Verification Checklist

- [ ] Zero `println!`/`eprintln!` in library source files
- [ ] All library logging uses `tracing::` macros
- [ ] Appropriate log levels used
- [ ] CI gates pass: `cargo build`, `cargo clippy`, `cargo test`

---
*Audit completed: 2026-02-06*
