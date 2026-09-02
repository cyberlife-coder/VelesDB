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
| `sq8` | int8 graph traversal + exact f32 re-ranking (Euclidean/Cosine; other metrics stay f32) |
| `binary` | f32 — behaves as `full` today |
| `pq` | f32 storage + ADC-rescored search (wired) |
| `rabitq` | quantized traversal (wired end-to-end) |

Pick `rabitq`, `sq8` (Euclidean/Cosine) or `pq` when you want a quantized
query hot path. `binary` is accepted and persisted so the intent survives a
reopen, but currently changes neither memory use nor the search path — use
`rabitq` for a real 1-bit query path.

---

## 🚀 SQ8: 4x Compression

> **Status: wired into the collection query path for Euclidean and Cosine,
> including across restarts.**
> A collection created with `storage='sq8'` uses the int8-traversal HNSW
> backend (`Sq8PrecisionHnsw`): graph traversal reads 1 byte per dimension
> instead of 4 (4x memory-bandwidth reduction in the hot loop) and the
> final top-k is re-ranked with exact f32 distances. The quantizer trains
> lazily after 1000 inserts or explicitly via `TRAIN QUANTIZER` with
> `type=sq8`; both persist to `sq8.idx` (the lazy one on each full flush)
> and are re-installed on reopen with an O(n·d) re-encode, mirroring
> RaBitQ. Int8 traversal engages at 10 000+ vectors; below that, and on
> metrics whose ordering int8 L2 cannot preserve (DotProduct, Hamming,
> Jaccard), search stays exact f32.

> **What the 4x figure is, and what it is not:** it is the size of the
> codes. The f32 vectors are still there — exact re-ranking needs them — but
> since [#2112](https://github.com/cyberlife-coder/VelesDB/issues/2112) they
> live in a file-backed arena rather than an anonymous allocation, so the
> kernel can reclaim them. Measured at 100 000 x 768-d: **anonymous RSS falls
> from 385 MiB to 150 MiB**, a 61% cut in the memory a device without swap
> cannot get back. Total RSS *rises* 11% over `Full` — the f32 moves rather
> than disappears, and the codes are additive on top. The trade is more total
> memory for less un-evictable memory. See
> [Measured resident set](#measured-resident-set) below.

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

> **What the 32x figure is, and what it is not:** it is the size of the
> codes. The backend keeps the full-precision f32 for exact re-ranking, with
> the 1-bit codes alongside. Since
> [#2112](https://github.com/cyberlife-coder/VelesDB/issues/2112) that f32
> sits in a file-backed arena, so it is evictable rather than pinned; the mode
> buys both traversal speed (32x memory-bandwidth reduction in the hot loop)
> and a lower un-evictable floor. The measurement below was taken on SQ8; the
> arena is shared by both quantized backends, so RaBitQ moves the same 4
> bytes per dimension out of anonymous memory.

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
collection query path **RaBitQ**, **SQ8** (Euclidean/Cosine) and **PQ** are
wired up today (see the status callouts above) — the Binary collection mode
behaves as `full`.

---

## Measured resident set

Taken 2026-08-24 on a 4-vCPU Linux container, 16 GiB RAM, via
`cargo run --release --example resident_set --features persistence -- <mode>`
with `mode` one of `full`, `sq8`, `cold`, one per process. Numbers
are deltas across building one collection of **100 000 x 768-d** vectors, each
mode in its own process — the first collection built in a process absorbs
every one-time cost (thread pools, allocator arenas), so measuring both in one
run charges the whole difference to whichever ran first.

| Mode | Anonymous RSS | File-backed RSS | Total | Build |
|---|---|---|---|---|
| `Full` (heap arena) | **385.0 MiB** | 132.9 MiB | 517.9 MiB | 28.2 s |
| `SQ8` (file-backed arena) | **150.4 MiB** | 425.8 MiB | 576.2 MiB | 134.6 s |
| Delta | **−234.6 MiB (−61%)** | +292.9 MiB | +58.3 MiB (+11%) | 4.8x |

**Read the anonymous column.** A mapped file's pages count toward total RSS
while they are resident, so `VmRSS` barely moves and reporting it would hide
the change. What matters on a device without swap is `RssAnon`: the kernel can
reclaim file-backed pages by dropping them, and anonymous pages only by
swapping. The file-backed arena moves 293.0 MiB — exactly `100 000 x 768 x 4` —
out of the column that cannot be reclaimed — anonymous RSS falls 234.6 MiB,
not the full 293.0, because the 73.2 MiB of codes land in that same column.
The residual 150.4 MiB is those codes plus the graph, which is the
`codes + graph` the design aimed at. Total RSS rises 58.3 MiB: this buys a
lower un-evictable floor, not a smaller process.

The 4.8x build time is the honest other side — but almost none of it is the
arena. Filling the same 100 000 vectors into each backing, with no graph and
no quantizer in the frame:

| Backing | Fill time |
|---|---|
| heap | 58–62 ms, stable across runs |
| file-mapped | 0.30–1.35 s, rising with each consecutive run |

The heap figure is steady; the mapped one is I/O-bound and climbs as repeated
293 MiB writes fill the host's dirty-page pool, so it is reported as a range
rather than a point. Even at its worst it is **1.3 s of the 106.4 s that
separates an SQ8 build from a Full one — under 1.3%**. The rest is quantizer
training and code encoding, which `SQ8` pays with any backing.

### Why the arena is not configurable

That 1.3% ceiling is the whole case. An opt-out would let a caller avoid at
most ~1 s of build time and the cold-re-rank penalty, in exchange for
234.6 MiB of un-evictable RAM, a new persisted setting, and another branch
through the backend dispatch.

It would also mostly avoid a cost that is not being paid. A host with free
memory never has these pages reclaimed, so the 8-10 ms cold re-rank never
happens there; the mapping only costs latency once memory is tight, which is
exactly when its 61% saving is worth having. The trade is self-regulating.

Two costs *are* paid unconditionally and are the honest counterweight: the
+11% total RSS, and the 0.24 s. Reopen this if either turns out to hurt a
real deployment — measurements first, per the note above.

### Cold-page re-rank cost

The price of evictability, measured on 100 scattered vectors — a re-rank-shaped
access — with every dimension read, since a 3 KiB vector straddles one or two
pages and a distance touches all of them.

| State | Median | vs warm |
|---|---|---|
| warm | 0.14 ms | — |
| pages dropped, page cache warm | 0.55 ms | +0.4 ms |
| pages dropped, page cache dropped | 8–10 ms | +8 to +9 ms |

The two cold rows are different events and the gap between them is a factor of
15. `MADV_DONTNEED` alone takes the process's pages while the file's contents
stay cached, so the next touch is a minor fault. Reclaim under real memory
pressure takes the cache too, and the next touch is a disk read — that bottom
row is what an edge device actually pays, and it varies about 15% run to run
on shared storage.

Budget roughly **8-10 ms of added latency on the first query after a reclaim**,
per 100 candidates re-ranked. Subsequent queries touching the same vectors are
warm again.

## Choosing the right method

| Scenario | Recommendation |
|----------|----------------|
| **General production** | f32 (default); `sq8` on Euclidean/Cosine at 10K+ vectors for a lighter traversal hot loop |
| **Large dataset (100K+)** | PQ m=8 + rescore |
| **Very limited RAM** | `sq8` or `rabitq` — the f32 arena is file-backed, cutting anonymous RSS 61% at 100K x 768-d; budget ~8 ms per cold re-rank |
| **Maximum precision** | f32 (no quantization) |
| **High compression + good recall** | RaBitQ |
| **Fingerprints/hashes** | `BinaryQuantizedVector` primitives (direct use) |
| **Correlated data** | PQ + OPQ |

> Note: these recommendations compare the methods as such. In the collection
> query path, **RaBitQ**, **SQ8** (Euclidean/Cosine) and **PQ** are wired up
> today (see the status callouts above); the Binary mode is not.

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

*VelesDB Documentation -- Last updated: 2026-08-08 · Applies to: velesdb-core 6.0.0*
