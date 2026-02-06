# Phase 02-01 Unsafe Audit Inventory

## BUG-02 Scope Boundary

In scope for this plan:
- Every non-test file under `crates/velesdb-core/src` containing `unsafe {}`, `unsafe impl`, or `unsafe fn`.

Out of scope for this plan:
- Parser files (`crates/velesdb-core/src/velesql/parser/*`) handled in Plan 02-02.
- Test-only files (`*test*.rs`, `#[cfg(test)]` modules).
- Prose/docs outside Rust source modules.

## Unsafe + Must-Use Ledger

file: crates/velesdb-core/src/alloc_guard.rs
unsafe_sites: 3
safety_status: needs_fix
must_use_status: not_needed
comment_audit_status: corrected

file: crates/velesdb-core/src/perf_optimizations.rs
unsafe_sites: 11
safety_status: needs_fix
must_use_status: not_needed
comment_audit_status: corrected

file: crates/velesdb-core/src/simd_native.rs
unsafe_sites: 61
safety_status: needs_fix
must_use_status: not_needed
comment_audit_status: corrected

file: crates/velesdb-core/src/simd_neon.rs
unsafe_sites: 9
safety_status: needs_fix
must_use_status: needs_add
comment_audit_status: corrected

file: crates/velesdb-core/src/simd_neon_prefetch.rs
unsafe_sites: 6
safety_status: needs_fix
must_use_status: not_needed
comment_audit_status: corrected

file: crates/velesdb-core/src/storage/guard.rs
unsafe_sites: 3
safety_status: needs_fix
must_use_status: not_needed
comment_audit_status: corrected

file: crates/velesdb-core/src/storage/compaction.rs
unsafe_sites: 4
safety_status: needs_fix
must_use_status: not_needed
comment_audit_status: corrected

file: crates/velesdb-core/src/storage/mmap.rs
unsafe_sites: 3
safety_status: covered
must_use_status: not_needed
comment_audit_status: accurate

file: crates/velesdb-core/src/storage/vector_bytes.rs
unsafe_sites: 2
safety_status: needs_fix
must_use_status: not_needed
comment_audit_status: corrected

file: crates/velesdb-core/src/collection/graph/memory_pool.rs
unsafe_sites: 7
safety_status: needs_fix
must_use_status: not_needed
comment_audit_status: corrected

file: crates/velesdb-core/src/index/trigram/simd.rs
unsafe_sites: 5
safety_status: needs_fix
must_use_status: not_needed
comment_audit_status: corrected

file: crates/velesdb-core/src/index/hnsw/index/mod.rs
unsafe_sites: 1
safety_status: covered
must_use_status: not_needed
comment_audit_status: accurate

file: crates/velesdb-core/src/index/hnsw/index/vacuum.rs
unsafe_sites: 1
safety_status: covered
must_use_status: not_needed
comment_audit_status: accurate

file: crates/velesdb-core/src/index/hnsw/vector_store.rs
unsafe_sites: 2
safety_status: needs_fix
must_use_status: not_needed
comment_audit_status: corrected

file: crates/velesdb-core/src/index/hnsw/native_inner.rs
unsafe_sites: 2
safety_status: needs_fix
must_use_status: not_needed
comment_audit_status: corrected
