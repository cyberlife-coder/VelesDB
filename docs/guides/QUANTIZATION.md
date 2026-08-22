# 📦 Quantization - Vector Compression

*User guide for reducing the memory footprint*

---

## 🎯 What is Quantization?

**Quantization** reduces the in-memory size of vectors while preserving excellent search accuracy. VelesDB offers four methods:

- **SQ8** (Scalar 8-bit) — one byte per dimension via min/max scaling, no training
- **PQ** (Product Quantization) — codebook-based sub-vector encoding, trained
- **Binary** (1-bit) — one sign bit per dimension, no training
- **RaBitQ** (Randomized Binary) — binary encoding behind a trained orthogonal rotation

This guide covers how each method works, its training workflow, and its
persistence behavior. The compression ratios, recall impact, and training cost
per method are consolidated in one place:
[Tuning Guide — When to Use Each Mode](TUNING_GUIDE.md#when-to-use-each-mode).

### What each storage mode actually does

The compression figures above describe the quantization **primitives**. In the
**collection search path**, only some modes are wired up:

| Storage mode | Collection storage + search path |
|--------------|----------------------------------|
| `full` | f32 (baseline) |
| `sq8` | f32 — behaves as `full` today; int8 traversal tracked in [#2112](https://github.com/cyberlife-coder/VelesDB/issues/2112) |
| `binary` | f32 — behaves as `full` today |
| `pq` | f32 storage + ADC-rescored search (wired) |
| `rabitq` | quantized traversal (wired end-to-end) |

Pick `rabitq` (or `pq`) when you want a quantized query hot path. `sq8` and
`binary` are accepted and persisted so the intent survives a reopen, but they
currently change neither memory use nor throughput.

---

## 🚀 SQ8: 4x Compression

> **Status: collection mode behaves as `full`.**
> A collection in `storage='sq8'` mode stores and searches full-precision
> f32 vectors. (Earlier releases also filled an SQ8 side-cache at insertion
> time that no search path ever read — pure CPU + unbounded RAM cost — so
> that cache was removed.) Wiring the in-tree int8 traversal engine into an
> HNSW backend is tracked in
> [#2112](https://github.com/cyberlife-coder/VelesDB/issues/2112). The SQ8
> primitives below (`QuantizedVector`, SIMD distances) are fully functional
> for direct programmatic use.

Each `f32` value (4 bytes) is converted to a `u8` (1 byte):

```
Before: [0.123, 0.456, 0.789, ...]  → 768 × 4 = 3072 bytes
After:  [31, 116, 201, ...]         → 768 × 1 = 776 bytes (with metadata)
```

### Rust Example

```rust
use velesdb_core::quantization::{QuantizedVector, dot_product_quantized_simd};

// Create a quantized vector
let original = vec![0.1, 0.5, 0.9, -0.3, 0.0];
let quantized = QuantizedVector::from_f32(&original);

// Search with an f32 query vector
let query = vec![0.2, 0.4, 0.8, -0.2, 0.1];
let similarity = dot_product_quantized_simd(&query, &quantized);

println!("Similarity: {:.4}", similarity);
println!("Memory saved: {}%", 
    (1.0 - quantized.memory_size() as f32 / (original.len() * 4) as f32) * 100.0);
```

### Performance

| Operation | f32 (768D) | SQ8 (768D) | Gain |
|-----------|------------|------------|------|
| **Memory** | 3072 bytes | 776 bytes | **4x** |
| **Dot Product** | 41 ns | ~60 ns | -30% |
| **Recall@10** | 99.4% | ~97.5% | -2% |

---

## ⚡ Binary: 32x Compression

> **Status: collection mode behaves as `full`.**
> Same as SQ8: `storage='binary'` mode stores and searches full-precision
> f32 (its never-read insertion-time cache was removed). For effective 32x
> compression in the query path, use RaBitQ. The `BinaryQuantizedVector`
> primitives remain directly usable.

Each `f32` value becomes **1 bit**:
- Value ≥ 0 → 1
- Value < 0 → 0

```
Before: [0.5, -0.3, 0.1, -0.8, ...]  → 768 × 4 = 3072 bytes
After:  [0b10100110, ...]            → 768 ÷ 8 = 96 bytes
```

### Rust Example

```rust
use velesdb_core::quantization::BinaryQuantizedVector;

// Create a binary vector
let vector = vec![0.5, -0.3, 0.1, -0.8, 0.2, -0.1, 0.9, -0.5];
let binary = BinaryQuantizedVector::from_f32(&vector);

// Hamming distance (number of differing bits)
let other = BinaryQuantizedVector::from_f32(&[0.1, -0.1, 0.2, -0.9, 0.3, -0.2, 0.8, -0.4]);
let distance = binary.hamming_distance(&other);

println!("Hamming distance: {}", distance);
println!("Memory: {} bytes (vs {} bytes f32)", 
    binary.memory_size(), vector.len() * 4);
```

### Binary use cases

- **Audio/image fingerprints**: Duplicate detection
- **Locality-sensitive hashing**: Ultra-fast approximate search
- **IoT/Edge**: Very limited RAM

---

## PQ: Product Quantization (8-32x)

### How does it work?

The vector is split into **m sub-vectors**, each quantized independently against a **codebook** of k centroids (k-means++ training). Each sub-vector is replaced by an 8-bit index into the codebook.

```
Before: [0.1, 0.2, ..., 0.8]  → 768 × 4 = 3072 bytes
After:  [idx_1, idx_2, ..., idx_m]  → m × 1 = 8 bytes (m=8)
```

### Configuration

| Parameter | Default | Description |
|-----------|--------|-------------|
| `m` | 8 (recommended; required, no struct default) | Number of subspaces (must divide the dimension) |
| `k` | 256 | Codebook size per subspace (centroids) |
| `opq_enabled` | `false` | Enables Optimized PQ (OPQ rotation) |
| `rescore_oversampling` | `Some(4)` | Oversampling factor for rescoring |

### When to use PQ?

- **Large datasets** (100K+ vectors) where memory is a limiting factor
- **Approximate search is acceptable** (85-95% recall with rescoring)
- **Low latency required**: ADC (Asymmetric Distance Computation) avoids decoding the vectors

### Training via VelesQL

```sql
TRAIN QUANTIZER ON my_collection WITH (m=8, k=256)
```

Training is **explicit**: it is not triggered automatically. The collection must contain enough vectors (at least k vectors recommended).

**Persistence**: `TRAIN QUANTIZER` saves the codebook (`codebook.pq`, plus
`rotation.opq` for OPQ) into the collection directory. On reopen, the
codebook is reloaded and the PQ cache is rebuilt by re-encoding all stored
vectors (O(n) cost at open time) — ADC rescoring therefore survives
restarts. A quantizer trained lazily at insertion time (`storage='pq'` mode
without `TRAIN QUANTIZER`) is persisted too: every full flush writes the
current codebook to disk (`flush_pq_codebook`), so lazy-trained PQ also
survives restarts — at parity with the RaBitQ flush hook.

### Training via Rust

```rust
use velesdb_core::quantization::ProductQuantizer;

let pq = ProductQuantizer::train(&vectors, m, k)?;
// Explicit persistence (TRAIN QUANTIZER does this automatically):
pq.save_codebook(collection_dir)?;
```

### OPQ (Optimized Product Quantization)

OPQ applies an orthogonal rotation to the vectors before PQ quantization. This rotation minimizes the quantization error by aligning the data variance with the subspaces.

**When to enable OPQ:**
- Data with strong correlations between dimensions (clustered embeddings)
- Typical recall improvement: +3-8% on correlated data
- Extra cost: 2x training time (PCA rotation matrix computation)

**When not to enable OPQ:**
- Already decorrelated or uniformly distributed data
- Low dimensionality (< 64), where the rotation brings no significant gain

### PQ Performance

| Configuration | Memory (768D, 100K vecs) | Recall@10 | Latency |
|---------------|--------------------------|-----------|---------|
| f32 (baseline) | 295 MB | 99.4% | ~2 ms |
| PQ m=8, k=256 | ~8 MB | ~85% | ~1 ms |
| PQ m=16, k=256 | ~16 MB | ~90% | ~1.2 ms |
| PQ m=8 + rescore 4x | ~8 MB + rescore | ~93% | ~3 ms |
| PQ m=8 + OPQ | ~8 MB | ~88% | ~1 ms |

---

## RaBitQ: Randomized Binary Quantization (32x)

> **Status: wired end-to-end into the collection query path, including
> across restarts.**
> A collection created with `storage='rabitq'` uses the binary-traversal
> HNSW backend (`RaBitQPrecisionHnsw`). `TRAIN QUANTIZER` with
> `type=rabitq` trains the quantizer, persists it to `rabitq.idx` AND
> installs it immediately into the live index (O(n·d) re-encoding of
> existing vectors). On reopen, `rabitq.idx` is reloaded and the vectors
> are re-encoded (O(n·d) cost at open time, same class as HNSW gap
> recovery). If the collection was created with a different storage mode,
> training persists the index and switches the config; the RaBitQ backend
> takes effect on the next open. A quantizer trained automatically (lazy,
> 1000-insertion threshold) is also persisted to `rabitq.idx` on a full
> flush, at parity with the PQ codebook.

> **Resident-memory caveat:** the RaBitQ backend keeps the full-precision
> f32 vectors in the index for exact re-ranking, with the 1-bit codes held
> *alongside* them. The 32x figure is the size of the codes, not of the
> resident set — today the mode buys traversal speed (32x memory-bandwidth
> reduction in the hot loop), not a smaller RAM floor. A codes-resident
> variant for small-RAM devices is part of
> [#2112](https://github.com/cyberlife-coder/VelesDB/issues/2112).

### How does it work?

RaBitQ combines binary compression (1 bit per dimension) with a **random orthogonal rotation** that preserves distances. Unlike naive binary quantization, the orthogonal rotation spreads the information more uniformly across all bits.

```
Before:   [0.5, -0.3, 0.1, ...]  → 768 × 4 = 3072 bytes
Rotation: R × v = [0.2, 0.4, -0.1, ...]
After:    [0b10100110, ...]      → 768 / 8 = 96 bytes
```

### Advantages over naive Binary

| Aspect | Naive Binary | RaBitQ |
|--------|------------|--------|
| **Recall@10** | ~85% | ~90-93% |
| **Compression** | 32x | 32x |
| **Training** | No | Yes (rotation) |
| **Distance** | Hamming | Binary inner product |

### Use cases

- Same memory constraints as Binary, but better recall
- Large high-dimensional datasets (128D+) where the random rotation is more effective
- Fast pre-filtering followed by exact rescoring

---

## Method comparison

The cross-method comparison table (compression, Recall@10, training cost per
method) moved to the
[Tuning Guide — When to Use Each Mode](TUNING_GUIDE.md#when-to-use-each-mode),
the single home for those numbers. Keep in mind the wiring caveat: in the
collection query path only **RaBitQ** and **PQ** are wired up today (see the
status callouts above) — the SQ8/Binary collection modes behave as `full`.

---

## Choosing the right method

| Scenario | Recommendation |
|----------|----------------|
| **General production** | f32 (default) — SQ8 adds nothing until [#2112](https://github.com/cyberlife-coder/VelesDB/issues/2112) lands |
| **Large dataset (100K+)** | PQ m=8 + rescore |
| **Very limited RAM** | f32 with modest HNSW `M` — no current mode shrinks the resident set (see [#2112](https://github.com/cyberlife-coder/VelesDB/issues/2112)) |
| **Maximum precision** | f32 (no quantization) |
| **High compression + good recall** | RaBitQ |
| **Fingerprints/hashes** | `BinaryQuantizedVector` primitives (direct use) |
| **Correlated data** | PQ + OPQ |

> Note: these recommendations compare the methods as such. In the collection
> query path, only **RaBitQ** and **PQ** are wired up today (see the status
> callouts above) — the SQ8/Binary modes maintain caches there that search
> does not consume yet.

---

## 🔧 Full API

### QuantizedVector (SQ8)

```rust
// Creation
let q = QuantizedVector::from_f32(&vector);

// Properties
q.dimension();      // Number of dimensions
q.memory_size();    // Size in bytes
q.min;              // Original min value
q.max;              // Original max value

// Reconstruction (lossy)
let reconstructed = q.to_f32();

// Serialization
let bytes = q.to_bytes();
let restored = QuantizedVector::from_bytes(&bytes)?;
```

### BinaryQuantizedVector

```rust
// Creation
let b = BinaryQuantizedVector::from_f32(&vector);

// Properties
b.dimension();      // Original dimensions
b.memory_size();    // Bytes (dimension / 8)
b.get_bits();       // Vec<bool> of the bits

// Distances
let dist = b.hamming_distance(&other);  // Differing bits
let sim = b.hamming_similarity(&other); // 0.0 to 1.0

// Serialization
let bytes = b.to_bytes();
let restored = BinaryQuantizedVector::from_bytes(&bytes)?;
```

### SIMD Distance Functions

```rust
use velesdb_core::quantization::*;

// Optimized dot product
let dot = dot_product_quantized_simd(&query, &quantized);

// Squared Euclidean distance
let dist = euclidean_squared_quantized_simd(&query, &quantized);

// Cosine similarity
let cos = cosine_similarity_quantized_simd(&query, &quantized);
```

---

## 🧪 Benchmarks

Run the benchmarks:

```bash
cargo bench --bench quantization_benchmark
```

Typical results (768D, modern CPU):

```
SQ8 Encode/768        time:   [1.2 µs 1.3 µs 1.4 µs]
Dot Product f32_simd  time:   [41 ns 42 ns 43 ns]
Dot Product sq8_simd  time:   [58 ns 60 ns 62 ns]
```

---

See also: [Tuning Guide](TUNING_GUIDE.md) — the numeric home for the
quantization comparison, memory estimation, and mode defaults ·
[Search Modes](SEARCH_MODES.md) — the recall/latency modes these storage
modes combine with.

---

*VelesDB Documentation -- Last updated: 2026-08-08 · Applies to: velesdb-core 5.2.0*
