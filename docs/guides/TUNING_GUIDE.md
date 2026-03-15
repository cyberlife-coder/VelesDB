# VelesDB Tuning Guide

A practical reference for configuring VelesDB to balance recall, latency, memory, and
durability for your workload.

---

## Quantization Modes

VelesDB supports five storage modes via the `StorageMode` enum
(`velesdb_core::quantization::StorageMode`). Each trades memory for recall accuracy.

### When to Use Each Mode

| Mode | Compression | Recall Impact | Training Required | Best For |
|------|-------------|---------------|-------------------|----------|
| `Full` (default) | 1x (baseline) | Perfect | No | Small datasets (<100K), high-precision needs |
| `SQ8` | 4x | ~1-2% recall loss | No | Medium datasets (100K-10M), general purpose |
| `Binary` | 32x | ~10-15% recall loss | No | Edge/IoT, fingerprints, memory-constrained |
| `ProductQuantization` | 8-32x | ~5-15% recall loss | Yes | Large datasets, aggressive compression |
| `RaBitQ` | 32x | ~5-10% recall loss | Yes (rotation matrix) | High compression with better recall than Binary |

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
| M | `max_connections` | auto (16-64 based on dim) | Bi-directional links per node. Higher = better recall, more memory |
| ef_construction | `ef_construction` | auto (300-400 based on dim) | Build-time candidate list size. Higher = better graph quality, slower build |
| max_elements | `max_elements` | 100,000 | Initial capacity (grows automatically if exceeded) |
| storage_mode | `storage_mode` | `StorageMode::Full` | Vector compression mode |

### Auto-Tuned Defaults by Dimension

| Dimension | M (`max_connections`) | `ef_construction` |
|-----------|----------------------|-------------------|
| <= 256 | 16 | 300 |
| >= 257 | 32 | 400 |

### Dataset-Size-Aware Parameters

Use `HnswParams::for_dataset_size(dimension, count)` for optimal parameters at scale:

| Dataset Size | M (384D) | M (768D) | ef_construction |
|-------------|----------|----------|-----------------|
| <= 10K | 16 | 32 | 200-400 |
| <= 100K | 32-64 | 64-128 | 800-1600 |
| <= 500K | 64-96 | 96-128 | 1200-2000 |
| <= 1M | 64-96 | 96-128 | 800-1600 |

### Convenience Constructors

```rust
use velesdb_core::index::hnsw::HnswParams;

// Auto-tune for dimension (default)
let params = HnswParams::default_for_dimension(768);

// Scale-aware: optimized for 500K vectors at 768D
let params = HnswParams::for_dataset_size(768, 500_000);

// Million-scale: M=128, ef_construction=1600
let params = HnswParams::million_scale(768);

// Maximum recall: aggressive params for evaluation
let params = HnswParams::max_recall(768);

// Fastest build: M=8, ef_construction=100 (for bulk import)
let params = HnswParams::turbo();

// Fully custom
let params = HnswParams::custom(48, 800, 200_000);
```

### Tuning Guidelines

- **Low latency priority**: Lower M (8-16), use `HnswParams::turbo()` for import then rebuild
- **High recall priority**: Higher M (32-64), use `HnswParams::max_recall(dim)`
- **Memory constrained**: Lower M (4-8), combine with `StorageMode::SQ8` or `StorageMode::Binary`
- **Bulk import**: Use `HnswParams::turbo()` during import, rebuild with production params after

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

### SearchMode (Collection-level)

Defined in `velesdb_core::config::SearchMode`. A simpler preset used at the collection
configuration level.

| Variant | ef_search | Use Case |
|---------|-----------|----------|
| `Fast` | 64 | Real-time serving |
| `Balanced` (default) | 128 | General purpose |
| `Accurate` | 256 | Analytics |
| `Perfect` | exhaustive | Ground truth |

### Choosing Between Them

- Use `SearchMode` when configuring a collection's default search behavior
- Use `SearchQuality` when you need per-query control or dynamic k-scaling

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
let params = HnswParams::default_for_dimension(128);
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
   Start with `HnswParams::default_for_dimension()` and increase only if recall is
   insufficient.

3. **ef_search scales with k**: `SearchQuality` automatically scales `ef_search` with
   the number of requested results. Requesting `k=100` with `Balanced` uses
   `max(128, 400)` = 400, not 128.

4. **Quantize after tuning**: Get your HNSW parameters right with `StorageMode::Full`
   first, then switch to SQ8/Binary and verify recall is acceptable.

5. **Monitor guard-rails**: If queries hit `VELES-027` (GuardRail), you are exceeding
   configured limits. Tune the guard-rail thresholds or narrow your queries with
   filters.
