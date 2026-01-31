# SIMD Performance Guide

VelesDB uses **adaptive SIMD dispatch** for ultra-fast vector operations, automatically selecting the optimal backend based on runtime micro-benchmarks.

## Adaptive Dispatch Architecture

The `simd_ops` module automatically selects the fastest SIMD backend for each (metric, dimension) combination on the current machine:

```
┌─────────────────────────────────────────────────────────────────┐
│                    simd_ops::similarity()                        │
│                                                                  │
│  First call: micro-benchmarks (~5-10ms) → builds DispatchTable  │
│  Subsequent calls: O(1) lookup → direct backend call            │
└─────────────────────────────────────────────────────────────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
  ┌───────────┐        ┌───────────┐        ┌───────────┐
  │ NativeAVX │        │  Wide32   │        │  Wide8    │
  │ (512/256) │        │ (4×f32x8) │        │ (f32x8)   │
  └───────────┘        └───────────┘        └───────────┘
```

## Architecture Support

| Platform | Backend | Instructions | Performance (768D) |
|----------|---------|-------------|-------------------|
| **x86_64 AVX-512** | NativeAvx512 | 512-bit | ~30-50ns |
| **x86_64 AVX2** | NativeAvx2 | 256-bit | ~45-80ns |
| **aarch64** | NativeNeon | NEON 128-bit | ~60-100ns |
| **WASM** | Wide8 | SIMD128 | ~80-120ns |
| **Fallback** | Scalar | Native Rust | ~150-200ns |

## Performance Benchmarks

### Distance Functions (768D vectors)

| Function | Latency | Throughput |
|----------|---------|------------|
| `dot_product_fast` | **45ns** | 22M ops/s |
| `euclidean_distance_fast` | **47ns** | 21M ops/s |
| `cosine_similarity_fast` | **79ns** | 12M ops/s |
| `cosine_similarity_normalized` | **45ns** | 22M ops/s |

### Scaling by Dimension

| Dimension | Cosine | Dot Product | Model |
|-----------|--------|-------------|-------|
| 128 | 15ns | 8ns | MiniLM |
| 384 | 35ns | 20ns | all-MiniLM-L6-v2 |
| 768 | 79ns | 45ns | BERT, ada-002 |
| 1536 | 152ns | 90ns | text-embedding-3-small |
| 3072 | 304ns | 170ns | text-embedding-3-large |

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

## Adaptive Dispatch API

```rust
use velesdb_core::simd_ops;
use velesdb_core::DistanceMetric;

// Automatic dispatch to fastest backend
let sim = simd_ops::similarity(DistanceMetric::Cosine, &a, &b);
let dist = simd_ops::distance(DistanceMetric::Euclidean, &a, &b);
let n = simd_ops::norm(&v);
simd_ops::normalize_inplace(&mut v);

// Introspection
let info = simd_ops::dispatch_info();
println!("Init time: {}ms", info.init_time_ms);
println!("Cosine backends: {:?}", info.cosine_backends);
```

## Future Optimizations

1. **ARM SVE** - Scalable vectors for ARM servers
2. **WASM SIMD relaxed** - Additional browser performance
3. **GPU offload** - Optional CUDA/Metal for batch operations

## License

VelesDB Core is licensed under Elastic License 2.0 (ELv2).
