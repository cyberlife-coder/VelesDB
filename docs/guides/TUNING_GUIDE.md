# VelesDB Tuning Guide

A practical reference for configuring VelesDB to balance recall, latency, memory, and
durability for your workload.

---

## Quantization Modes

VelesDB supports five storage modes via the `StorageMode` enum
(`velesdb_core::quantization::StorageMode`). Each trades memory for recall accuracy.

### When to Use Each Mode

| Mode | Aliases | Compression | Recall Impact | Training Required | Best For |
|------|---------|-------------|---------------|-------------------|----------|
| `Full` (default) | `f32` | 1x (baseline) | Perfect | No | Small datasets (<100K), high-precision needs |
| `SQ8` | `int8` | 4x | ~1-2% recall loss | No | Medium datasets (100K-10M), general purpose |
| `Binary` | `bit` | 32x | ~10-15% recall loss | No | Edge/IoT, fingerprints, memory-constrained |
| `ProductQuantization` | | 8-32x | ~5-15% recall loss | Yes | Large datasets, aggressive compression |
| `RaBitQ` | | 32x | ~5-10% recall loss | Yes (rotation matrix) | High compression with better recall than Binary |

### SQ8 (Scalar Quantization 8-bit)

Quantizes each `f32` (4 bytes) to a `u8` (1 byte) using min/max scaling per vector.

- 4x memory reduction with minimal recall loss
- No training step required -- quantization is computed per-vector
- Recommended as the default for production workloads above 100K vectors

```
Before:  [0.123, 0.456, 0.789, ...]  -> 768 x 4 = 3072 bytes
After:   [31, 116, 201, ...]          -> 768 x 1 = 768 bytes + metadata
```

### Binary Quantization

Quantizes each `f32` to a single bit (positive = 1, negative = 0).

- 32x memory reduction
- Best with high-dimensional embeddings (>=768 dimensions)
- Distance computed via Hamming distance (extremely fast with POPCNT)
- Accuracy degrades significantly for low-dimensional spaces

### Product Quantization (PQ)

Splits the vector into subspaces and quantizes each subspace independently using
learned codebooks.

- Configurable compression ratio via subspace count
- Requires a training step on representative data (>=256 vectors recommended)
- Vector dimension must be evenly divisible by the number of subspaces
- Error `VELES-028` is raised for invalid PQ configurations

### RaBitQ (Randomized Binary Quantization)

Binary quantization with a learned rotation matrix and scalar correction factors.

- 32x compression like Binary, but with significantly better recall
- Requires a training step to compute the rotation matrix
- Best choice when memory is critical but recall matters more than Binary can provide

---

## HNSW Index Parameters

The `HnswParams` struct (`velesdb_core::index::hnsw::HnswParams`) controls the HNSW
graph index. VelesDB auto-tunes these based on vector dimension, but manual tuning
can improve results for specific workloads.

### Key Parameters

| Parameter | Field | Default | Effect |
|-----------|-------|---------|--------|
| M | `max_connections` | auto (24-32 based on dim) | Bi-directional links per node. Higher = better recall, more memory |
| ef_construction | `ef_construction` | auto (300-400 based on dim) | Build-time candidate list size. Higher = better graph quality, slower build |
| max_elements | `max_elements` | 100,000 | Initial capacity (grows automatically if exceeded) |
| storage_mode | `storage_mode` | `StorageMode::Full` | Vector compression mode |

### Auto-Tuned Defaults by Dimension

| Dimension | M (`max_connections`) | `ef_construction` |
|-----------|----------------------|-------------------|
| <= 256 | 24 | 300 |
| >= 257 | 32 | 400 |

### Dataset-Size-Aware Parameters

Use `HnswParams::for_dataset_size(dimension, count)` for optimal parameters at scale:

| Dataset Size | M (dim <= 256) | M (dim > 256) | ef_construction |
|-------------|----------------|---------------|-----------------|
| <= 10K | 24 | 32 | 200-400 |
| <= 100K | 64 | 128 | 800-1600 |
| <= 500K | 96 | 128 | 1200-2000 |
| <= 1M | 64 | 128 | 800-1600 |

### Convenience Constructors

```rust
use velesdb_core::index::hnsw::HnswParams;

// Auto-tune for dimension (default: M=32, ef=400 for 768D)
let params = HnswParams::auto(768);

// Scale-aware: optimized for 500K vectors at 768D
let params = HnswParams::for_dataset_size(768, 500_000);

// Million-scale: M=128, ef_construction=1600
let params = HnswParams::million_scale(768);

// Maximum recall: aggressive params for evaluation
let params = HnswParams::max_recall(768);

// Turbo: M=12, ef=100 — fastest build, ~85% recall
let params = HnswParams::turbo();

// Fast indexing: M/2, ef/2 of auto — balanced speed/recall, ~90% recall
let params = HnswParams::fast_indexing(768);

// Fully custom
let params = HnswParams::custom(48, 800, 200_000);
```

### Index Construction Modes

VelesDB offers three index constructors with different speed/recall tradeoffs:

| Constructor | HNSW Params | Recall | Insert Speed | Use Case |
|-------------|-------------|--------|-------------|----------|
| `HnswIndex::new(dim, metric)` | `auto()` (M=32, ef=400) | ≥95% | Baseline | Production workloads |
| `HnswIndex::new_fast_insert(dim, metric)` | `fast_indexing()` (M/2, ef/2) | ~90% | ~2-3x faster | High-velocity streaming, memory-constrained |
| `HnswIndex::new_turbo(dim, metric)` | `turbo()` (M=12, ef=100) | ~85% | ~3-5x faster | Bulk loading, development, benchmarks |

**Recommended pattern**: Use `new_turbo()` for initial bulk import, then rebuild with
`new()` or `with_params()` for production search quality.

### Tuning Guidelines

- **Low latency priority**: Lower M (8-16), use `HnswParams::turbo()` for import then rebuild
- **High recall priority**: Higher M (32-64), use `HnswParams::max_recall(dim)`
- **Memory constrained**: Lower M (4-8), combine with `StorageMode::SQ8` or `StorageMode::Binary`
- **Bulk import**: Use `HnswParams::turbo()` during import, rebuild with production params after

### Upsert Semantics (v1.7+)

Since v1.7, inserting a vector with an existing ID **replaces it in-place**. The HNSW graph is automatically updated: old edges are removed and new edges are created based on the new vector position.

```rust
// Insert vector with id=1
collection.insert(1, vec![1.0, 0.0, 0.0, 0.0], None)?;

// Replace vector with id=1 — no delete needed
collection.insert(1, vec![0.0, 1.0, 0.0, 0.0], None)?;
```

This applies to both single inserts and batch operations. No configuration change is needed — upsert semantics are always enabled.

**Performance note:** In-place upsert is faster than delete + reinsert because it reuses the node slot in the HNSW graph and avoids a full graph reconnection.

### Batch Insert Optimization (v1.7+)

Large batch inserts are automatically optimized with several techniques:

1. **Chunked Phase B** — Batches are split into optimal chunks (computed by `compute_chunk_size()`). Each chunk updates the global entry point, improving graph connectivity for subsequent chunks. This is particularly effective for batches > 1000 vectors.

2. **Alloc/Connect Separation** — Node allocation is separated from edge connection. All node slots are pre-allocated first, then edges are connected in parallel without lock contention on the allocator. This yields ~2x throughput improvement for large batches.

Both optimizations are automatic and require no configuration. Use `insert_batch_parallel()` for best performance on large datasets.

3. **Batch Upsert Fast-Path (v1.7.2)** — Pure-insert workloads (all new IDs) now skip the expensive `DashMap::entry()` write lock introduced by upsert semantics in v1.7.0. A read-lock `contains_key()` check routes new IDs to a cheaper allocation path. This eliminates the ~14% overhead observed on pure-insert workloads. Mixed workloads (some new, some existing IDs) automatically fall back to the full upsert path for correctness.

4. **Upsert Lock Contention Fix (v1.7.2)** — `Collection::upsert()` was previously bottlenecked by three sources of lock contention: (a) a write lock on the HNSW index for each insert (changed to a read lock since `NativeHnswInner::insert` uses internal per-node synchronization), (b) per-point `insert_or_defer()` calls replaced by a single `bulk_index_or_defer()` batch call, and (c) per-point I/O replaced by `store_batch()` with 1 fsync per storage. The result is a 3-phase pipeline: batch storage, per-point secondary updates (no storage locks held), then batch HNSW insert. On local benchmarks (i9-14900KF, 10K/384D), this closed the throughput gap between `upsert()` and `upsert_bulk()` from ~19x to ~1x.

5. **Graduated ef_construction (v1.9.3)** — For batches >= 1000 vectors, `ef_construction` is varied across three phases following a VAMANA/DiskANN-inspired pattern (`BatchEfSchedule`):

   | Phase | Fraction | ef_construction | Purpose |
   |-------|----------|-----------------|---------|
   | Scaffold | First 10% | Full ef | Build a high-quality backbone graph |
   | Bulk | Middle 80% | 0.5x ef (floor: 2*M) | Fast insertion leveraging existing scaffold |
   | Finalize | Last 10% | 0.75x ef (floor: 2*M) | Restore edge quality at graph periphery |

   The `2*M` floor ensures the candidate pool is never smaller than the number of neighbors to select, preserving graph connectivity. For small batches (< 1000), all nodes use full `ef_construction` unchanged.

6. **Pre-allocated vector storage (v1.9.3)** — `allocate_batch()` splits into two lock scopes: `reserve_vector_capacity()` (cold path -- may resize the buffer under a write lock) followed by `bulk_push_vectors()` (hot path -- bulk memcpy into pre-reserved space). This reduces write-lock contention during batch insert because the expensive reallocation only happens once.

7. **Lock-free entry-point promotion (v1.9.3)** — HNSW entry-point updates during batch insert use atomic CAS (`compare_exchange`) instead of a mutex, eliminating a serialization point for concurrent inserters.

8. **WAL deferred sync (v1.9.3)** — `upsert_bulk_streaming()` skips intermediate `fsync` calls between streaming batches, syncing only on the final batch. This improves bulk import throughput when data can be re-derived on crash.

---

## Search Quality Modes

VelesDB provides two complementary quality enums for controlling search precision at
query time.

### SearchQuality (HNSW-level)

Defined in `velesdb_core::index::hnsw::SearchQuality`. Controls the `ef_search`
parameter with dynamic scaling based on the requested result count `k`.

| Variant | Base ef_search | Scaling | Approx. Recall | Use Case |
|---------|---------------|---------|----------------|----------|
| `Fast` | 64 | max(64, k*2) | ~92% | Real-time serving, low latency |
| `Balanced` (default) | 128 | max(128, k*4) | ~99% | General purpose, production |
| `Accurate` | 512 | max(512, k*16) | ~100% | Analytics, batch processing |
| `Perfect` | 4096 | max(4096, k*100) | 100% | Ground truth, evaluation |
| `Custom(n)` | n | n | Varies | Fine-grained control |
| `Adaptive { min_ef, max_ef }` | min_ef | escalates to max_ef | 95%+ | Mixed workloads, latency-sensitive |

### Adaptive Search

The `Adaptive` variant uses a two-phase approach to reduce median latency by 2-4x
while maintaining recall on hard queries:

1. **Phase 1**: Search with `min_ef` (e.g., 32). Fast result for easy queries.
2. **Phase 2**: Compute result spread (`max_dist / min_dist`). If spread > 2.0
   (hard query with scattered results), re-search with doubled ef (up to `max_ef`).

```rust
use velesdb_core::SearchQuality;

// Typical configuration: start at ef=32, cap at ef=512
let quality = SearchQuality::Adaptive { min_ef: 32, max_ef: 512 };
let results = index.search_with_quality(&query, 10, quality);
```

**When to use**: Production workloads where most queries are "easy" (hit a dense
cluster) but some are "hard" (scattered results). Adaptive saves 2-4x latency on
easy queries while gracefully escalating for hard ones.

### Custom and Adaptive via REST API (v1.9.2)

The `mode` parameter in the search endpoint accepts `Custom` and `Adaptive` using
a colon-separated syntax:

```bash
# Custom ef_search = 256
curl -X POST http://localhost:8080/collections/docs/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [...], "top_k": 10, "mode": "custom:256"}'

# Adaptive with min_ef=32, max_ef=512
curl -X POST http://localhost:8080/collections/docs/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [...], "top_k": 10, "mode": "adaptive:32:512"}'
```

This complements the existing named presets (`fast`, `balanced`, `accurate`,
`perfect`, `autotune`) with fine-grained control over `ef_search`.

### SearchMode (Collection-level)

Defined in `velesdb_core::config::SearchMode`. A simpler preset used at the collection
configuration level.

| Variant | ef_search | Use Case |
|---------|-----------|----------|
| `Fast` | 64 | Real-time serving |
| `Balanced` (default) | 128 | General purpose |
| `Accurate` | 512 | Analytics |
| `Perfect` | exhaustive | Ground truth |

### AutoTune Mode (v1.7.2)

The `AutoTune` variant computes optimal `ef_search` automatically from the
collection's size and vector dimension, removing the need for manual ef tuning.

Internally it calls `auto_ef_range(count, dimension, k)` which returns a
`(min_ef, max_ef)` pair used in an adaptive two-phase search (same mechanism
as `Adaptive`).

**Scaling tiers:**

| Collection Size | Base ef (per k) | Strategy |
|-----------------|-----------------|----------|
| <= 1K           | k * 2           | Conservative — small datasets are fast regardless |
| 1K - 10K        | k * 4           | Moderate |
| 10K - 100K      | k * 8           | Moderate-aggressive |
| > 100K          | k * 12          | Aggressive — large datasets need wider exploration |

A dimension factor of **1.5x** is applied for dimensions > 512 (sparser
neighborhoods require more candidates). The `max_ef` is always `4 * min_ef`,
giving the second adaptive phase headroom for hard queries.

**REST API:**

```json
POST /collections/{name}/search
{
  "vector": [0.1, 0.2, ...],
  "top_k": 10,
  "mode": "autotune"
}
```

**Python:**

```python
results = collection.search_with_quality(vector, "autotune", top_k=10)
```

**Rust:**

```rust
use velesdb_core::SearchQuality;
let results = index.search_with_quality(&query, 10, SearchQuality::AutoTune);
```

**When to use:** Recommended for applications that want good recall without
manual ef tuning. AutoTune provides a solid default that scales with your data
— start with it and only switch to manual `Custom(ef)` or `Adaptive` if you
need to squeeze out the last microseconds.

### Choosing Between Them

- Use `SearchMode` when configuring a collection's default search behavior
- Use `SearchQuality` when you need per-query control or dynamic k-scaling
- Use `SearchQuality::AutoTune` when you want good recall without manual ef tuning

---

## Durability Modes

The `DurabilityMode` enum (`velesdb_core::storage::DurabilityMode`) controls how
aggressively VelesDB syncs data to disk.

| Mode | Behavior | Write Latency | Data Safety | Use Case |
|------|----------|---------------|-------------|----------|
| `Fsync` (default) | flush + `sync_all()` | Highest | Power-loss safe | Production workloads |
| `FlushOnly` | flush to OS only | Medium | Safe on clean shutdown | Development, testing |
| `None` | No sync | Lowest | Data loss on any crash | Bulk import, ephemeral data |

### Bulk Import Pattern

For maximum throughput during initial data loading, disable durability during import
and re-enable it for production queries:

```rust
use velesdb_core::storage::DurabilityMode;

// 1. Configure collection with turbo HNSW params for fast build
//    and DurabilityMode::None for maximum write throughput
//    (set via collection storage options at creation time)

// 2. Bulk insert all vectors
//    ...

// 3. After import: rebuild index with production params
//    and switch to DurabilityMode::Fsync for production safety
```

### Guidance

- Always use `Fsync` in production unless you can tolerate data loss on power failure
- `FlushOnly` is acceptable for development and CI testing
- `None` should only be used for ephemeral or re-derivable data (e.g., bulk import
  where you can re-import if the process crashes)

---

## Memory Estimation

Use these tables to estimate memory requirements for your workload.

### Per-Vector Memory (bytes)

| Dimension | Full (f32) | SQ8 (u8) | Binary (1-bit) |
|-----------|-----------|----------|----------------|
| 128 | 512 | 128 | 16 |
| 384 | 1,536 | 384 | 48 |
| 768 | 3,072 | 768 | 96 |
| 1,536 | 6,144 | 1,536 | 192 |
| 3,072 | 12,288 | 3,072 | 384 |

### HNSW Index Overhead

The HNSW graph adds approximately `M * 2 * 8` bytes per vector (each link stores a
`u64` neighbor ID, with up to `2*M` links per node across all layers).

| M (max_connections) | Overhead per Vector |
|--------------------|---------------------|
| 16 | ~256 bytes |
| 32 | ~512 bytes |
| 64 | ~1,024 bytes |
| 128 | ~2,048 bytes |

### Total Memory Examples

For 1 million vectors at 768 dimensions:

| Configuration | Vector Storage | HNSW Overhead | Total |
|--------------|----------------|---------------|-------|
| Full, M=32 | 2.87 GB | 488 MB | ~3.4 GB |
| SQ8, M=32 | 732 MB | 488 MB | ~1.2 GB |
| Binary, M=32 | 91 MB | 488 MB | ~579 MB |
| SQ8, M=16 | 732 MB | 244 MB | ~976 MB |

### Formula

```
total_bytes = num_vectors * (dim * bytes_per_element + M * 2 * 8)
```

Where `bytes_per_element` is:
- Full: 4 (f32)
- SQ8: 1 (u8) + small metadata overhead
- Binary: 1/8 (1 bit)

---

## Common Tuning Scenarios

### Scenario 1: RAG with OpenAI Embeddings (1536D)

High-dimensional embeddings for retrieval-augmented generation.

```rust
// 1536D embeddings, expecting ~500K documents
let params = HnswParams::for_dataset_size(1536, 500_000);
// Use SQ8 to keep memory manageable at 1536D
// StorageMode::SQ8 -> ~750 MB for vectors instead of ~3 GB
```

- Use `SearchQuality::Balanced` for production queries
- Use `SearchQuality::Accurate` for evaluation runs

### Scenario 2: Image Similarity (128D)

Low-dimensional feature vectors for near-duplicate detection.

```rust
// 128D, small dataset ~50K images
let params = HnswParams::auto(128);
// Full precision is fine at 128D (only ~6 MB for vectors)
```

- Use `SearchQuality::Fast` for real-time deduplication
- Binary quantization is NOT recommended for low dimensions

### Scenario 3: Edge/IoT Device (768D, Memory Limited)

Running on a device with 512 MB total RAM.

```rust
// 768D embeddings, ~100K vectors, tight memory budget
let params = HnswParams::for_dataset_size(768, 100_000);
// StorageMode::Binary -> ~10 MB for vectors
// Or StorageMode::SQ8 -> ~77 MB for vectors (better recall)
```

- Use `DurabilityMode::FlushOnly` if the device has flash storage (avoid excessive fsync)
- Use `SearchQuality::Fast` to minimize query-time memory and latency

### Scenario 4: Bulk Data Migration

Loading millions of vectors from an external source.

```rust
// Phase 1: Fast import
let import_params = HnswParams::turbo();
// DurabilityMode::None for maximum throughput

// Phase 2: Production rebuild
let prod_params = HnswParams::for_dataset_size(768, 2_000_000);
// DurabilityMode::Fsync for safety
```

---

## Compile-Time CPU Targeting

By default VelesDB builds for the generic `x86-64` baseline, which means SIMD
dispatch falls back to runtime feature detection. For local benchmarks you can
unlock the full instruction set of your CPU (AVX2, AVX-512, etc.) by setting
`target-cpu=native`.

### One-Shot (Recommended)

Pass the flag via the environment — nothing is committed:

```bash
RUSTFLAGS="-C target-cpu=native" cargo bench -p velesdb-core --bench simd_benchmark -- --noplot
```

### Persistent (Local Only)

`.cargo/config.toml` contains a commented section at the bottom:

```toml
# [target.x86_64-pc-windows-msvc]
# rustflags = ["-C", "target-cpu=native"]
```

Uncomment the block that matches your host target for persistent local
optimizations. **Do NOT commit the uncommented version** — CI runners use
generic x86-64 targets. Proc-macro crates compiled with `target-cpu=native`
will crash with `SIGILL` / `STATUS_ILLEGAL_INSTRUCTION` if restored from a
cache on a CPU that lacks those instructions.

### SIMD Padding Utility

VelesDB's AVX2 kernels process 8 `f32` lanes at a time. Vectors whose
dimension is not a multiple of 8 incur a scalar tail-loop. You can eliminate
this overhead by pre-padding vectors:

```rust
use velesdb_core::pad_to_simd_width;

let raw = vec![0.1_f32; 768]; // 768 is already 8-aligned — no-op
let padded = pad_to_simd_width(&raw);
assert_eq!(padded.len(), 768);

let raw = vec![0.1_f32; 100]; // 100 → 104 (next multiple of 8)
let padded = pad_to_simd_width(&raw);
assert_eq!(padded.len(), 104);
```

Both query and stored vectors must be padded to the same length for correct
distance results. This is only necessary for custom pipelines — VelesDB's
built-in search handles alignment internally.

---

## Performance Tips

1. **Dimension alignment**: VelesDB's SIMD kernels (AVX2/AVX-512) are most efficient
   when dimensions are multiples of 16. Pad vectors if needed.

2. **Avoid over-provisioning M**: Doubling M roughly doubles memory and insert time.
   Start with `HnswParams::auto()` and increase only if recall is
   insufficient.

3. **ef_search scales with k**: `SearchQuality` automatically scales `ef_search` with
   the number of requested results. Requesting `k=100` with `Balanced` uses
   `max(128, 400)` = 400, not 128.

4. **Quantize after tuning**: Get your HNSW parameters right with `StorageMode::Full`
   first, then switch to SQ8/Binary and verify recall is acceptable.

5. **Monitor guard-rails**: If queries hit `VELES-027` (GuardRail), you are exceeding
   configured limits. Tune the guard-rail thresholds or narrow your queries with
   filters.

6. **Use Adaptive for mixed workloads**: If your query distribution has both easy
   (cluster-adjacent) and hard (scattered) queries, `SearchQuality::Adaptive`
   automatically detects query difficulty and only escalates ef for hard queries.
   This can cut median latency by 2-4x compared to a fixed `Balanced` mode.

7. **Filter-then-hydrate**: When using `search_with_filter`, VelesDB tests metadata
   filters before retrieving vectors. For selective filters (<25% pass rate), this
   avoids loading hundreds of KB of unnecessary vector data. The over-fetch factor
   adapts automatically based on estimated filter selectivity.

8. **Software pipelining (v1.7.2)**: Search automatically uses speculative prefetch
   for vectors whose dimension spans at least 2 cache lines (>= 32 dimensions) and
   whose dataset exceeds ~8 MB (not fully L3-resident). The pipeline peeks at the
   next candidate in the min-heap and prefetches its neighbor vectors while computing
   distances for the current batch, hiding main-memory latency behind ALU work. This
   is fully automatic and produces identical results to the non-pipelined path — no
   configuration needed.

9. **PDX columnar layout (v1.7.2)**: After BFS graph reordering
   (`reorder_for_locality()`), vectors can be transposed into a block-columnar (PDX)
   layout via `ColumnarVectors`. Each block contains 64 vectors with dimensions
   interleaved, enabling the SIMD kernel to broadcast `query[d]` once and compute
   the d-th contribution for all 64 vectors simultaneously. This achieves 64x better
   register reuse compared to standard array-of-structures layout. The conversion is
   automatic after reordering for indices above 1000 vectors.

---

## Performance Metrics (v1.7.2)

Reference benchmarks captured on 2026-03-27 with `target-cpu=native`.

**Hardware:** Intel i9-14900KF, 64 GB DDR5, Windows 11

### Rust Core (Criterion)

| Benchmark | Configuration | Result |
|-----------|---------------|--------|
| Search top-10 | 5K vectors, 768D, Cosine | ~55 us |
| Parallel insert | 1K vectors, 768D | ~20.7 ms (48.2K vec/s) |
| SIMD dot product | 768D (AVX2/AVX-512) | ~21.7 ns |

### Python Bindings (PyO3 + NumPy)

| Benchmark | Configuration | Result |
|-----------|---------------|--------|
| Bulk insert | 10K vectors, 384D | ~15.4K vec/s |
| Search (avg) | 10K vectors, 384D, top-10 | ~630 us |

These numbers include all overhead (lock acquisition, HNSW traversal, result
conversion). Actual SIMD kernel throughput is higher — the ~21.7 ns dot product
processes 768 floats at >35 GFLOP/s per core.

**Note:** Micro-benchmarks on Windows have 5-10% noise. Use the numbers above as
order-of-magnitude references, not exact targets. For reproducible comparisons,
run `cargo bench` locally with `RUSTFLAGS="-C target-cpu=native"` and compare
against your own baseline.
