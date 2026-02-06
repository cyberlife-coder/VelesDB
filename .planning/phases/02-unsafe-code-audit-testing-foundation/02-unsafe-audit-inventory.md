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
safety_status: fixed
must_use_status: not_needed
comment_audit_status: corrected
must_use_rationale: Constructors/accessors already enforce usage via existing attributes and side-effect APIs.

file: crates/velesdb-core/src/perf_optimizations.rs
unsafe_sites: 11
safety_status: fixed
must_use_status: not_needed
comment_audit_status: corrected
must_use_rationale: Return-value-significant APIs already annotated; mutating APIs are side-effect-driven.

file: crates/velesdb-core/src/simd_native.rs
unsafe_sites: 61
safety_status: fixed
must_use_status: not_needed
comment_audit_status: corrected
must_use_rationale: Hot-path return APIs already carry must_use; unsafe internals are implementation details.

file: crates/velesdb-core/src/simd_neon.rs
unsafe_sites: 9
safety_status: fixed
must_use_status: added
comment_audit_status: corrected
must_use_rationale: NEON score-returning APIs now warn when discarded.

file: crates/velesdb-core/src/simd_neon_prefetch.rs
unsafe_sites: 6
safety_status: fixed
must_use_status: not_needed
comment_audit_status: corrected
must_use_rationale: Prefetch APIs are side-effect hints; ignoring return is intentional (unit type).

file: crates/velesdb-core/src/storage/guard.rs
unsafe_sites: 3
safety_status: fixed
must_use_status: not_needed
comment_audit_status: corrected
must_use_rationale: Key slice accessor already marked must_use; other methods are trait glue.

file: crates/velesdb-core/src/storage/compaction.rs
unsafe_sites: 4
safety_status: fixed
must_use_status: not_needed
comment_audit_status: corrected
must_use_rationale: Operational APIs are effectful maintenance functions, not value-only results.

file: crates/velesdb-core/src/storage/mmap.rs
unsafe_sites: 3
safety_status: covered
must_use_status: not_needed
comment_audit_status: accurate
must_use_rationale: Public ratio/metrics APIs already marked must_use where applicable.

file: crates/velesdb-core/src/storage/vector_bytes.rs
unsafe_sites: 2
safety_status: fixed
must_use_status: not_needed
comment_audit_status: corrected
must_use_rationale: Conversion utility outputs are consumed immediately by storage call paths.

file: crates/velesdb-core/src/collection/graph/memory_pool.rs
unsafe_sites: 7
safety_status: fixed
must_use_status: not_needed
comment_audit_status: corrected
must_use_rationale: Value-returning helper accessors already annotated in pool handle APIs.

file: crates/velesdb-core/src/index/trigram/simd.rs
unsafe_sites: 5
safety_status: fixed
must_use_status: not_needed
comment_audit_status: corrected
must_use_rationale: Trigram extraction/count APIs already marked must_use.

file: crates/velesdb-core/src/index/hnsw/index/mod.rs
unsafe_sites: 1
safety_status: covered
must_use_status: not_needed
comment_audit_status: accurate
must_use_rationale: Structural drop-safety module; no additional return-value-significant public APIs.

file: crates/velesdb-core/src/index/hnsw/index/vacuum.rs
unsafe_sites: 1
safety_status: covered
must_use_status: not_needed
comment_audit_status: accurate
must_use_rationale: Vacuum helpers already apply must_use to ratio/tombstone inspection APIs.

file: crates/velesdb-core/src/index/hnsw/vector_store.rs
unsafe_sites: 2
safety_status: fixed
must_use_status: not_needed
comment_audit_status: corrected
must_use_rationale: VectorRef::as_slice already marked must_use; mutators are side-effect APIs.

file: crates/velesdb-core/src/index/hnsw/native_inner.rs
unsafe_sites: 2
safety_status: fixed
must_use_status: not_needed
comment_audit_status: corrected
must_use_rationale: Value-returning APIs already marked must_use in public wrapper surface.

## Verification Evidence

- `cargo test -p velesdb-core simd_native_tests`: pass (66 tests)
- `cargo test -p velesdb-core storage::tests`: pass (33 tests)
- `cargo clippy -p velesdb-core -- -D warnings`: pass
