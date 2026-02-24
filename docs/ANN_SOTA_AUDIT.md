# SIMD / HNSW / AVX State-of-the-Art Audit

Date: 2026-02-24  
Scope: `velesdb-core` SIMD kernels and HNSW implementation.

## Executive conclusion

Short answer: **partially yes**.

- The codebase contains several modern optimizations (runtime SIMD dispatch, AVX2/AVX-512 kernels with FMA, masked tails, prefetch, fused cosine paths, and HNSW diversification hooks).
- But it is **not yet fully state of the art** versus current ANN leaders (Faiss/ScaNN/DiskANN/SPANN style stacks), mainly due to missing algorithmic layers and some implementation bottlenecks.

## What is already strong

1. **SIMD architecture-aware dispatch**
   - Runtime feature detection with cached dispatch (`OnceLock`) and ISA-specific routing (AVX-512, AVX2+FMA, NEON, scalar).
   - Hot paths are inlined and specialized by vector length thresholds.

2. **x86 intrinsics quality**
   - AVX2 and AVX-512 kernels implement multi-accumulator patterns for ILP.
   - Masked-tail AVX-512 handling avoids scalar remainder loops.
   - Fused cosine variants are present for large vectors.

3. **HNSW search-side optimizations**
   - Standard top-down greedy descent + bottom-layer ef search.
   - Multi-entry probing is implemented for harder queries.
   - Software prefetch is used during neighbor expansion for larger dimensions.

4. **Graph diversification support**
   - Neighbor selection includes an alpha-based diversification criterion (VAMANA-style idea).

## Gaps vs state-of-the-art (priority order)

1. **Not a full modern ANN stack yet**
   - No IVF/IMI layer, no graph-on-disk (DiskANN-style), no compressed-routing tiers, and no multi-stage candidate generation pipeline.
   - Current design is primarily HNSW(+quantization), which is strong but not the full SOTA envelope for very large-scale / memory-constrained serving.

2. **Native module maturity ambiguity**
   - `index/hnsw/native/mod.rs` still labels itself as **PROTOTYPE**, which conflicts with “production-grade” positioning and makes maturity status unclear.

3. **Insertion path has avoidable vector cloning**
   - `get_vector()` clones vectors (`Vec<f32>`) and is called repeatedly in insert/neighbor logic.
   - This creates allocation/copy overhead and reduces competitiveness during high-throughput builds.

4. **Lock granularity / contention risk**
   - Core data structures (`vectors`, `layers`) rely heavily on `RwLock` with repeated lock/unlock phases.
   - At high concurrency, this is likely behind specialized lock-free or sharded-graph implementations.

5. **SIMD coverage can be expanded**
   - AVX-512 detection currently hinges on `avx512f`; no explicit exploitation of optional extensions (e.g., AVX-512 VNNI/BW/VL variants) for additional kernels.
   - No FP16/BF16 path for reduced bandwidth compute on capable hardware.

## Recommendation roadmap

### P0 (high impact, low/medium risk)
- Replace clone-heavy `get_vector()` usage in native HNSW hot paths with borrowed/snapshot-based access.
- Profile and rebalance lock scope in insert/search (especially neighbor update paths).
- Clarify production status in docs/comments for native HNSW module.

### P1 (high impact)
- Add optional two-stage ANN mode (coarse quantizer/partition + HNSW rerank).
- Add stronger adaptive query-time policies (`ef_search`, probes, rerank depth) based on latency SLO and query difficulty.

### P2 (hardware frontier)
- Add additional ISA kernels where supported (e.g., FP16/BF16/vectorized quantized-dot paths).
- Consider dimension-specialized codegen (common dims: 384/768/1024/1536) for peak throughput.

## Bottom line

- For a general-purpose Rust vector DB, the current SIMD and HNSW code is **solid and modern**.
- Relative to absolute SOTA in 2025+ ANN systems, it is **one step short**: excellent kernels and graph baseline, but missing some large-scale system-level algorithmic layers and a few hot-path engineering refinements.


## Progress update (after follow-up implementation rounds)

### Implemented from roadmap
- **P0 completed (core items):**
  - Clone-heavy vector retrieval paths in native HNSW were removed/refactored to snapshot/borrow patterns.
  - Lock handling and lock-rank instrumentation were centralized with helper-based read snapshots.
  - In-place neighbor mutation helpers were added to reduce read-clone-write churn and duplicate-edge risk.
  - Native module status language was clarified away from prototype ambiguity.

- **P1 partially completed:**
  - Two-stage ANN behavior (candidate generation + exact SIMD rerank) has been added behind adaptive policy in `HnswIndex`.
  - Query-time adaptation now considers quality profile, `ef_search`, `k`, and dataset scale.

- **P2 partially completed:**
  - Runtime visibility for AVX-512 optional extensions (`VL`, `BW`, `VNNI`) has been added.

### Still open
- Full IVF/partitioned coarse stage (true IVF/IMI) is still not present.
- Disk-backed graph/search path (DiskANN-style) is still not present.
- FP16/BF16 compute paths and dimension-specialized kernel generation are still future work.
- Benchmark-driven auto-tuning (SLO-aware latency/recall controller) should be added to harden adaptive policies.
