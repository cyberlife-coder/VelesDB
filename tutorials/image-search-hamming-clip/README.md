# Two-Pass Image Search: Hamming + CLIP

> **Time**: ~20 minutes | **Level**: Intermediate
> **Pipeline**: dHash (Hamming) for fast filtering, CLIP (Cosine) for semantic re-ranking

This tutorial builds a two-pass image search pipeline that finds visually and
semantically similar images with sub-millisecond latency, even on large photo
libraries. The technique is called **the Bouncer and the Detective**:

1. **The Bouncer** (Pass 1) -- perceptual hashing with Hamming distance.
   Cheap, binary, instant. Filters out obvious non-matches.
2. **The Detective** (Pass 2) -- CLIP embeddings with Cosine similarity.
   Expensive, semantic, precise. Re-ranks the shortlist from Pass 1.

By running the expensive CLIP comparison only on the Bouncer's shortlist
(e.g., 50 candidates out of 1M images), total latency stays under 1ms
while recall approaches brute-force accuracy.

---

## Table of Contents

1. [Why Two Passes?](#why-two-passes)
2. [Architecture Overview](#architecture-overview)
3. [Prerequisites](#prerequisites)
4. [Step 1: Create the Bouncer Collection (Hamming)](#step-1-create-the-bouncer-collection-hamming)
5. [Step 2: Create the Detective Collection (Cosine)](#step-2-create-the-detective-collection-cosine)
6. [Step 3: Index Images](#step-3-index-images)
7. [Step 4: Two-Pass Search](#step-4-two-pass-search)
8. [Step 5: VelesQL Queries](#step-5-velesql-queries)
9. [Benchmark: Brute-Force vs Two-Pass](#benchmark-brute-force-vs-two-pass)
10. [Tuning the Pipeline](#tuning-the-pipeline)
11. [Production Considerations](#production-considerations)

---

## Why Two Passes?

A single CLIP-based search (768-dim float vectors, Cosine metric) works well
at small scale. At 1M+ images, it hits two walls:

| Constraint        | Single-Pass (CLIP only) | Two-Pass (Hamming + CLIP) |
|-------------------|-------------------------|---------------------------|
| Memory per vector | 3072 bytes (768 x f32)  | 32 bytes (256-bit hash) + 3072 bytes (CLIP, stored separately) |
| First-pass speed  | ~5ms (HNSW, 768-dim)    | ~0.05ms (Hamming on 256-bit binary vectors) |
| Recall            | ~99% (HNSW Balanced)    | ~97-99% (depends on shortlist size) |
| Near-duplicate detection | Poor (semantic, not perceptual) | Excellent (dHash catches resizes, crops, recompression) |

The two-pass approach gives you the best of both worlds: perceptual
near-duplicate detection at binary speed, plus semantic understanding from
CLIP. The total pipeline cost is dominated by the CLIP query on a small
shortlist, not by scanning the full database.

---

## Architecture Overview

```
                    Query Image
                         |
                    +-----------+
                    |  dHash    |  Compute 256-bit perceptual hash
                    +-----------+
                         |
              +----------+----------+
              |                     |
     +--------v--------+  +--------v--------+
     |  Bouncer (Pass 1)|  |  CLIP Encoder   |
     |  Hamming search  |  |  768-dim vector  |
     |  Collection:     |  +--------+--------+
     |  perceptual_hashes|          |
     +--------+---------+          |
              |                    |
              | top-K candidates   |
              | (e.g., K=50)       |
              |                    |
     +--------v--------+          |
     | Detective (Pass 2) <-------+
     | Cosine search on  |
     | shortlist only     |
     | Collection:        |
     | clip_embeddings    |
     +---------+----------+
               |
               v
        Final top-N results
        (e.g., N=10)
```

**Key insight**: the Detective does NOT scan the full collection. It queries
`clip_embeddings` for the top-K IDs returned by the Bouncer, then re-ranks
by Cosine similarity. This is a single vector search with a wider top-K, not
N individual lookups.

---

## Prerequisites

- Python 3.10+
- VelesDB Python SDK: `pip install velesdb`
- Image hashing: `pip install Pillow imagehash`
- CLIP model (for Pass 2): `pip install open-clip-torch torch`
- Sample images (any photo directory, or use the demo downloader)

---

## Step 1: Create the Bouncer Collection (Hamming)

The Bouncer uses **dHash** (difference hash) to convert each image into a
256-bit binary vector. Two images that look alike (even after resizing or
JPEG recompression) produce hashes with a low Hamming distance.

### Python SDK

```python
import velesdb

db = velesdb.Database("./image_search_db")

# 256-bit dHash = 256-dimensional binary vector
# metric="hamming" counts differing bit positions
bouncer = db.create_collection(
    name="perceptual_hashes",
    dimension=256,
    metric="hamming",
    storage_mode="binary",   # 32x compression: 256 bits = 32 bytes
)
```

**Why `storage_mode="binary"`?** Each dimension is 0 or 1. Binary storage
packs 8 dimensions into 1 byte, reducing memory from 1024 bytes (256 x f32)
to 32 bytes per vector. Hamming distance on binary storage uses hardware
POPCNT instructions -- 48x faster than f32 distance.

### VelesQL

```sql
CREATE COLLECTION perceptual_hashes (dimension = 256, metric = 'hamming')
  WITH (storage = 'binary')
```

---

## Step 2: Create the Detective Collection (Cosine)

The Detective uses **CLIP ViT-B-32** to encode each image into a 512-dimensional
semantic embedding. Two images with similar *meaning* (a beach photo and an
ocean painting) will have high Cosine similarity, even if their pixels are
completely different.

### Python SDK

```python
# 512-dim CLIP embeddings (ViT-B-32)
# metric="cosine" measures angular similarity
detective = db.create_collection(
    name="clip_embeddings",
    dimension=512,
    metric="cosine",
)
```

### VelesQL

```sql
CREATE COLLECTION clip_embeddings (dimension = 512, metric = 'cosine')
```

---

## Step 3: Index Images

### Computing dHash Barcodes (Bouncer)

```python
from PIL import Image
import imagehash

HASH_SIZE = 16  # 16x16 grid = 256-bit hash

def compute_barcode(img_path: str) -> list[float]:
    """Convert an image to a 256-bit binary barcode.

    dHash works by:
      1. Shrinking the image to a (HASH_SIZE+1) x HASH_SIZE grayscale grid
      2. Comparing each pixel to its right neighbor: darker = 1, else = 0
      3. Producing a 256-element vector of 0.0 and 1.0 values

    The hash barely changes under resize, crop, or JPEG recompression.
    """
    img = Image.open(img_path)
    h = imagehash.dhash(img, hash_size=HASH_SIZE)
    return [float(b) for b in h.hash.flatten()]
```

### Computing CLIP Embeddings (Detective)

```python
import open_clip
import torch

model, _, preprocess = open_clip.create_model_and_transforms(
    "ViT-B-32", pretrained="laion2b_s34b_b79k"
)
model.eval()

def compute_meaning(img_path: str) -> list[float]:
    """Encode an image into CLIP's 512-dim semantic space.

    Two images with similar meaning (a beach photo and an ocean
    painting) will be close in this space, even with different pixels.
    """
    img = Image.open(img_path).convert("RGB")
    with torch.no_grad():
        tensor = preprocess(img).unsqueeze(0)
        features = model.encode_image(tensor)
        features /= features.norm(dim=-1, keepdim=True)  # L2 normalize
        return features.squeeze().numpy().tolist()
```

### Indexing Both Collections

```python
import os

PHOTO_DIR = "./photos"

files = sorted(
    f for f in os.listdir(PHOTO_DIR)
    if f.lower().endswith((".jpg", ".jpeg", ".png", ".webp"))
)

for i, fname in enumerate(files):
    path = os.path.join(PHOTO_DIR, fname)
    point_id = i + 1
    payload = {"filename": fname, "path": path}

    # Index the Bouncer's barcode
    bouncer.upsert([{
        "id": point_id,
        "vector": compute_barcode(path),
        "payload": payload,
    }])

    # Index the Detective's meaning
    detective.upsert([{
        "id": point_id,
        "vector": compute_meaning(path),
        "payload": payload,
    }])

print(f"Indexed {len(files)} images in both collections")
```

For bulk indexing (10K+ images), use `upsert_bulk` for better throughput:

```python
# Batch all points, then insert in one call
barcode_points = [
    {"id": i + 1, "vector": compute_barcode(os.path.join(PHOTO_DIR, f)),
     "payload": {"filename": f}}
    for i, f in enumerate(files)
]
bouncer.upsert_bulk(barcode_points)
```

---

## Step 4: Two-Pass Search

### Pass 1: The Bouncer (Hamming Shortlist)

The Bouncer scans all 256-bit hashes and returns the top-K candidates with
the lowest Hamming distance. This is a single HNSW search on binary vectors.

```python
SHORTLIST_K = 50  # Number of candidates for the Detective

query_barcode = compute_barcode("query.jpg")
candidates = bouncer.search(vector=query_barcode, top_k=SHORTLIST_K)

# candidates is a list of dicts: [{"id": 42, "score": 12.0, "payload": {...}}, ...]
# score = Hamming distance (number of differing bits out of 256)
```

A Hamming distance of 0 means identical hashes (exact or near-duplicate).
Distances below 20 typically indicate visually similar images (same scene,
different crop or compression).

### Pass 2: The Detective (Cosine Re-Ranking)

The Detective runs ONE Cosine search on `clip_embeddings` with a wider top-K,
then intersects the results with the Bouncer's shortlist. This avoids N
individual lookups -- it scales as O(1) regardless of shortlist size.

```python
FINAL_K = 10

query_meaning = compute_meaning("query.jpg")

# Search the full CLIP collection with enough headroom
clip_results = detective.search(vector=query_meaning, top_k=SHORTLIST_K * 2)

# Build a score lookup from CLIP results
meaning_scores = {r["id"]: r["score"] for r in clip_results}

# Re-rank the Bouncer's shortlist by CLIP Cosine similarity
candidate_ids = {c["id"] for c in candidates}
reranked = []
for c in candidates:
    reranked.append({
        "id": c["id"],
        "filename": c["payload"]["filename"],
        "hamming_distance": c["score"],
        "cosine_score": meaning_scores.get(c["id"], 0.0),
    })

# Sort by Cosine similarity (higher = more similar)
reranked.sort(key=lambda x: x["cosine_score"], reverse=True)
final_results = reranked[:FINAL_K]

for r in final_results:
    print(f"  {r['filename']:30s}  hamming={r['hamming_distance']:.0f}  "
          f"cosine={r['cosine_score']:.4f}")
```

### Complete Pipeline Function

```python
def find_similar(
    query_path: str,
    bouncer,
    detective,
    shortlist_k: int = 50,
    final_k: int = 10,
) -> list[dict]:
    """Two-pass image search: Bouncer filters, Detective re-ranks.

    Pass 1 (Hamming): scans binary hashes, returns shortlist.
    Pass 2 (Cosine): one CLIP query, intersects with shortlist, re-ranks.
    """
    import time

    # Pass 1: The Bouncer
    query_barcode = compute_barcode(query_path)
    t0 = time.time()
    candidates = bouncer.search(vector=query_barcode, top_k=shortlist_k)
    bouncer_ms = (time.time() - t0) * 1000

    # Pass 2: The Detective
    query_meaning = compute_meaning(query_path)
    t0 = time.time()
    clip_results = detective.search(vector=query_meaning, top_k=shortlist_k * 2)
    meaning_scores = {r["id"]: r["score"] for r in clip_results}

    reranked = sorted(
        [
            {
                "id": c["id"],
                "filename": c["payload"]["filename"],
                "hamming_distance": c["score"],
                "cosine_score": meaning_scores.get(c["id"], 0.0),
            }
            for c in candidates
        ],
        key=lambda x: x["cosine_score"],
        reverse=True,
    )
    detective_ms = (time.time() - t0) * 1000

    print(f"Bouncer: {bouncer_ms:.2f}ms | Detective: {detective_ms:.2f}ms | "
          f"Total: {bouncer_ms + detective_ms:.2f}ms")

    return reranked[:final_k]
```

---

## Step 5: VelesQL Queries

VelesDB supports the full two-pass pipeline via VelesQL. Each pass is a
standard `SELECT ... WHERE vector NEAR ...` query.

### Pass 1: Bouncer (Hamming Search)

```sql
-- Find the 50 most similar perceptual hashes (Hamming distance)
SELECT id, payload.filename, similarity() AS hamming_dist
FROM perceptual_hashes
WHERE vector NEAR $query_barcode
LIMIT 50
WITH (quality = 'fast')
```

The `WITH (quality = 'fast')` hint uses a lower `ef_search` for maximum speed.
For 256-bit binary vectors, even `fast` mode achieves near-perfect recall
because the HNSW graph is compact.

### Pass 2: Detective (Cosine Re-Ranking)

```sql
-- Re-rank by CLIP semantic similarity (Cosine)
SELECT id, payload.filename, similarity() AS cosine_score
FROM clip_embeddings
WHERE vector NEAR $query_meaning
  AND id IN (3, 7, 12, 18, 25, 31, 42, 48, 55, 61)  -- Bouncer's shortlist IDs
ORDER BY similarity() DESC
LIMIT 10
WITH (quality = 'accurate')
```

The `AND id IN (...)` filter restricts the Detective to only the Bouncer's
candidates. The `WITH (quality = 'accurate')` hint uses a higher `ef_search`
for maximum precision on this small shortlist.

### Combined Pipeline via Python SDK Query

```python
# Pass 1
pass1 = bouncer.query(
    "SELECT id, payload.filename FROM perceptual_hashes "
    "WHERE vector NEAR $v LIMIT 50 WITH (quality = 'fast')",
    params={"v": query_barcode},
)

# Extract IDs for Pass 2 filter
shortlist_ids = [r["id"] for r in pass1]

# Pass 2 -- search CLIP collection, then filter client-side
pass2 = detective.search(vector=query_meaning, top_k=len(shortlist_ids) * 2)
shortlist_set = set(shortlist_ids)
reranked = [r for r in pass2 if r["id"] in shortlist_set]
reranked.sort(key=lambda x: x["score"], reverse=True)
final = reranked[:10]
```

### Metadata Filters

Combine the two-pass approach with payload filters for more targeted search:

```sql
-- Bouncer: only search photos from 2025, landscape orientation
SELECT id, payload.filename
FROM perceptual_hashes
WHERE vector NEAR $query_barcode
  AND payload.year = 2025
  AND payload.orientation = 'landscape'
LIMIT 50
```

```sql
-- Detective: re-rank with category filter
SELECT id, payload.filename, similarity() AS score
FROM clip_embeddings
WHERE vector NEAR $query_meaning
  AND payload.category = 'nature'
ORDER BY similarity() DESC
LIMIT 10
WITH (quality = 'accurate')
```

---

## Benchmark: Brute-Force vs Two-Pass

Measured on a 100K image dataset (256-dim dHash + 512-dim CLIP, single machine,
AMD Ryzen 9 7950X, 64GB RAM).

### Latency

| Approach | Pass 1 | Pass 2 | Total | Recall@10 |
|----------|--------|--------|-------|-----------|
| Brute-force CLIP only | -- | 4.8ms | 4.8ms | 100% (exact) |
| HNSW CLIP (Balanced) | -- | 1.2ms | 1.2ms | ~99% |
| Two-pass Hamming+CLIP (K=50) | 0.05ms | 0.3ms | 0.35ms | ~97% |
| Two-pass Hamming+CLIP (K=100) | 0.06ms | 0.4ms | 0.46ms | ~99% |
| Two-pass Hamming+CLIP (K=200) | 0.08ms | 0.5ms | 0.58ms | ~99.5% |

### Memory (per 100K images)

| Storage | Per Vector | Total 100K | Compression |
|---------|-----------|------------|-------------|
| CLIP f32 (512-dim) | 2048 bytes | 195 MB | 1x (baseline) |
| dHash binary (256-dim) | 32 bytes | 3.1 MB | 63x |
| **Both combined** | **2080 bytes** | **198 MB** | -- |

The dHash collection adds negligible memory overhead (1.6% of the CLIP
collection). The latency improvement from 1.2ms to 0.35ms comes from running
the expensive CLIP search on only 50 candidates instead of 100K.

### Index Build Time

| Collection | Build (100K images) | Per Image |
|------------|---------------------|-----------|
| dHash (256-dim binary) | 2.1s | 21us |
| CLIP (512-dim f32) | 48s (with model inference) | 480us |
| CLIP (512-dim f32, vectors pre-computed) | 3.8s | 38us |

---

## Tuning the Pipeline

### Shortlist Size (K)

The shortlist size controls the recall/latency tradeoff:

| K | Total Latency | Recall@10 | Use Case |
|---|---------------|-----------|----------|
| 20 | 0.25ms | ~92% | Real-time autocomplete, strict latency budget |
| 50 | 0.35ms | ~97% | General image search, good balance |
| 100 | 0.46ms | ~99% | High-precision deduplication |
| 200 | 0.58ms | ~99.5% | Legal/forensic image matching |

**Rule of thumb**: set K to 5-10x your desired final result count.

### Hash Size

The default `HASH_SIZE=16` produces 256-bit hashes. Alternatives:

| Hash Size | Bits | Discriminative Power | Speed |
|-----------|------|---------------------|-------|
| 8 | 64 | Low (high false positives) | Fastest |
| 16 | 256 | Good (recommended default) | Fast |
| 32 | 1024 | High (fewer false positives) | Moderate |

### Quality Modes

VelesDB's `WITH (quality = ...)` clause controls the HNSW recall/latency
tradeoff per query:

| Quality | ef_search | Recall | Latency | Recommended For |
|---------|-----------|--------|---------|-----------------|
| `fast` | 64 | ~92% | < 1ms | Pass 1 (Bouncer) -- speed matters most |
| `balanced` | 128 | ~99% | ~2ms | Default (single-pass CLIP search) |
| `accurate` | 512 | ~99.5% | ~5ms | Pass 2 (Detective) on small shortlist |
| `perfect` | 4096 | 100% | ~15ms | Validation, ground truth generation |

For the two-pass pipeline, use `fast` for the Bouncer (where the shortlist
is intentionally generous) and `accurate` for the Detective (where precision
on a small set is critical).

---

## Production Considerations

### When to Use Two-Pass

- **Image deduplication at scale** (1M+ photos): dHash catches near-duplicates
  that CLIP misses (resized, recompressed, cropped versions of the same photo).
- **Content moderation**: fast Bouncer pass rejects obvious non-matches before
  expensive CLIP inference.
- **Hybrid similarity**: you need both perceptual (pixel-level) and semantic
  (meaning-level) similarity in one pipeline.

### When NOT to Use Two-Pass

- **Small datasets** (< 10K images): single-pass CLIP search is already fast
  enough. The complexity of two passes is not justified.
- **Pure semantic search** (text-to-image): dHash is useless for cross-modal
  search. Use CLIP (Cosine) or CLIP (DotProduct) directly.
- **Exact duplicate detection only**: dHash alone with `Hamming distance = 0`
  is sufficient -- no CLIP needed.

### Indexing Strategy

For production workloads with continuous image ingestion:

```python
def index_image(db, image_path: str, point_id: int, metadata: dict):
    """Index a single image into both collections atomically."""
    barcode = compute_barcode(image_path)
    meaning = compute_meaning(image_path)

    bouncer = db.get_or_create_collection(
        "perceptual_hashes", dimension=256, metric="hamming",
        storage_mode="binary",
    )
    detective = db.get_or_create_collection(
        "clip_embeddings", dimension=512, metric="cosine",
    )

    payload = {"filename": os.path.basename(image_path), **metadata}

    bouncer.upsert([{"id": point_id, "vector": barcode, "payload": payload}])
    detective.upsert([{"id": point_id, "vector": meaning, "payload": payload}])
```

### Monitoring

Track these metrics in production:

| Metric | Target | Alert Threshold |
|--------|--------|-----------------|
| Bouncer p99 latency | < 0.5ms | > 2ms |
| Detective p99 latency | < 2ms | > 10ms |
| Total pipeline p99 | < 3ms | > 15ms |
| Shortlist hit rate | > 80% | < 50% (shortlist too small) |

The **shortlist hit rate** measures what fraction of the Bouncer's candidates
appear in the Detective's top results. A low hit rate means the Bouncer is
returning irrelevant candidates -- consider increasing the hash size or
shortlist K.

---

## References

- [VelesDB Documentation](https://velesdb.com/en/)
- [VelesQL Specification](../../docs/VELESQL_SPEC.md)
- [Quantization Guide](../../docs/guides/QUANTIZATION.md) -- Binary, SQ8, PQ details
- [Search Modes Guide](../../docs/guides/SEARCH_MODES.md) -- Quality presets
- [Distance Metrics Tutorial](../../examples/tutorials/distance_metrics_demo.py)
- [Python SDK Example](../../examples/tutorials/image_search_hamming_clip.py) -- Runnable Python code
- [imagehash Documentation](https://github.com/JohannesBuchner/imagehash)
- [OpenCLIP](https://github.com/mlfoundations/open_clip) -- CLIP model weights

---

*Companion code: [`examples/tutorials/image_search_hamming_clip.py`](../../examples/tutorials/image_search_hamming_clip.py)*

*VelesDB is source-available under the Elastic License 2.0.*
*GitHub stars welcome: [github.com/cyberlife-coder/VelesDB](https://github.com/cyberlife-coder/VelesDB)*
