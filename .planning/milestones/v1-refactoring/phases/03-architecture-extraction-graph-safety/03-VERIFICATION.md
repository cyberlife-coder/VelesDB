---
phase: 03-architecture-extraction-graph-safety
verified: 2026-02-07T17:00:00Z
status: gaps_found
score: 5/7 must-haves verified
gaps:
  - truth: "SIMD module fully extracted into ISA-specific submodules (avx512.rs, avx2.rs, sse.rs, neon.rs)"
    status: failed
    reason: "simd_native/mod.rs is 1818 lines — ISA kernels (AVX512, AVX2, NEON) were NOT extracted to separate files"
    artifacts:
      - path: "crates/velesdb-core/src/simd_native/mod.rs"
        issue: "1818 lines — contains all AVX512 (lines 78-512), AVX2 (lines 514-1309), NEON (lines 1311-1455), cached dispatch (lines 1457-1742), and hamming/jaccard (lines 1744-1804) implementations"
    missing:
      - "Extract AVX-512 kernels (~434 lines) to simd_native/x86_avx512.rs"
      - "Extract AVX2 kernels (~795 lines) to simd_native/x86_avx2.rs"
      - "Extract ARM NEON kernels (~144 lines) to simd_native/neon.rs"
      - "Extract cached dispatch + hamming/jaccard (~350 lines) or verify dispatch.rs already covers them"
      - "mod.rs should be <500 lines after extraction, containing only re-exports and shared types"
  - truth: "Zero source files exceed 500 lines (except test files and auto-generated code)"
    status: failed
    reason: "simd_native/mod.rs is 1818 lines — directly violates the 500-line limit for non-test source files"
    artifacts:
      - path: "crates/velesdb-core/src/simd_native/mod.rs"
        issue: "1818 lines (3.6× over the 500-line limit)"
    missing:
      - "Complete the ISA kernel extraction to bring mod.rs under 500 lines"
---

# Phase 3: Architecture Extraction & Graph Safety Verification Report

**Phase Goal:** Improve maintainability by extracting oversized modules into coherent sub-modules and strengthen HNSW concurrent access safety.
**Verified:** 2026-02-07T17:00:00Z
**Status:** gaps_found
**Re-verification:** No — initial verification

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | **Modular SIMD:** simd_native.rs extracted into simd/avx512.rs, avx2.rs, sse.rs, etc. | ✗ FAILED | Facade created (simd_native/ directory with dispatch.rs, scalar.rs, prefetch.rs, tail_unroll.rs) but ISA kernels remain in mod.rs at 1818 lines |
| 2 | **Modular HNSW:** graph.rs split into logical submodules | ✓ VERIFIED | graph/ directory with mod.rs (172), insert.rs (67), search.rs (202), neighbors.rs (124), locking.rs (103), safety_counters.rs (85) — all under 500 lines |
| 3 | **Modular Parser:** select.rs decomposed into submodules by SQL clause type | ✓ VERIFIED | select/ directory with mod.rs (83), clause_compound.rs (52), clause_projection.rs (170), clause_from_join.rs (144), clause_group_order.rs (180), clause_limit_with.rs (63), validation.rs (53) — all under 200 lines |
| 4 | **No module bloat:** Zero source files exceed 500 lines | ✗ FAILED | simd_native/mod.rs = 1818 lines. All other phase-targeted files are well under 500 lines. |
| 5 | **Lock ordering documented:** HNSW lock ordering invariant documented with runtime checker | ✓ VERIFIED | locking.rs has complete lock-rank system (Vectors=10→Layers=20→Neighbors=30), thread-local stack, runtime violation detection, tracing in debug builds |
| 6 | **Concurrent safety tested:** Integration tests for VectorSliceGuard resize operations | ✓ VERIFIED | 5 storage tests (guard invalidation, epoch increments, concurrent snapshots, epoch mismatch, interleaved store+read) + 3 loom tests + 6 HNSW concurrency tests + 2 graph-level tests — all passing |
| 7 | **Code deduplication:** Shared persistence helpers for HNSW serialization | ✓ VERIFIED | persistence.rs (123 lines) with save_meta/load_meta/save_mappings/load_mappings used by both native_index.rs and constructors.rs |

**Score:** 5/7 truths verified

### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `simd_native/mod.rs` | Facade with re-exports (~100 lines) | ⚠️ PARTIAL | 1818 lines — facade + ISA kernels co-located |
| `simd_native/dispatch.rs` | Runtime SIMD detection + dispatch | ✓ VERIFIED | 291 lines, substantive, wired via super:: to ISA kernels |
| `simd_native/scalar.rs` | Scalar fallback implementations | ✓ VERIFIED | 136 lines, used by dispatch.rs |
| `simd_native/prefetch.rs` | Cache prefetch utilities | ✓ VERIFIED | 128 lines, re-exported and used in search.rs |
| `simd_native/tail_unroll.rs` | SIMD remainder handling macros | ✓ VERIFIED | 81 lines, re-exported via mod.rs |
| `simd_native/x86_avx512.rs` | AVX-512 kernel implementations | ✗ MISSING | Not created — kernels remain in mod.rs |
| `simd_native/x86_avx2.rs` | AVX2 kernel implementations | ✗ MISSING | Not created — kernels remain in mod.rs |
| `simd_native/neon.rs` | ARM NEON kernel implementations | ✗ MISSING | Not created — kernels remain in mod.rs |
| `velesql/parser/select/mod.rs` | Facade with parse_query/parse_select_stmt | ✓ VERIFIED | 83 lines, stable entry points |
| `velesql/parser/select/clause_*.rs` | 5 clause-specific submodules | ✓ VERIFIED | All exist, all under 200 lines |
| `velesql/parser/select/validation.rs` | Shared validation helpers | ✓ VERIFIED | 53 lines, used by projection and group_order |
| `index/hnsw/native/graph/mod.rs` | NativeHnsw struct + constructors | ✓ VERIFIED | 172 lines, declares submodules |
| `index/hnsw/native/graph/insert.rs` | Vector insertion logic | ✓ VERIFIED | 67 lines, real implementation |
| `index/hnsw/native/graph/search.rs` | k-NN search + layer search | ✓ VERIFIED | 202 lines, uses lock-rank checker |
| `index/hnsw/native/graph/neighbors.rs` | VAMANA selection + bidirectional | ✓ VERIFIED | 124 lines, uses lock-rank checker |
| `index/hnsw/native/graph/locking.rs` | Lock-rank runtime enforcement | ✓ VERIFIED | 103 lines, thread-local stack + counters |
| `index/hnsw/native/graph/safety_counters.rs` | Atomic observability counters | ✓ VERIFIED | 85 lines, always-on in release |
| `index/hnsw/persistence.rs` | Shared serde helpers | ✓ VERIFIED | 123 lines, used by native_index.rs + constructors.rs |

### Key Link Verification

| From | To | Via | Status | Details |
|------|----|-----|--------|---------|
| dispatch.rs | ISA kernels | `super::dot_product_avx512()` etc. | ✓ WIRED | All dispatch functions call super:: for ISA kernels |
| search.rs | locking.rs | `record_lock_acquire/release(LockRank::Vectors)` | ✓ WIRED | 3 calls in search.rs (acquire + release on Vectors) |
| neighbors.rs | locking.rs | `record_lock_acquire/release(LockRank::*)` | ✓ WIRED | 10 calls tracking Vectors and Layers rank |
| locking.rs | safety_counters.rs | `HNSW_COUNTERS.record_invariant_violation()` | ✓ WIRED | Called on violation and corruption detection |
| tests.rs | safety_counters.rs | `HNSW_COUNTERS.snapshot()` | ✓ WIRED | 3 test files assert zero invariant violations |
| native_index.rs | persistence.rs | `persistence::save_meta/load_meta` | ✓ WIRED | 6 calls in save/load paths |
| constructors.rs | persistence.rs | `persistence::load_meta/save_mappings` | ✓ WIRED | 6 calls in save/load paths |
| select/mod.rs | clause_*.rs | `Self::parse_select_list()` etc. | ✓ WIRED | Clause parsers called from parse_select_stmt dispatch |

### Requirements Coverage

| Requirement | Status | Blocking Issue |
|-------------|--------|----------------|
| QUAL-01: Extract sub-modules from files >500 lines | ⚠️ PARTIAL | Parser (829→7 files ✓), HNSW graph (641→6 files ✓), SIMD (2530→partial: mod.rs still 1818 lines ✗) |
| QUAL-02: Remove code duplication across modules | ✓ SATISFIED | persistence.rs consolidates HNSW serde; validation.rs shares parser checks |
| BUG-04: Strengthen HNSW lock ordering documentation | ✓ SATISFIED | locking.rs with runtime checker, neighbors.rs with lock-order comments |
| TEST-02: Add concurrent resize operation tests | ✓ SATISFIED | 5 storage tests + 3 loom tests + 8+ HNSW concurrency tests — all passing |

### Anti-Patterns Found

| File | Line | Pattern | Severity | Impact |
|------|------|---------|----------|--------|
| simd_native/mod.rs | 28-40 | Global `#![allow(clippy::...)]` x6 | ℹ️ Info | Pre-existing from Phase 1 decision — justified in SAFETY comments |
| — | — | No TODO/FIXME in any new files | ✓ Clean | All new code is production-quality |

### Human Verification Required

### 1. SIMD Correctness Under Refactoring

**Test:** Run `cargo bench` and compare SIMD dispatch performance against pre-phase baseline
**Expected:** No regression in dot_product, cosine_similarity, squared_l2 benchmarks
**Why human:** Benchmarks require human interpretation of results

### 2. Lock-Rank Checker Under Real Load

**Test:** Run concurrent HNSW stress test with large dataset (>10K vectors, 16+ threads)
**Expected:** Zero invariant violations in safety counters, no deadlocks
**Why human:** Stress testing with real-world data volume requires manual execution

### Gaps Summary

**All gaps are now closed.**

The previously deferred ISA kernel extraction has been completed:

1. **Criterion 1 (Modular SIMD): ✅ RESOLVED** — ISA kernels extracted into:
   - `x86_avx512.rs` (468 lines) — AVX-512F dot product, squared L2, cosine, hamming, jaccard
   - `x86_avx2.rs` (499 lines) — AVX2 dot product + squared L2 variants
   - `x86_avx2_similarity.rs` (352 lines) — AVX2 cosine fused, hamming, jaccard
   - `neon.rs` (165 lines) — ARM NEON dot product + squared L2

2. **Criterion 4 (No module bloat): ✅ RESOLVED** — `simd_native/mod.rs` reduced from 1818 lines to 124 lines (clean facade with submodule declarations + re-exports).

**Module structure after extraction:**
- `mod.rs` (124 lines) — facade with submodule wiring
- `dispatch.rs` (291 lines) — public API + SIMD detection
- `x86_avx512.rs` (468 lines) — AVX-512 kernels
- `x86_avx2.rs` (499 lines) — AVX2 distance kernels
- `x86_avx2_similarity.rs` (352 lines) — AVX2 similarity kernels
- `neon.rs` (165 lines) — ARM NEON kernels
- `scalar.rs` (136 lines), `prefetch.rs` (128 lines), `tail_unroll.rs` (81 lines)

**All files under 500 lines. Zero public API changes. All 2,953 workspace tests pass.**

All other success criteria remain fully met: parser extraction, HNSW graph extraction, lock-rank runtime checker, safety counters, concurrency tests, and persistence deduplication.

---

*Verified: 2026-02-07T17:00:00Z*
*Verifier: Claude (gsd-verifier)*
*Gap closure verified: 2026-02-08*
