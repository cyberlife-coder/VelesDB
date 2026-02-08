# Phase 7: SIMD Tolerance Hardening & DistanceEngine Integration — Context

**Captured:** 2026-02-07

## Vision

No weakness accepted. Every tolerance must be mathematically justified. Every dispatch
overhead must be eliminated if possible. We don't widen tolerances to hide problems —
we fix the root cause.

## User Experience

- Zero flaky tests, ever
- HNSW search as fast as the hardware allows — no wasted cycles on dispatch

## Essentials

Things that MUST be true:

- **Scalar references must use f64 accumulation** — not f32. The f32 scalar `.sum()` is
  NOT a ground truth; it has the same order of error as SIMD. Using f64 gives us a proper
  reference (52-bit mantissa vs 23-bit) against which both SIMD and scalar f32 can be measured.
- **Tolerances must be dynamic and derived from Higham's error bound** —
  `|error| <= γ(N) × Σ|terms|` where `γ(N) = N × ε / (1 - N × ε) ≈ N × f32::EPSILON`.
  This is the standard forward error bound from "Accuracy and Stability of Numerical
  Algorithms" (Higham, 2002). It adapts per test case based on dimension AND values.
- **All 5 distance metrics must use cached SIMD dispatch** in the HNSW hot loop —
  not just Cosine/Euclidean/DotProduct. Hamming and Jaccard must also be cached.
- **The metric transform (1-cosine, -dot) is a single predictable branch** —
  acceptable because the branch predictor will always predict correctly for a given index.
  But the SIMD kernel selection (simd_level + dimension thresholds) must be resolved once.

## Boundaries

Things to explicitly AVOID:

- **DO NOT widen tolerances with arbitrary constants** — every tolerance must trace back
  to a proven error bound
- **DO NOT accept "close enough"** — if the math says the bound should be X, use X
- **DO NOT leave Hamming/Jaccard as second-class citizens** in `DistanceEngine` — extend
  the struct to cache their fn pointers too
- **DO NOT use `Box<dyn Fn>`** for hot-path dispatch — fn pointers or struct fields only

## Implementation Notes

### Plan 07-01: f64 Reference + Higham Bound

1. Rewrite scalar reference functions to accumulate in f64:
   ```rust
   fn scalar_dot_f64(a: &[f32], b: &[f32]) -> (f64, f64) {
       let mut sum = 0.0_f64;
       let mut abs_sum = 0.0_f64;
       for (x, y) in a.iter().zip(b.iter()) {
           let p = f64::from(*x) * f64::from(*y);
           sum += p;
           abs_sum += p.abs();
       }
       (sum, abs_sum)  // (reference, condition_number)
   }
   ```

2. Tolerance per test case:
   ```rust
   let gamma_n = (n as f64) * (f32::EPSILON as f64);
   let allowed = gamma_n * condition_number;
   // Floor at f32::EPSILON for near-zero cases
   let allowed = allowed.max(f32::EPSILON as f64);
   ```

3. Apply same pattern to all 5 metrics (dot, L2, cosine, hamming, jaccard).

### Plan 07-02: Extended DistanceEngine + CachedSimdDistance

1. Extend `simd_native::DistanceEngine` with `hamming_fn` and `jaccard_fn` fields
2. Create `CachedSimdDistance` in `hnsw/native/distance.rs` wrapping the engine
3. Wire into `NativeHnswBackend`

## Open Questions

None — decisions are final.

---
*This context informs planning. The planner will honor these preferences.*
