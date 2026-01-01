# SIMD Performance Guide

VelesDB uses explicit SIMD optimizations for ultra-fast vector operations.

## Architecture Support

| Platform | Instructions | Performance (768D) |
|----------|-------------|-------------------|
| **x86_64** | AVX2 (256-bit) | ~45-80ns |
| **x86_64** | AVX-512 (512-bit) | ~30-50ns (if available) |
| **aarch64** | NEON (128-bit) | ~60-100ns |
| **WASM** | SIMD128 | ~80-120ns |

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

## Future Optimizations

1. **AVX-512 native** - 80% gain on supported CPUs (Zen 4+, Skylake-X+)
2. **ARM SVE** - Scalable vectors for ARM servers
3. **WASM SIMD relaxed** - Additional browser performance
4. **GPU offload** - Optional CUDA/Metal for batch operations

## License

VelesDB Core is licensed under Elastic License 2.0 (ELv2).
