# Debug: SIMD Orphan Code, Dead Code & Comment Accuracy Audit

**Started:** 2026-02-07T21:14:00+01:00
**Status:** investigating

## Symptoms

1. `simd_native/mod.rs` is 1604 lines — ISA extraction files exist but are orphaned (not declared as submodules)
2. Potential dead/orphan code across velesdb-core after multiple refactoring phases
3. Comments may not match code behavior after module splitting

## Evidence Collected

- `simd_native/mod.rs`: 1604+ lines with inline AVX-512, AVX2, NEON kernels
- Orphaned files: `x86_avx512.rs` (396L), `x86_avx2.rs` (423L), `x86_avx2_similarity.rs` (298L), `neon.rs` (134L)
- These files are NOT declared as `mod` in `mod.rs` — they are dead code
- Root cause: develop→main merge brought back old inline kernels

## Hypotheses

### H1: Orphaned ISA files are duplicates of inline code in mod.rs
**Test:** Compare function signatures in both locations
**Status:** testing

### H2: mod.rs inline kernels are authoritative (actively compiled)
**Test:** Check which functions dispatch.rs calls
**Status:** testing

## Timeline

| Time | Action | Result |
|------|--------|--------|
| 21:14 | Audit started | mod.rs 1604 lines, 4 orphaned ISA files |

## Resolution

**Root cause:** The develop→main merge brought back the old `mod.rs` with inline ISA kernels (1604 lines). The Phase 3 extraction files existed (`x86_avx512.rs`, `x86_avx2.rs`, `x86_avx2_similarity.rs`, `neon.rs`, `dispatch.rs`) but were never wired as submodules — they were orphaned dead code. Additionally, the orphaned files were stale (pre-Phase 4), missing the `dot_avx2_tail_under16` and `dot_avx2_remainder` helpers.

**Fix:**
1. Deleted all 5 stale orphaned ISA files
2. Extracted fresh ISA submodules from authoritative `mod.rs`:
   - `x86_avx512.rs` (396 lines) — AVX-512 dot, L2, cosine, hamming, jaccard
   - `x86_avx2.rs` (472 lines) — AVX2 dot product + squared L2 (includes Phase 4 helpers)
   - `x86_avx2_similarity.rs` (296 lines) — AVX2 cosine fused, hamming, jaccard
   - `neon.rs` (134 lines) — ARM NEON dot product + squared L2
   - `dispatch.rs` (331 lines) — SimdLevel detection + public dispatch API
3. Rewrote `mod.rs` as thin facade (105 lines) with submodule declarations and re-exports
4. Fixed `similar_names` clippy lint in `scalar.rs` (previously suppressed by old mod.rs blanket allow)
5. Ran `cargo fmt` to fix formatting

**Verification:**
- `cargo fmt --all --check` → clean
- `cargo clippy --workspace -- -D warnings` → 0 errors
- `cargo test -p velesdb-core --lib` → 2382 passed, 0 failed
- `cargo clippy -- -W dead_code` → 0 dead code warnings
- All files under 500 lines (QUAL-01 fully satisfied)
- No stale comments found

**Prevention:** Ensure ISA submodule wiring is verified after any merge that touches `simd_native/`.
