# Native HNSW Implementation

VelesDB includes a **native HNSW implementation** that provides significant performance improvements over the external `hnsw_rs` library.

> **🎉 v0.8.12+**: Native HNSW is now the **DEFAULT** — no feature flag needed!

## Performance Comparison

*Benchmarked January 8, 2026 — Intel Core i9-14900KF, 64GB DDR5, Windows 11, Rust 1.92.0*

| Operation | Native HNSW | hnsw_rs | Improvement |
|-----------|-------------|---------|-------------|
| **Search (100 queries)** | 26.9 ms | 32.4 ms | **1.2x faster** ✅ |
| **Parallel Insert (5k)** | 1.47 s | 1.57 s | **1.07x faster** ✅ |
| **Recall** | ~99% | baseline | Parity ✓ |

> **Key insight**: Native HNSW excels at **search operations** — the most critical path for production workloads. The 20% improvement on search latency directly benefits RAG pipelines and real-time applications.

## Feature Flags (v0.8.12+)

### Default: Native HNSW

Native HNSW is enabled by default. No configuration needed:

```toml
[dependencies]
velesdb-core = "0.8.12"  # Native HNSW by default
```

### Legacy: hnsw_rs (for compatibility)

If you need to fall back to `hnsw_rs` for compatibility:

```toml
[dependencies]
velesdb-core = { version = "0.8.12", default-features = false, features = ["legacy-hnsw"] }
```

## API

When enabled, `NativeHnswIndex` is exported alongside the standard `HnswIndex`:

```rust
use velesdb_core::index::hnsw::NativeHnswIndex;
use velesdb_core::DistanceMetric;

// Create index
let index = NativeHnswIndex::new(768, DistanceMetric::Cosine);

// Insert vectors
index.insert(1, &vec![0.1; 768]);
index.insert_batch(&[(2, vec![0.2; 768]), (3, vec![0.3; 768])]);

// Search
let results = index.search(&query, 10);

// Persistence
index.save("./my_index")?;
let loaded = NativeHnswIndex::load("./my_index", 768, DistanceMetric::Cosine)?;
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     NativeHnswIndex                             │
├─────────────────────────────────────────────────────────────────┤
│  inner: NativeHnswInner      (HNSW graph + SIMD distances)      │
│  mappings: ShardedMappings   (lock-free ID <-> index mapping)   │
│  vectors: ShardedVectors     (parallel vector storage)          │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     NativeHnsw<D>                               │
├─────────────────────────────────────────────────────────────────┤
│  distance: SimdDistance      (AVX2/SSE/NEON optimized)          │
│  vectors: RwLock<Vec<f32>>   (stored vectors)                   │
│  layers: RwLock<Vec<Layer>>  (hierarchical graph)               │
└─────────────────────────────────────────────────────────────────┘
```

## Available Methods

### Construction

| Method | Description |
|--------|-------------|
| `new(dim, metric)` | Create with auto-tuned params |
| `with_params(dim, metric, params)` | Create with custom params |
| `new_turbo(dim, metric)` | Optimized for speed |
| `new_fast_insert(dim, metric)` | Optimized for bulk loading |

### Operations

| Method | Description |
|--------|-------------|
| `insert(id, vector)` | Insert single vector |
| `insert_batch(&[(id, vec)])` | Batch insert |
| `insert_batch_parallel(items)` | Parallel batch insert |
| `search(query, k)` | Standard search |
| `search_with_quality(query, k, quality)` | Search with quality profile |
| `search_batch_parallel(queries, k, quality)` | Batch parallel search |
| `brute_force_search_parallel(query, k)` | Exact search (100% recall) |
| `remove(id)` | Remove vector |

### Persistence

| Method | Description |
|--------|-------------|
| `save(path)` | Save index to disk |
| `load(path, dim, metric)` | Load index from disk |

## Dual-Precision Search

For even higher performance, VelesDB includes a **dual-precision HNSW** implementation:

```rust
use velesdb_core::index::hnsw::native::DualPrecisionHnsw;

let mut hnsw = DualPrecisionHnsw::new(distance, 768, 32, 200, 100000);

// Insert vectors (quantizer trains automatically after 1000 vectors)
for (id, vec) in vectors {
    hnsw.insert(vec);
}

// Search with dual-precision (graph traversal + exact rerank)
let results = hnsw.search(&query, 10, 128);
```

### How It Works

1. **Graph Traversal**: Uses SIMD-accelerated float32 distances
2. **Re-ranking**: Computes exact float32 distances for final results
3. **Result**: Fast exploration + accurate final ranking

## Migration Guide

### From `HnswIndex` to `NativeHnswIndex`

The API is largely compatible. Key differences:

1. **Feature flag required**: Add `features = ["native-hnsw"]`
2. **Load signature**: `load(path, dim, metric)` vs `load(path)`
3. **No `set_searching_mode`**: Native doesn't need this (no-op provided)

### Gradual Migration

```rust
// Conditional compilation
#[cfg(feature = "native-hnsw")]
use velesdb_core::index::hnsw::NativeHnswIndex as HnswIndex;

#[cfg(not(feature = "native-hnsw"))]
use velesdb_core::index::hnsw::HnswIndex;
```

## Benchmarks

Run the comparison benchmark:

```bash
cargo bench --bench hnsw_comparison_benchmark
```

## Removing hnsw_rs Dependency

The Native HNSW implementation is now **production-ready** and can fully replace `hnsw_rs`:

### Current Status

| Capability | Native HNSW | hnsw_rs | Status |
|------------|-------------|---------|--------|
| Insert | ✅ | ✅ | Parity |
| Batch Insert | ✅ Parallel | ✅ Sequential | Native faster |
| Search | ✅ 1.2x faster | ✅ | Native faster |
| Recall | ~99% | baseline | Parity |
| Persistence | ✅ | ✅ | Parity |
| Brute-force | ✅ | ✅ | Parity |

### Migration Path

1. **Test with feature flag**: `cargo test --features native-hnsw`
2. **Benchmark your workload**: `cargo bench --bench hnsw_comparison_benchmark`
3. **Full migration**: Make `native-hnsw` the default in a future release

### Files to Update for Full Migration

- `Cargo.toml`: Make `hnsw_rs` optional
- `src/index/hnsw/index.rs`: Use `NativeHnswInner` by default
- `src/index/hnsw/mod.rs`: Export `NativeHnswIndex` as `HnswIndex`

## Future Optimizations

- **int8 graph traversal**: Use quantized vectors for graph exploration
- **PCA dimension reduction**: Reduce dimensions during traversal
- **GPU acceleration**: CUDA/Vulkan compute shaders for batch operations
