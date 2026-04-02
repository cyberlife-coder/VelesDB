# Native HNSW Implementation

`VelesDB` includes a **custom native HNSW implementation** — VelesDB's single native HNSW implementation (no pluggable backends) since v1.0.

> **🎉 v1.0**: `hnsw_rs` dependency **completely removed**. Native HNSW is now the only implementation.

## Performance

*Benchmarked March 20, 2026 — Intel Core i9-14900KF, 64GB DDR5, Windows 11, Rust 1.92.0*

| Operation | Native HNSW | External libs | Improvement |
|-----------|-------------|---------------|-------------|
| **Search (100 queries)** | 26.9 ms | ~32 ms | **1.2x faster** ✅ |
| **Parallel Insert (5k)** | 1.47 s | ~1.6 s | **1.07x faster** ✅ |
| **Recall** | ~99% | baseline | Parity ✓ |

> **Key insight**: Native HNSW excels at **search operations** — the most critical path for production workloads.

## Usage (v1.0+)

No feature flags needed. Native HNSW is the only implementation:

```toml
[dependencies]
velesdb-core = "1.0"
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
│  vectors: RwLock<ContiguousVectors>  (64-byte aligned storage)  │
│  layers: RwLock<Vec<Layer>>  (hierarchical graph)               │
│  entry_point: AtomicUsize    (lock-free CAS promotion)          │
│  max_layer: AtomicUsize      (lock-free CAS promotion)          │
└─────────────────────────────────────────────────────────────────┘
```

## Available Methods

### Construction

| Method | Params | Recall | Speed | Description |
|--------|--------|--------|-------|-------------|
| `new(dim, metric)` | M=32, ef=400 | ≥95% | Baseline | Production workloads |
| `with_params(dim, metric, params)` | Custom | Custom | Custom | Full control |
| `new_turbo(dim, metric)` | M=12, ef=100 | ~85% | 3-5x faster | Bulk import, dev, benchmarks |
| `new_fast_insert(dim, metric)` | M/2, ef/2 | ~90% | 2-3x faster | Streaming, no vector storage |

### Operations

| Method | Description |
|--------|-------------|
| `insert(id, vector)` | Insert single vector |
| `insert_batch(&[(id, vec)])` | Batch insert |
| `insert_batch_parallel(items)` | Parallel batch insert |
| `search(query, k)` | Standard search (Balanced mode) |
| `search_with_quality(query, k, quality)` | Search with quality preset (Fast/Balanced/Accurate/Perfect/Adaptive/AutoTune) |
| `search_with_ef(query, k, ef_search)` | Search with explicit ef_search value |
| `search_batch_parallel(queries, k, ef_search)` | Batch parallel search |
| `brute_force_search_parallel(query, k)` | Exact search (100% recall) |
| `remove(id)` | Remove vector |

### Persistence

| Method | Description |
|--------|-------------|
| `save(path)` | Save index to disk |
| `load(path, dim, metric)` | Load index from disk |

## Batch Insert Internals

### Two-Phase Allocation

`allocate_batch()` uses two separate lock scopes to minimize write-lock
contention on vector storage:

1. **`reserve_vector_capacity()`** (cold path): Acquires a write lock,
   initializes storage if needed, and pre-reserves capacity for the entire
   batch. This may trigger a buffer resize (reallocation), but it happens
   at most once per batch.
2. **`bulk_push_vectors()`** (hot path): Acquires a write lock and performs
   a bulk `push_batch()` into the pre-reserved space. No reallocation
   occurs, so the lock is held only for fast memcpy operations.

### Graduated ef_construction

For batches >= 1000 vectors, `BatchEfSchedule` applies a 3-phase VAMANA/
DiskANN-inspired schedule that reduces total construction work while
preserving graph quality:

- **Scaffold** (first 10%): Full `ef_construction` -- builds a high-quality
  backbone that guides subsequent insertions.
- **Bulk** (middle 80%): 0.5x `ef_construction` -- leverages the existing
  scaffold for efficient navigation with reduced candidate evaluation.
- **Finalize** (last 10%): 0.75x `ef_construction` -- restores edge quality
  at the graph periphery.

All reduced ef values are floored at `2 * M` to ensure the candidate pool
is never smaller than the number of neighbors to select.

### Lock-Free Entry-Point Promotion

Entry-point updates use atomic CAS (`compare_exchange`) instead of a mutex.
Two CAS operations handle the two cases:

1. **Empty index**: CAS on `entry_point` from `NO_ENTRY_POINT` to the
   inserting node's ID.
2. **Layer promotion**: CAS on `max_layer` to claim the new maximum,
   then store the new `entry_point`.

This eliminates a serialization point during concurrent batch insert.
Entry-point promotion is rare (O(log_M(N)) times per index lifetime).

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

## RaBitQ Backend

VelesDB supports an optional **RaBitQ backend** that uses binary graph traversal for 32x memory bandwidth reduction during search, with exact float32 re-ranking for final results.

### `HnswBackend` Enum

`NativeHnswInner` selects the backend at construction time via the `HnswBackend` enum:

```rust
enum HnswBackend {
    /// Standard f32 distance backend (NativeHnsw<CachedSimdDistance>).
    Standard(NativeHnsw<CachedSimdDistance>),
    /// RaBitQ binary traversal + f32 re-ranking backend (boxed to avoid
    /// inflating the Standard variant's cache-line footprint).
    RaBitQ(Box<RaBitQPrecisionHnsw<CachedSimdDistance>>),
}
```

- **`Standard`**: Full f32 distances for both traversal and results. Default for `StorageMode::Full`.
- **`RaBitQ`**: Binary distances (XOR + popcount) for graph traversal, f32 re-ranking for final results. Activated by `StorageMode::RaBitQ`.

### Enabling RaBitQ

Set `StorageMode::RaBitQ` when creating a collection:

```rust
use velesdb_core::{Database, StorageMode};

let db = Database::open("./data")?;
db.create_collection_with_storage("documents", 768, "cosine", StorageMode::RaBitQ)?;
```

**CLI**:

```bash
velesdb-cli create --name documents --dim 768 --metric cosine --storage rabitq
```

**REST API**:

```json
POST /collections
{
  "name": "documents",
  "dimension": 768,
  "metric": "cosine",
  "storage_mode": "rabitq"
}
```

### Architecture

```
┌──────────────────────────────────────────────────────────────┐
│               RaBitQPrecisionHnsw<D>                         │
├──────────────────────────────────────────────────────────────┤
│  inner: NativeHnsw<D>          (graph structure + float32)   │
│  rabitq_index: RaBitQIndex     (rotation matrix + centroid)  │
│  rabitq_store: RaBitQVectorStore  (bits + corrections)       │
└──────────────────────────────────────────────────────────────┘
```

`RaBitQPrecisionHnsw<D>` wraps `NativeHnsw<D>` and adds a RaBitQ quantizer and binary vector store. The inner graph remains a standard HNSW graph — only the distance function changes during traversal.

### Search Flow

1. **Query preparation**: Rotate the query vector using the learned orthogonal rotation matrix. Cost: ~60 us for 768D (amortized over hundreds of distance evaluations per search).
2. **Binary traversal**: Traverse the HNSW graph using XOR + popcount binary distances with affine correction factors. Oversampling ratio of 6x compensates for coarser binary fidelity (vs 4x for SQ8). Cost: ~2 ns per candidate.
3. **Float32 re-ranking**: Collect `k * 6` coarse candidates, then compute exact f32 distances from the inner `NativeHnsw` vector store. Return the top-k with exact distances.

If the quantizer is not yet trained, search falls back transparently to standard f32 distances.

### Training

Training is **lazy**: vectors are buffered until `training_sample_size` (1000) are accumulated, then the quantizer trains automatically on the next insert. Until trained, all operations use standard f32 distances.

```rust
// Quantizer trains automatically after 1000 inserts
for (id, vec) in vectors {
    collection.upsert(id, &vec, None)?;
}

// Or force training early with fewer vectors
rabitq_hnsw.force_train_quantizer()?;
```

### Interior Mutability

`RaBitQPrecisionHnsw` uses interior mutability for thread-safe concurrent access:

| Field | Type | Purpose |
|-------|------|---------|
| `rabitq_index` | `RwLock<Option<Arc<RaBitQIndex>>>` | Trained quantizer (write-locked once during training, then read-only) |
| `rabitq_store` | `RwLock<Option<RaBitQVectorStore>>` | Binary-encoded vector storage |
| `training_buffer` | `Mutex<Vec<Vec<f32>>>` | Pre-training vector accumulator |

**Ordering invariant**: The store must be visible before the index. Search checks `rabitq_index` first — a `Some(index)` with `None` store would silently skip RaBitQ encoding.

### Performance

| Metric | Standard (f32) | RaBitQ | Ratio |
|--------|---------------|--------|-------|
| Memory bandwidth per candidate | 1x | 1/32x | **32x reduction** |
| Distance computation | ~10 ns (f32 SIMD) | ~2 ns (XOR + popcount) | **5x faster** |
| Query preparation | 0 | ~60 us (768D) | One-time per query |
| Minimum index size | N/A | 5000 vectors | Below threshold: f32 fallback |

## PDX Block-Columnar Layout

VelesDB includes a **PDX block-columnar vector layout** that transposes row-major vectors into 64-vector blocks for SIMD-parallel distance computation.

### Memory Layout

Standard Array-of-Structures (AoS) stores vectors contiguously:

```
[v0_d0, v0_d1, ..., v0_dD, v1_d0, v1_d1, ..., v1_dD, ...]
```

PDX block-columnar layout groups 64 vectors into blocks, with dimensions interleaved within each block:

```
Block k: [v_{kB}_d0,   ..., v_{kB+63}_d0,    // dim 0
          v_{kB}_d1,   ..., v_{kB+63}_d1,    // dim 1
          ...
          v_{kB}_dD-1, ..., v_{kB+63}_dD-1]  // dim D-1
```

This enables broadcasting `query[d]` once per dimension and computing the d-th contribution for all 64 vectors simultaneously, achieving 64x better register reuse vs AoS.

### `ColumnarVectors`

The `ColumnarVectors` struct transposes `ContiguousVectors` (AoS) into PDX layout:

```rust
// Auto-built after BFS reordering — not created manually
let pdx = ColumnarVectors::from_contiguous(&contiguous_vectors);

// Access a block of 64 vectors
let block_data: &[f32] = pdx.block_ptr(block_idx);
let valid_count: usize = pdx.block_size(block_idx);
```

The last block is zero-padded if the vector count is not a multiple of 64. Zero-padding is safe: squared-L2 and dot-product contributions from padded slots are zeroed out by the block distance kernels.

### Auto-Build After BFS Reordering

PDX layout is built automatically after `reorder_for_locality()` completes BFS graph reordering:

```rust
// After bulk insert, reorder for cache locality
hnsw.reorder_for_locality()?;
// PDX columnar layout is now available in hnsw.columnar
```

The columnar data is stored in `NativeHnsw::columnar: RwLock<Option<ColumnarVectors>>` with **lock rank 15** (between vectors=10 and layers=20), ensuring deadlock-free acquisition:

```
vectors (rank 10) -> columnar (rank 15) -> layers (rank 20) -> neighbors (rank 30)
```

### Block Distance Kernels

Three kernels compute distances from a query to all 64 vectors in a block simultaneously:

| Kernel | Metric | Returns |
|--------|--------|---------|
| `block_squared_l2(query, block, dim, block_size)` | Euclidean | Squared L2 distances |
| `block_dot_product(query, block, dim, block_size)` | DotProduct | Negative dot products |
| `block_cosine_distance(query, block, dim, block_size)` | Cosine | Cosine distances |

Each kernel returns `[f32; 64]` — only indices `0..block_size` contain valid results.

LLVM auto-vectorizes the inner loop over 64 elements into:
- **4 iterations** with AVX-512 (16 f32 lanes)
- **8 iterations** with AVX2 (8 f32 lanes)
- **16 iterations** with NEON (4 f32 lanes)

No manual SIMD intrinsics are needed.

### Reference

Pirk, H. et al. "Efficient Cross-Columnar Sorting" (PDX layout).

## Software Pipelining

VelesDB implements **software-pipelined HNSW search** that overlaps prefetch of the next candidate's neighbor vectors with distance computation of the current batch, hiding main-memory latency behind useful ALU work.

### Activation Conditions

The pipelined path activates when **both** conditions are met:

1. **`should_prefetch()` returns `true`**: the vector spans at least 2 cache lines (dimension >= 32 for 4-byte f32, i.e., >= 128 bytes).
2. **`vectors.len() >= 10_000`**: the dataset exceeds ~30 MB at 768-dim (3 KB/vec), ensuring data is not fully L3-resident. Below this threshold, vectors are likely cache-hot and prefetch overhead exceeds the benefit.

```rust
// Activation logic in search_layer:
let use_prefetch = should_prefetch(vectors.dimension());
let use_pipeline = use_prefetch && vectors.len() >= 10_000;
```

### Pipeline Strategy

The pipeline uses **peek-based speculative prefetch** (not pop-ahead), which preserves identical heap exploration order and recall:

```
1. Pop current candidate from min-heap
2. Gather current candidate's unvisited neighbors
3. Peek (without popping) at the NEXT candidate in the min-heap
4. Prefetch next candidate's neighbor vectors into CPU cache
5. Compute distances for current batch (DRAM latency hidden by step 4)
6. Process results into search state
7. Repeat
```

Because the next candidate is only peeked — never consumed before the current batch is fully processed — the heap exploration order is identical to the non-pipelined loop.

### Correctness Guarantee

The pipelined path produces **identical results** to the non-pipelined path. Only memory access order differs. If the current batch adds a closer candidate that displaces the peeked one, the speculative prefetch is wasted but harmless (only occupies a few cache lines).

### Key Source Files

| File | Purpose |
|------|---------|
| `native/graph/search_pipeline.rs` | Pipelined search loop implementation |
| `native/graph/search.rs` | `should_prefetch()` threshold, activation logic |

## AutoTune Search

`SearchQuality::AutoTune` computes optimal `ef_search` range from collection statistics, then delegates to the adaptive two-phase search algorithm. This is the recommended quality setting for applications that want good recall without manual ef tuning.

### How It Works

1. **`auto_ef_range(count, dimension, k)`** computes `(min_ef, max_ef)`:
   - **Base ef** scales in discrete tiers by collection size:
     - 0--1K vectors: `k * 2`
     - 1K--10K vectors: `k * 4`
     - 10K--100K vectors: `k * 8`
     - 100K+ vectors: `k * 12`
   - **Dimension factor**: high-dimensional spaces (>512) apply a 1.5x multiplier for sparser neighborhoods.
   - **`min_ef`** is clamped to at least `k` (never fewer candidates than requested results).
   - **`max_ef`** is set to `4 * min_ef`, giving the adaptive second phase ample headroom for hard queries.

2. **Adaptive two-phase search**: starts with `min_ef`, escalates to `max_ef` if the query is hard (same algorithm as `SearchQuality::Adaptive`).

### Usage

**Rust**:

```rust
use velesdb_core::SearchQuality;

let results = index.search_with_quality(&query, 10, SearchQuality::AutoTune);
```

**Python**:

```python
results = collection.search_with_quality(
    vector=query,
    quality="autotune",
    top_k=10,
)
```

**REST API**:

```json
POST /collections/documents/search
{
  "vector": [0.1, 0.2, ...],
  "top_k": 10,
  "mode": "autotune"
}
```

### When to Use AutoTune

| Scenario | Recommended Quality |
|----------|-------------------|
| Fixed workload, known recall target | `Balanced` or `Accurate` with explicit `ef_search` |
| Variable collection sizes, no tuning budget | **`AutoTune`** |
| Latency-critical, recall > 90% acceptable | `Fast` |
| Must guarantee 100% recall | `Perfect` |

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

> **ANN State of the Art:** [ANN_SOTA_AUDIT.md](../ANN_SOTA_AUDIT.md)
