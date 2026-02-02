# SIMD Performance Guide

VelesDB uses **native SIMD dispatch** for ultra-fast vector operations, automatically selecting the optimal implementation based on CPU features and vector size.

## Native SIMD Architecture (EPIC-052/077)

The `simd_native` module provides hand-tuned SIMD implementations using `core::arch` intrinsics:

```
┌─────────────────────────────────────────────────────────────────┐
│              simd_native::cosine_similarity_native()             │
│                                                                  │
│  Runtime: feature detection → tiered dispatch → native SIMD     │
│  - AVX-512: 4/2/1 accumulators based on size                    │
│  - AVX2: 4-acc (>1024), 2-acc (64-1023), 1-acc (<64)            │
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
| **x86_64 AVX-512** | simd_native | 512-bit 4-acc | ~38-42ns |
| **x86_64 AVX2** | simd_native | 256-bit 2/4-acc | ~40-82ns |
| **aarch64** | simd_native | NEON 128-bit | ~60-100ns |
| **WASM** | wide_simd | SIMD128 | ~80-120ns |
| **Fallback** | Scalar | Native Rust | ~150-200ns |

### Tiered Dispatch Strategy (EPIC-077)

AVX2 implementations adapt based on vector size to minimize register pressure:

| Size Range | Accumulators | Use Case |
|------------|--------------|----------|
| >= 1024 elements | 4-acc | Large vectors (text-embedding-3-large) |
| 64-1023 elements | 2-acc | Medium vectors (BERT, ada-002) |
| 8-63 elements | 4-acc | Small vectors (legacy) |
| < 8 elements | Scalar | Tiny vectors (avoid SIMD overhead) |

## Performance Benchmarks (February 2026)

### Distance Functions (768D vectors)

| Function | Latency | Throughput | vs Previous |
|----------|---------|------------|-------------|
| `dot_product_native` | **37.7ns** | 26.5M ops/s | Baseline |
| `euclidean_native` | **20.9ns** | 47.8M ops/s | Improved |
| `cosine_similarity_native` | **40.9ns** | 24.4M ops/s | Optimized (2-acc) |
| `cosine_normalized_native` | **37.7ns** | 26.5M ops/s | Same as dot |
| `hamming_distance_native` | **18.7ns** | 53.5M ops/s | 2x faster |
| `jaccard_similarity_native` | **22.8ns** | 43.9M ops/s | 20% faster |

### Scaling by Dimension (simd_native)

| Dimension | Cosine | Dot Product | Model |
|-----------|--------|-------------|-------|
| 128 | 10.0ns | 6.3ns | MiniLM |
| 384 | 20.9ns | 11.4ns | all-MiniLM-L6-v2 |
| 768 | 40.9ns | 37.7ns | BERT, ada-002 |
| 1536 | 66.4ns | 32.5ns | text-embedding-3-small |
| 3072 | 138.7ns | 81.2ns | text-embedding-3-large |

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
// Standard cosine: 3 passes (dot, norm_a, norm_b)
pub fn cosine_similarity_fast(a: &[f32], b: &[f32]) -> f32;

// Normalized: 1 pass (dot only) - 40% faster!
pub fn cosine_similarity_normalized(a: &[f32], b: &[f32]) -> f32;
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

The adaptive dispatch system handles this automatically:

```rust
// 500 iterations per benchmark captures warmup cost
const BENCHMARK_ITERATIONS: usize = 500;

// Eager initialization at Database::open() avoids first-call latency
let info = simd_ops::init_dispatch();
```

**Result**: The dispatch table reflects real-world performance *after* warmup, ensuring AVX-512 is only selected when it provides a genuine advantage over AVX2.

### Recommendations

| Workload | Recommendation |
|----------|----------------|
| **Sustained vector ops** (batch search) | AVX-512 beneficial |
| **Sporadic single queries** | AVX2 may be faster |
| **Mixed workloads** | Let adaptive dispatch decide |

To check which backend was selected:
```bash
velesdb simd info
```

## Best Practices

### 1. Pre-normalize at Insertion

```rust
// Normalize once at insertion
let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
let normalized: Vec<f32> = vector.iter().map(|x| x / norm).collect();

// Fast cosine at search time
let similarity = cosine_similarity_normalized(&stored, &query);
```

### 2. Batch Operations

```rust
// Single query, multiple candidates
let results = batch_cosine_normalized(&candidates, &query);
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

// Fast approximate (Newton-Raphson rsqrt)
let fast_sim = simd_native::cosine_similarity_fast(&a, &b);
```

### Module Structure

| Module | Purpose | Use When |
|--------|---------|----------|
| `simd_native` | Hand-tuned intrinsics (AVX2/AVX-512/NEON) | Maximum performance, native CPU |
| `wide_simd` | Portable SIMD (f32x8) | WASM, cross-platform |
| `simd` | Auto-vectorized fallback | Generic builds |

## Future Optimizations

1. **ARM SVE** - Scalable vectors for ARM servers
2. **WASM SIMD relaxed** - Additional browser performance
3. **GPU offload** - Optional CUDA/Metal for batch operations

## License

VelesDB Core is licensed under Elastic License 2.0 (ELv2).
