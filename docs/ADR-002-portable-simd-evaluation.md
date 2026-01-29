# ADR-002: portable_simd Evaluation

## Status

**EVALUATED** - 2026-01-29

## Context

VelesDB currently uses architecture-specific SIMD intrinsics (AVX2 for x86_64, NEON for ARM64) for vector distance calculations. This requires maintaining separate code paths and manual dispatch logic.

Rust's `portable_simd` feature (nightly) offers a unified API that compiles to optimal SIMD for each target.

## Evaluation

### Implementation (EPIC-054/US-004)

Created `simd_portable.rs` with:
- `l2_distance_portable()` - L2 Euclidean distance
- `dot_product_portable()` - Dot product
- `cosine_similarity_portable()` - Cosine similarity
- `l2_squared_portable()` - Squared L2 (no sqrt)

All functions use `f32x8` lanes (256-bit on AVX2, 2x128-bit on NEON).

### Code Comparison

| Aspect | Current Intrinsics | portable_simd |
|--------|-------------------|---------------|
| **L2 distance code** | ~45 lines (AVX2) + ~40 lines (NEON) | ~25 lines |
| **Total SIMD code** | ~400 lines across 4 files | ~100 lines single file |
| **Code reduction** | Baseline | **~75%** |
| **Dispatch logic** | Manual `#[cfg(target_arch)]` | Automatic |
| **FMA utilization** | Manual intrinsics | Automatic |

### Performance (Theoretical)

| Target | Expected Performance |
|--------|---------------------|
| x86_64 (AVX2) | ~95-100% of intrinsics |
| ARM64 (NEON) | ~95-100% of intrinsics |
| WASM (SIMD128) | ~90% of native |

*Note: Actual benchmarks require running on each target architecture.*

### Stability Assessment

| Criterion | Status |
|-----------|--------|
| **Nightly required** | ✅ Yes (tracking issue: rust-lang/rust#86656) |
| **Stabilization ETA** | ~2026 Q3 (estimated) |
| **API stability** | Mostly stable, minor changes possible |
| **WASM support** | ✅ Works with `wasm32` target |

## Decision

### Recommendation: **ADOPT FOR NEW CODE**

**Rationale:**
1. **75% code reduction** exceeds 40% target
2. **Cross-platform by default** - single implementation
3. **FMA automatically utilized** - no manual intrinsics needed
4. **WASM compatibility** - critical for browser deployments

### Migration Strategy

1. **Phase 1 (Current)**: Parallel implementation alongside intrinsics
2. **Phase 2 (Post-stabilization)**: Gradual migration of hot paths
3. **Phase 3 (v2.0)**: Full replacement, intrinsics as fallback only

### Risk Mitigation

| Risk | Mitigation |
|------|------------|
| API changes before stabilization | Feature-gated, easy to update |
| Performance regression | Benchmark suite per-architecture |
| Nightly dependency | Fallback to scalar implementation |

## Consequences

### Positive
- Simpler codebase
- Automatic cross-platform optimization
- Easier maintenance and testing
- Better WASM performance

### Negative
- Nightly toolchain for development
- Slight learning curve for contributors
- Need to wait for stabilization for production use

## Files

- `crates/velesdb-core/src/simd_portable.rs` - Implementation
- `crates/velesdb-core/benches/portable_simd_eval.rs` - Benchmarks
- Feature flag: `portable-simd` in `Cargo.toml`

## Verification

```bash
# Run tests (works without nightly - uses scalar fallback)
cargo test -p velesdb-core simd_portable

# Run benchmarks (requires nightly + feature)
cargo +nightly bench --features portable-simd --bench portable_simd_eval
```

## References

- [rust-lang/portable-simd](https://github.com/rust-lang/portable-simd)
- [Tracking issue #86656](https://github.com/rust-lang/rust/issues/86656)
- [std::simd documentation](https://doc.rust-lang.org/nightly/std/simd/index.html)
