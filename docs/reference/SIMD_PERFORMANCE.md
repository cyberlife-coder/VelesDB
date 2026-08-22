# SIMD Performance Guide

VelesDB uses **native SIMD dispatch** for ultra-fast vector operations, automatically selecting the optimal implementation based on CPU features and vector size.

## Native SIMD Architecture (EPIC-052/077)

The `simd_native` module provides hand-tuned SIMD implementations using `core::arch` intrinsics:

```
┌─────────────────────────────────────────────────────────────────┐
│              simd_native::cosine_similarity_native()             │
│                                                                  │
│  Runtime: feature detection → tiered dispatch → native SIMD     │
│  - AVX-512: 8/4-acc + masked single-acc based on size           │
│  - AVX2: 4-acc (>=256), plain (>=64), 1-acc (>=8)               │
│  - ARM NEON: 128-bit SIMD                                       │
│  - Scalar: fallback for small vectors                           │
└─────────────────────────────────────────────────────────────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
  ┌───────────┐        ┌───────────┐        ┌───────────┐
  │ AVX-512   │        │ AVX2/FMA  │        │  Scalar   │
  │ (512-bit) │        │ (256-bit) │        │ (native)  │
  └───────────┘        └───────────┘        └───────────┘
```

## Architecture Support

| Platform | Implementation | Instructions | Performance (768D) |
|----------|----------------|-------------|-------------------|
| **x86_64 AVX-512** | simd_native | 512-bit 8/4-acc | ~38-42ns |
| **x86_64 AVX2+FMA** | simd_native | 256-bit 4/1-acc | ~40-82ns |
| **aarch64** | simd_native | NEON 128-bit | ~60-100ns |
| **WASM** | scalar fallback (SIMD128 planned) | Native Rust | see Fallback |
| **Fallback** | Scalar | Native Rust | ~150-200ns |

### Tiered Dispatch Strategy (EPIC-077)

Implementations adapt based on vector size and ISA to minimize register pressure and maximize throughput:

**AVX-512 (cosine):**

| Size Range | Accumulators | Use Case |
|------------|--------------|----------|
| >= 1024 elements | 8-acc | Very large vectors (text-embedding-3-large) |
| 512-1023 elements | 4-acc | Large vectors (ada-002) |
| 16-511 elements | single fused kernel | Medium vectors (BERT, MiniLM) |
| < 16 elements | Scalar | Tiny vectors |

**AVX-512 (dot product, squared L2):**

| Size Range | Accumulators | Use Case |
|------------|--------------|----------|
| >= 1024 elements | 8-acc | Very large vectors |
| 512-1023 elements | 4-acc | Large vectors |
| < 512 elements | masked single-acc kernel (no minimum) | All other sizes |

**AVX2 (cosine):**

| Size Range | Accumulators | Use Case |
|------------|--------------|----------|
| >= 512 elements | 4-acc (12 ymm regs) | Large vectors |
| 8-511 elements | 2-acc (6 ymm regs) | Medium/small vectors |
| < 8 elements | Scalar | Tiny vectors |

**AVX2 (dot product, squared L2):**

| Size Range | Accumulators | Use Case |
|------------|--------------|----------|
| >= 256 elements | 4-acc | Large vectors |
| 64-255 elements | plain 8-wide kernel | Medium vectors |
| 8-63 elements | 1-acc | Small vectors |
| < 8 elements | Scalar | Tiny vectors |

All AVX2 cosine kernels use vectorized 8-wide remainder handling, reducing the
scalar tail from up to 31 elements to at most 7. AVX-512 kernels use masked
loads for zero-cost remainder.

## Dispatch Registry

What actually serves each operation on each target, from an audit of
`crates/velesdb-core/src/simd_native/` (2026-08-18). This section is the
completion of the two tables above: those answer "which accumulator count at
which dimension" for the three hot op families; this one answers "which kernel
class serves op X on target Y" for everything.

### Runtime detection

- `SimdLevel` (`simd_native/dispatch/mod.rs`) has four variants — `Avx512`,
  `Avx2`, `Neon`, `Scalar` — detected **once** and cached in a `OnceLock`.
- **x86_64**: `avx512f` → `Avx512`; else `avx2` **and** `fma` (both required)
  → `Avx2`; else `Scalar`. An AVX2-without-FMA machine runs scalar.
- **aarch64**: always `Neon` (architecturally guaranteed, no probe).
- **wasm32 and everything else**: `Scalar`.
- Sub-feature probes (`avx512vl`, `avx512bw`, `avx512vnni`,
  `avx512vpopcntdq`) are separate, per-call checks. Only `vpopcntdq` is
  consumed by a kernel (binary Hamming). **VNNI is detected but no kernel
  uses it yet.**
- Two dispatch styles coexist: the `*_native` free functions re-match the
  level on every call, and `DistanceEngine` resolves five fn pointers once at
  construction — the HNSW hot loop uses the latter.
- The trigram index carries its **own independent detector**
  (`index/trigram/simd.rs`), uncached and with different gates (AVX-512 needs
  `avx512f+avx512bw`; AVX2 does not require FMA).

### Operation × target matrix

| Operation | AVX-512F | AVX2+FMA | NEON | Scalar |
|---|---|---|---|---|
| dot product | 8/4-acc + masked (any dim) | 4/1-acc, >= 8 | 4-acc, >= 4 | yes |
| squared L2 / euclidean | same ladder (`sqrt` on top) | same | >= 4 | yes |
| cosine (fused) | 8/4-acc/plain, >= 16 | 4/2-acc, >= 8 | >= 4 | yes |
| cosine (normalized) | = dot product (alias) | = dot | = dot | yes |
| Hamming (f32) | 4-acc >= 512, plain >= 16 — **no 8-acc tier** | >= 8 | >= 4 | yes |
| Jaccard | 8/4-acc/plain, >= 16 | >= 8 | >= 4 | yes |
| binary Hamming (packed u64) | `vpopcntdq` kernel when detected, else AVX-512F, >= 8 | >= 4 | `vcnt`, >= 2 | yes |
| ADC (PQ table scan) | shares the AVX2 gather kernel | 8-subspace `i32gather`, >= threshold | 4-wide (scalar gathers + vector add) | yes |
| scale / normalize in place | none (deliberate — AVX-512 warmup cost) | >= 8 | **none — falls to scalar** | yes |
| batch variants (all ops) | prefetch loop over the single-pair kernel — no batch-specific SIMD kernels exist | same | same | same |
| prefetch (f32/u16/u64, multi-line) | `_mm_prefetch` T0/T1/T2 | same | inline-asm / `prefetch_read_l1` | no-op |
| trigram extract / match count | **nominal only** — scalar loops behind `#[target_feature]` shells | nominal only | cache-warmup load, then scalar | yes |
| SQ8 quantized dot/L2/cosine | — | — | — | **unrolled scalar everywhere** (no VNNI, no `sdot`/`udot`) |
| f16 / bf16 | — | — | — | scalar convert to f32, then the f32 kernels above (no F16C, no FP16 arithmetic) |

RaBitQ is the exception on the quantized side: its Hamming step routes through
`hamming_binary_native`, so it gets the full binary-Hamming ladder including
`vpopcntdq`.

### GPU (feature `gpu`, wgpu)

Wired at runtime, each behind a size threshold so small workloads never pay
dispatch cost:

| Path | Threshold |
|---|---|
| PQ k-means assignment (training) | `n * k * subspace_dim > 10M` |
| HNSW layer-0 traversal | `> 500K vectors` and `vectors * dim <= u32::MAX` |
| HNSW rerank | `rerank_k * dim > 262144` |
| Brute-force batch distances | whenever a device is available |

Cosine / Euclidean / Dot only — **Hamming, Jaccard, RaBitQ traversal, and the
ADC scan always stay CPU**. Upper-layer HNSW descent is CPU even on the GPU
path. Feature passthrough: `velesdb-python`, `velesdb-server`, `velesdb-cli`,
`velesdb-mobile`, `tauri-plugin-velesdb` expose `gpu`; `velesdb-node` cannot
(license boundary — it has no `velesdb-core` dependency), `velesdb-wasm` and
`velesdb-memory` deliberately do not (browser target; daemon bottleneck is
inference).

### Known gaps (each needs a before/after measurement to close — #1965)

- `scale_inplace` has no NEON kernel: aarch64 normalization is scalar.
- f32 Hamming lacks the AVX-512 8-acc tier its Jaccard sibling has.
- AVX-512 VNNI detected but unused; int8/SQ8 paths are scalar on every ISA.
- Trigram "SIMD" is nominal — the loops are scalar behind feature shells.
- wasm is pure scalar (`simd128` not used); all wasm distance math delegates
  to core's `DistanceMetric::calculate`, so a core `simd128` path would light
  the browser up without touching `velesdb-wasm`.
- `simd_neon.rs` is a legacy, unused duplicate of the live NEON kernels in
  `simd_native/neon.rs` (removal candidate).
- `velesdb simd info` prints a static summary rather than the detected
  runtime level.

## Performance Benchmarks (March 27, 2026)

### Distance Functions (768D vectors)

| Function | Latency | Throughput | vs Previous |
|----------|---------|------------|-------------|
| `dot_product_native` | **19.8ns** | 38.8 Gelem/s | Baseline |
| `euclidean_native` | **22.5ns** | 34.1 Gelem/s | Improved |
| `cosine_similarity_native` | **33.1ns** | 23.2 Gelem/s | Optimized (4-acc, single-sqrt finish) |
| `cosine_normalized_native` | **19.8ns** | 38.8 Gelem/s | Same as dot |
| `hamming_distance_native` | **35.8ns** | 21.5M ops/s | FP-domain 4-acc (no cross-domain penalty) + NEON + batch |
| `jaccard_similarity_native` | **35.1ns** | 21.9 Gelem/s | Optimized (4-acc + NEON + batch) |

*Measured March 27, 2026 on i9-14900KF (24C/32T, AVX2+FMA), 64GB DDR5, Rust 1.92.0, Windows 11 Pro, sequential run on idle machine.*

### Scaling by Dimension (simd_native)

| Dimension | Cosine | Dot Product | Model |
|-----------|--------|-------------|-------|
| 128 | 8.1ns | 5.4ns | MiniLM |
| 384 | 20.1ns | 12.0ns | all-MiniLM-L6-v2 |
| 768 | 33.1ns | 19.8ns | BERT, ada-002 |
| 1536 | 69.0ns | 43.8ns | text-embedding-3-small |
| 3072 | 112.2ns | 91.2ns | text-embedding-3-large |

## Optimization Techniques

### 1. 32-Wide Unrolling (4x f32x8)

```rust
// 4 parallel accumulators for maximum ILP
let mut sum0 = f32x8::ZERO;
let mut sum1 = f32x8::ZERO;
let mut sum2 = f32x8::ZERO;
let mut sum3 = f32x8::ZERO;

for i in 0..simd_len {
    let offset = i * 32;
    sum0 = va0.mul_add(vb0, sum0);
    sum1 = va1.mul_add(vb1, sum1);
    sum2 = va2.mul_add(vb2, sum2);
    sum3 = va3.mul_add(vb3, sum3);
}
```

**Why it works:**
- Modern CPUs have 4+ FMA units (Zen 3+, Alder Lake+)
- Out-of-order execution can run all 4 accumulators in parallel
- ~15-20% faster than single-accumulator SIMD

### 2. Pre-Normalized Vectors

For cosine similarity with pre-normalized vectors:

```rust
// Standard cosine: fused single pass over dot and both norms
pub fn cosine_similarity_native(a: &[f32], b: &[f32]) -> f32;

// Normalized: 1 pass (dot only) - 40% faster!
pub fn cosine_normalized_native(a: &[f32], b: &[f32]) -> f32;
```

**Use when:**
- Vectors are normalized at insertion time
- Same vector is compared multiple times
- Building custom distance functions

### 3. CPU Prefetch Hints

```rust
// Prefetch next vectors into L1 cache
#[cfg(target_arch = "x86_64")]
unsafe {
    use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
    _mm_prefetch(next_vector.as_ptr().cast::<i8>(), _MM_HINT_T0);
}
```

**Benefits:**
- Hides memory latency during HNSW traversal
- ~10-20% improvement on large datasets
- Critical for cold cache scenarios

### 4. Contiguous Memory Layout

```rust
pub struct ContiguousVectors {
    data: *mut f32,  // Single contiguous buffer
    dimension: usize,
    count: usize,
}
```

**Why it matters:**
- Cache line alignment (64 bytes)
- Sequential access pattern
- Enables hardware prefetching

## AVX-512 Transition Cost (Intel Skylake+)

On Intel Skylake-X and later CPUs, AVX-512 instructions incur a significant **warmup cost**:

| Phase | Cycles | Time @ 4GHz |
|-------|--------|-------------|
| License transition | ~20,000 | ~5μs |
| Register file power-up | ~36,000 | ~9μs |
| **Total warmup** | **~56,000** | **~14μs** |

### Why This Matters

1. **First AVX-512 instruction** triggers CPU frequency throttling (P-state transition)
2. **Subsequent instructions** run at reduced frequency until warmup completes
3. **Short bursts** of AVX-512 may be slower than AVX2 due to transition overhead

### VelesDB Mitigation

Dispatch is **static and tiered**, not benchmark-adaptive: the ISA level is
detected once (`OnceLock`), and the per-dimension tiers in the tables above
were fixed from offline measurements, so no runtime benchmarking happens on
the query path. Feature detection itself is lazy — the first distance call
pays it, every later call reads the cached level. One deliberate consequence
recorded in the dispatcher: in-place scaling has **no** AVX-512 kernel,
because a short normalize burst is exactly the shape the transition cost
punishes.

### Recommendations

| Workload | Recommendation |
|----------|----------------|
| **Sustained vector ops** (batch search) | AVX-512 beneficial |
| **Sporadic single queries** | AVX2 may be faster |
| **Mixed workloads** | The static tiers already encode this trade-off |

`velesdb simd info` prints a static summary of the dispatch design; it does
not currently report the level detected on the running machine (see the
Dispatch Registry's known gaps).

## Best Practices

### 1. Pre-normalize at Insertion

```rust
// Normalize once at insertion
let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
let normalized: Vec<f32> = vector.iter().map(|x| x / norm).collect();

// Fast cosine at search time
let similarity = cosine_normalized_native(&stored, &query);
```

### 2. Batch Operations

```rust
// Single query, multiple candidates (prefetching loop over the pair kernel)
let results = batch_cosine_native(&candidates, &query);
```

### 3. Use Appropriate Metric

| Use Case | Recommended Metric |
|----------|-------------------|
| Semantic search | Cosine (normalized) |
| Image embeddings | Euclidean |
| Recommendations | Dot Product |
| Binary features | Hamming |
| Set similarity | Jaccard |

## Running Benchmarks

```bash
# All SIMD benchmarks
cargo bench --bench simd_benchmark

# Specific dimension
cargo bench --bench simd_benchmark -- "768"

# Compare implementations
cargo bench --bench simd_benchmark -- "explicit_simd|auto_vec"
```

## Native SIMD API

```rust
use velesdb_core::simd_native;

// Direct native SIMD calls (no dispatch overhead)
let sim = simd_native::cosine_similarity_native(&a, &b);
let dist = simd_native::euclidean_native(&a, &b);
let dot = simd_native::dot_product_native(&a, &b);
let n = simd_native::norm_native(&v);
simd_native::normalize_inplace_native(&mut v);

// Batch operations with prefetching
let results = simd_native::batch_dot_product_native(&candidates, &query);
```

### Module Structure

| Module | Purpose | Use When |
|--------|---------|----------|
| `simd_native` | Hand-tuned intrinsics (AVX2/AVX-512/NEON) + tiered dispatch | The engine's own path — everything routes here |
| `simd_dispatch` | Thin public facade re-exporting `simd_native` entry points | External callers wanting a stable surface |

## Future Optimizations

1. **ARM SVE** - Scalable vectors for ARM servers
2. **WASM SIMD relaxed** - Additional browser performance
3. **Native f16/bf16 arithmetic** (F16C / AVX-512 FP16 / NEON FP16) - today mixed precision converts to f32 in scalar
4. **int8 VNNI / `sdot`** - dedicated int8 distance instructions (VNNI is already detected, unused)

GPU offload shipped behind the `gpu` feature — see the Dispatch Registry
above for what is wired and its thresholds.

## License

VelesDB Core is licensed under VelesDB Core License 1.0.

---

Last updated: 2026-08-18 · Applies to: velesdb-core 5.2.0
