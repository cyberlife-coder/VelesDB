# Phase 2: PQ Core Engine - Context

**Gathered:** 2026-03-06
**Status:** Ready for planning

<domain>
## Phase Boundary

Product quantization is production-quality internally -- k-means++ codebook training, ADC SIMD
lookup-table kernels (AVX2 + NEON + scalar), OPQ pre-rotation, GPU-accelerated training, and RaBitQ
quantization are all implemented with no changes to any public-facing API. Requirements PQ-01
through PQ-04, plus QUANT-ADV-01 (GPU PQ) and PQ-ADV-01 (RaBitQ) promoted from v2 scope.

VelesQL TRAIN command, QuantizationConfig PQ variant, and recall benchmark suite belong to Phase 3
(PQ Integration). No user-facing API surface is introduced in this phase.

</domain>

<decisions>
## Implementation Decisions

### ADC SIMD Architecture (PQ-02)
- **Dedicated module** `simd_native/adc.rs` -- ADC operates on precomputed lookup tables (LUT), not
  on raw vector-to-vector distances. The compute pattern is fundamentally different from the existing
  `dispatch/` kernels (which operate on paired f32 slices). Mixing them would force awkward
  abstractions.
- **LUT precomputation** stays in `quantization/pq.rs` -- build the `m x k` distance table from the
  query vector and codebook centroids. Returns a flat `Vec<f32>` or `&[f32]` slice.
- **SIMD scan** in `simd_native/adc.rs` -- given a precomputed LUT and a batch of PQ codes (`&[u8]`
  or `&[u16]`), sum the looked-up distances with ISA-specific paths:
  - **AVX2**: `_mm256_i32gather_ps` gather instructions for LUT lookups, 8-wide accumulation.
  - **NEON (aarch64)**: No hardware gather equivalent. Use `vld1q_f32` with manual index extraction
    (`vgetq_lane_u32`) to load 4 LUT entries per iteration, 4-wide NEON accumulation via `vfmaq_f32`.
    Alternative: precompute shuffled LUT tiles per subspace to enable `vtbl`-based permute lookups
    for k <= 16. For k=256 (standard), the index-extract approach is the pragmatic path.
  - **Scalar fallback**: Simple loop for platforms without SIMD or WASM.
- **LUT size constraint** enforced at training time: `m * k * 4 bytes <= 8KB` for default configs
  (m=8, k=256 = 8KB exactly). Warn via `tracing::warn!` if config exceeds L1-friendly size but
  don't reject -- user may have reasons.
- **Batch interface**: `adc_distances_batch(lut: &[f32], codes: &[&[u16]], m: usize) -> Vec<f32>`
  -- processes multiple PQ vectors against one precomputed LUT. This is the hot path for HNSW
  neighbor evaluation during PQ search.

### Rescore Oversampling Defaults (PQ-04)
- **Configurable oversampling factor** with default `4x` -- the current hardcoded `8x` in
  `collection/search/vector.rs:23` is aggressive for the general case. 4x provides good
  recall recovery without excessive rescore cost. Formula: `candidates_k = k * oversampling_factor`.
- **Minimum floor**: `candidates_k = max(k * factor, k + 32)` preserved from current code -- ensures
  meaningful oversampling even for small k values.
- **Config location**: new field `pq_rescore_oversampling: Option<u32>` on the internal collection
  config (not public API -- Phase 3 exposes it). Default `Some(4)`. `None` disables rescore
  (expert-only, not recommended).
- **Rescore always active by default** -- requirement PQ-04 says "silent recall collapse is not
  possible". The oversampling + rerank pipeline in `vector.rs` is the safety net; it must not be
  bypassable without explicit opt-out.

### Codebook Storage Layout
- **Same collection directory**: `<data_dir>/<collection_name>/codebook.pq` -- consistent with
  existing layout (config.json, vectors.bin, hnsw.bin, payloads.log all co-located).
- **Format**: postcard serialization of `PQCodebook` struct (consistent with Phase 1 decision to
  use postcard over bincode). File written atomically (write to `.tmp` then rename).
- **Lifecycle**: codebook is written after `train()` completes. Loaded on `Database::open()` if
  file exists. Deleted on collection drop. No separate "codebook management" -- it's part of the
  collection lifecycle.
- **OPQ rotation matrix** stored alongside: `rotation.opq` in same directory, same postcard format.
  Only present when OPQ is enabled.

### OPQ Pre-Rotation (PQ-03)
- **Dependency**: `ndarray` with `no-default-features` (pure Rust, no BLAS/LAPACK linkage). Binary
  size impact ~500KB -- acceptable against the 15MB budget. Manual matrix rotation would be fragile
  and hard to verify for correctness.
- **Algorithm**: Iterating PQ (IPQ) approach -- alternate between PQ training and SVD-based rotation
  optimization. 5 outer iterations is standard (matches Faiss default).
- **Rotation matrix**: `D x D` orthogonal matrix stored as `ndarray::Array2<f32>`. Applied to query
  vectors before LUT computation and to training vectors before codebook training.
- **Feature-gated**: OPQ behind `#[cfg(feature = "persistence")]` -- no ndarray in WASM builds.
  The `ProductQuantizer` struct gets an `Option<Vec<f32>>` field for the flattened rotation matrix
  (row-major D*D), avoiding ndarray in the core struct serialization.
- **Config flag**: `opq_enabled: bool` on the internal PQ config. Default `false` -- OPQ is opt-in
  because it adds training time and complexity. Phase 3 exposes this via QuantizationConfig.

### k-means Training Quality (PQ-01)
- **Convergence-based** with `max_iters = 50` (up from current 25) and early-stop when centroid
  movement < 1% relative to previous iteration. This matches production PQ implementations
  (Faiss uses 25 default but with optimized Lloyd's; our pure-Rust impl benefits from more iters).
- **Empty cluster re-seeding**: current approach (clone from samples) is adequate. Add split of
  largest cluster as fallback for persistent empty clusters after 3 consecutive empty iterations.
- **Degenerate centroid detection**: after training, verify no two centroids within same subspace
  are closer than `1e-6` L2 distance. Log warning if found but don't fail -- degenerate centroids
  reduce effective k but don't break correctness.
- **Parallelism**: subspace training is embarrassingly parallel. Use `rayon::par_iter` over
  subspaces behind `#[cfg(feature = "persistence")]` (rayon is already a persistence-gated dep).
  Scalar fallback for WASM.

### GPU-Accelerated PQ Training (QUANT-ADV-01 -- promoted from v2)
- **Backend**: `wgpu` (already an optional dependency via `gpu` feature flag). Compute shaders for
  the k-means assignment step (most expensive: N vectors x k centroids x subspace_dim distance
  computations per iteration).
- **Strategy**: GPU accelerates the **assignment step only** -- centroid update step stays on CPU
  (reduction is memory-bound, not compute-bound; CPU is fine). This avoids complex GPU reduction
  kernels and CPU-GPU data ping-pong.
- **Shader**: Single WGSL compute shader `pq_kmeans_assign.wgsl` -- each workgroup processes a
  tile of vectors, computes distances to all k centroids for one subspace, writes nearest centroid
  index to output buffer. Workgroup size 256 (tuned for typical GPUs).
- **Fallback**: If `wgpu` device initialization fails or `gpu` feature is disabled, fall back
  silently to CPU k-means (existing code path). GPU is an acceleration, not a requirement.
- **Feature gate**: `#[cfg(feature = "gpu")]` -- no GPU code in default builds. The `gpu` feature
  already exists in Cargo.toml.
- **Threshold**: Only dispatch to GPU when `N * k * subspace_dim > 10M` FLOPs (roughly N > 5000
  for m=8 k=256 d=12). Below that, CPU is faster due to GPU dispatch overhead.

### RaBitQ Quantization (PQ-ADV-01 -- promoted from v2)
- **Algorithm**: RaBitQ (arXiv:2405.12497) -- randomized binary quantization with theoretical
  guarantees. Encodes each vector as a binary code + scalar correction factors. Achieves comparable
  recall to PQ at 32x compression (vs PQ's 8-16x) with faster distance computation.
- **Implementation scope**: Core encode/decode/distance. Not a replacement for PQ -- a **third
  quantization strategy** alongside SQ8 and PQ, selectable via `StorageMode::RaBitQ`.
- **Key structures**:
  - `RaBitQVector { bits: Vec<u64>, norms: (f32, f32) }` -- binary codes packed in u64 + correction
    scalar pair (norm, residual norm).
  - `RaBitQIndex` -- random rotation matrix (orthogonal, stored as flat `Vec<f32>`), trained once
    per collection.
- **Distance computation**: XOR + popcount on u64 words (Hamming-like), then affine correction
  with stored norms. The existing `hamming_avx2` / `hamming_avx512` kernels in `simd_native/` are
  directly reusable for the XOR+popcount step.
- **Training**: Generate random orthogonal matrix via QR decomposition of random Gaussian matrix
  (using `ndarray` -- already added for OPQ). One-time cost, O(D^2) storage.
- **StorageMode extension**: Add `RaBitQ` variant to the existing `StorageMode` enum in
  `quantization/mod.rs`. The enum is already `#[non_exhaustive]` so this is non-breaking.
- **Recall target**: recall@10 >= 90% at 32x compression on standard benchmarks (SIFT1M-like).
  Below PQ's 92% but at 2-4x better compression ratio.

### Claude's Discretion
- Exact WGSL shader workgroup dimensions and tiling strategy for GPU k-means
- Internal naming conventions for new modules/functions
- Test fixture design for recall property tests
- Convergence metric implementation details (centroid delta computation)
- Temporary file naming for atomic codebook writes
- RaBitQ random matrix generation seed strategy (deterministic vs random per collection)

</decisions>

<code_context>
## Existing Code Insights

### Reusable Assets
- `quantization/pq.rs`: Full PQ pipeline (train, quantize, reconstruct, ADC) -- extend, don't
  rewrite. k-means++ init already production-quality from Phase 1.
- `simd_native/dispatch/`: Runtime SIMD detection (`SimdLevel` enum, `simd_level()` function) --
  reuse for ADC path selection. No new detection logic needed.
- `simd_native/tail_unroll.rs`: Remainder handling macros -- applicable to ADC scan tail.
- `collection/search/vector.rs`: Rescore pipeline with PQ cache + oversampling already functional.
  Refactor oversampling factor from hardcoded to configurable.
- `StorageMode::ProductQuantization`: Already defined in `quantization/mod.rs`.
- `postcard` serialization: Already integrated (Phase 1) -- use for codebook persistence.

### Established Patterns
- `parking_lot::RwLock` for all shared state -- codebook and rotation matrix follow this.
- `#[cfg(feature = "persistence")]` gates all I/O and rayon usage -- OPQ and codebook persistence
  follow this pattern.
- `// SAFETY:` comments required on all unsafe blocks -- ADC SIMD intrinsics must document.
- `// TODO(EPIC-063):` for any deferred work within this phase.
- Error handling via `thiserror` + `VelesError` with `?` propagation.

### Integration Points
- `collection/search/vector.rs:23` -- oversampling factor to make configurable.
- `collection/types.rs` -- Collection struct holds `pq_cache` and `pq_quantizer` fields.
- `database.rs` -- `Database::open()` auto-loads collections; codebook loading hooks in here.
- `simd_native/mod.rs` -- new `adc` submodule to register and re-export.
- `simd_native/x86_avx2_similarity.rs` -- existing `hamming_avx2` reusable for RaBitQ XOR+popcount.
- `simd_native/x86_avx512.rs` -- existing `hamming_avx512` reusable for RaBitQ.
- `quantization/mod.rs` -- `StorageMode` enum (`#[non_exhaustive]`) -- add `RaBitQ` variant.
- `Cargo.toml` (velesdb-core) -- add `ndarray` dep (persistence-gated), `wgpu` already present
  (gpu-gated).

</code_context>

<specifics>
## Specific Ideas

- User explicitly delegated all decisions: "En tant qu'expert Rust et Craftsman accompagne des
  experts algorithmiques et systemes, effectue les meilleures decisions possibles." All choices
  above reflect production PQ best practices (Faiss, ScaNN, Qdrant references).
- LUT must fit L1 cache: m=8 k=256 = 8KB is the sweet spot. This is a hard constraint from
  the success criteria, not a soft preference.
- The 15MB binary target is a product promise -- ndarray no-default-features (~500KB) is acceptable
  but heavy BLAS linking is not.
- Rescore oversampling is a safety net against "silent recall collapse" -- it must be on by default
  and require explicit opt-out, never implicit.

</specifics>

<deferred>
## Deferred Ideas

- Configurable distance metrics for ADC (currently L2 only for LUT path; cosine/dot go through
  reconstruct) -- optimize in Phase 3 or later if profiling shows bottleneck.
- WAND-accelerated RaBitQ search for very large collections (>1M vectors) -- standard scan is
  sufficient for v1.5 target workloads.

</deferred>

---

*Phase: 02-pq-core-engine*
*Context gathered: 2026-03-06*
