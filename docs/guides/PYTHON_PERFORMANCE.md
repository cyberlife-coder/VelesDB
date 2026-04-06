# Python Performance Guide

A practical, actionable guide to getting the most throughput out of the VelesDB Python
bindings. Every recommendation here maps directly to a real method in the API —
nothing is hypothetical.

**Baseline:** A naive insert loop using Python lists yields ~9 000 vec/s.
**Achievable with the right patterns:** ~15 000–17 000 vec/s on typical hardware
(i9-class CPU, 384D, batch 1 000–5 000). The gap is almost entirely Python overhead,
not VelesDB's core engine.

---

## 1. Quick Wins (30 seconds to implement)

These three changes alone close most of the gap.

### Use `numpy` float32 instead of Python lists

VelesDB's core engine works with `f32` (32-bit float) internally. When you pass a
Python list of floats, PyO3 must convert each element from a Python `float` (which is
a C `double`, 64-bit) to `f32`. A `numpy` array with `dtype=float32` skips that
conversion entirely.

```python
import numpy as np

# Slow: Python list of floats (double -> f32 conversion for every element)
vector = [0.1, 0.2, 0.3, ...]

# Fast: numpy float32 (zero-element-wise conversion)
vector = np.array([0.1, 0.2, 0.3, ...], dtype=np.float32)
```

For 768-dimensional vectors this is a 768-element type conversion avoided per search
call — it compounds quickly when you issue thousands of queries per second.

### Use `upsert_bulk_numpy()` for batch inserts

`upsert_bulk_numpy()` uses a zero-copy path: the flat `f32` buffer from the numpy
array is passed directly to the core engine, eliminating per-row `Vec<f32>`
allocations. For 100 000 vectors at 768D this avoids ~293 MB of intermediate copies.

```python
import numpy as np
import velesdb

db = velesdb.Database("./my_db")
collection = db.create_collection("docs", dimension=384, metric="cosine")

# Generate or load your vectors as float32
vectors = np.random.randn(10_000, 384).astype(np.float32)
ids = np.arange(10_000, dtype=np.uint64)

# One call — no Python loop, GIL released during the core engine work
count = collection.upsert_bulk_numpy(vectors, ids)
print(f"Inserted {count} vectors")
```

The GIL is released during the actual insertion, so other Python threads can run
while VelesDB is writing to the HNSW index.

### Use optimal batch sizes: 1 000–5 000 points

VelesDB's batch insert path uses chunked phase-B HNSW construction that is most
efficient between 1 000 and 5 000 vectors per call. Smaller batches add per-call
overhead; larger batches do not improve throughput and increase peak memory usage.

```python
BATCH_SIZE = 2_000  # Sweet spot for most workloads

total = len(all_vectors)
ids = np.arange(total, dtype=np.uint64)

for start in range(0, total, BATCH_SIZE):
    end = min(start + BATCH_SIZE, total)
    collection.upsert_bulk_numpy(all_vectors[start:end], ids[start:end].tolist())
```

---

## 2. Search Optimization

### Choose the right `SearchQuality` profile

`search_with_quality()` selects the HNSW `ef_search` parameter. The default
(`balanced`) is conservative. For latency-sensitive serving, `fast` is often
sufficient. Use `accurate` or `perfect` only for offline evaluation.

| Profile | ef_search (top-10) | Recall | Typical latency |
|---------|-------------------|--------|-----------------|
| `"fast"` | 64 | ~92% | Lowest |
| `"balanced"` | 128 | ~99% | Default |
| `"accurate"` | 512 | ~100% | 4x slower than balanced |
| `"perfect"` | 4 096 | 100% | Exhaustive — evaluation only |
| `"autotune"` | Adaptive | ~95%+ | Scales with collection size |

```python
# For production serving where sub-millisecond latency matters
results = collection.search_with_quality(query_vector, "fast", top_k=10)

# For analytics or evaluation where recall is critical
results = collection.search_with_quality(query_vector, "accurate", top_k=10)

# For new collections where you don't know the right ef yet
results = collection.search_with_quality(query_vector, "autotune", top_k=10)
```

`autotune` adapts `ef_search` based on the collection's current size and dimension.
It is the recommended default when you cannot benchmark your specific workload first.

### Use `batch_search()` for multiple simultaneous queries

When you need results for several query vectors (e.g., expanding a user query into
multiple embeddings, or processing a queue), `batch_search()` is significantly more
efficient than calling `search()` in a loop: parsing overhead is paid once, and the
GIL is released for the entire batch.

```python
# Slow: one GIL release + one HNSW traversal per query
results = []
for query in query_vectors:
    results.append(collection.search(query.tolist(), top_k=10))

# Fast: one GIL release, queries dispatched together
searches = [{"vector": q.tolist(), "top_k": 10} for q in query_vectors]
results = collection.batch_search(searches)
# results[i] corresponds to query_vectors[i]
```

Each search dict supports `"vector"`, `"top_k"` (also accepted as `"topK"`), and
`"filter"`. Queries with different `top_k` values are automatically partitioned and
dispatched in separate groups.

### Pre-allocate numpy arrays for search batches

If you are building search batches in a tight loop, avoid constructing Python lists
that get immediately converted back to arrays inside VelesDB. Pre-allocate a numpy
buffer and slice into it:

```python
import numpy as np

BATCH = 32
DIM = 384

# Allocate once, reuse every iteration
query_buf = np.empty((BATCH, DIM), dtype=np.float32)

for batch_queries in incoming_stream:
    # Fill the pre-allocated buffer
    for i, q in enumerate(batch_queries):
        query_buf[i] = q

    searches = [{"vector": query_buf[i].tolist(), "top_k": 10} for i in range(len(batch_queries))]
    results = collection.batch_search(searches)
    process(results)
```

---

## 3. Threading

### When the GIL is released

VelesDB releases the GIL during all CPU-intensive operations. The GIL is held only
while PyO3 parses Python arguments (list/dict extraction) and while converting results
back to Python objects.

| Method | GIL held | GIL released |
|--------|----------|--------------|
| `upsert_bulk_numpy()` | Payload dict parsing | HNSW insert + storage write |
| `search()` | Vector extraction | HNSW traversal |
| `batch_search()` | All query parsing | All HNSW traversals |
| `search_with_quality()` | Vector extraction | HNSW traversal |
| `flush()` | Never | Entire fsync call |

This means `ThreadPoolExecutor` gives real parallelism for search-heavy workloads.

### `ThreadPoolExecutor` pattern for parallel search

```python
from concurrent.futures import ThreadPoolExecutor, as_completed
import numpy as np

collection = db.get_collection("docs")

def search_one(query_vector: np.ndarray) -> list:
    return collection.search(query_vector.tolist(), top_k=10)

query_vectors = np.random.randn(100, 384).astype(np.float32)

# GIL is released during each HNSW traversal, so threads genuinely run in parallel
with ThreadPoolExecutor(max_workers=4) as pool:
    futures = [pool.submit(search_one, q) for q in query_vectors]
    all_results = [f.result() for f in as_completed(futures)]
```

Limit `max_workers` to your CPU core count. More threads than cores adds context-switch
overhead without improving throughput.

For very high query rates, prefer `batch_search()` over many small `search()` calls
in a thread pool — a single batch call amortizes the argument-parsing overhead across
all queries while still releasing the GIL for the traversal.

---

## 4. Benchmarking Correctly

Getting accurate numbers requires a few disciplines.

### Warm-up runs

The first few calls to VelesDB after process start are slower: the HNSW graph may be
paged in from disk, Rust's allocator is cold, and the CPU branch predictor has not
learned the search path. Always discard the first 3–5 iterations:

```python
import time

# Warm up — results discarded
for _ in range(5):
    collection.search(query.tolist(), top_k=10)

# Measure
times = []
for _ in range(100):
    t0 = time.perf_counter_ns()
    collection.search(query.tolist(), top_k=10)
    times.append(time.perf_counter_ns() - t0)

median_us = sorted(times)[len(times) // 2] / 1_000
print(f"Median search latency: {median_us:.1f} µs")
```

### Exclude vector generation from timing

`np.random.randn()` and `.astype(np.float32)` are not free. Pre-generate all vectors
before starting the timer:

```python
import numpy as np
import time

N = 10_000
DIM = 384

# Generate vectors BEFORE starting the clock
vectors = np.random.randn(N, DIM).astype(np.float32)
ids = np.arange(N, dtype=np.uint64)

# Time only the insert
t0 = time.perf_counter()
collection.upsert_bulk_numpy(vectors, ids.tolist())
elapsed = time.perf_counter() - t0

print(f"Throughput: {N / elapsed:,.0f} vec/s")
```

### Use `time.perf_counter_ns()` for sub-millisecond measurements

`time.time()` has millisecond resolution on some platforms. `time.perf_counter_ns()`
returns nanoseconds and has the highest resolution available on the OS:

```python
import time

t0 = time.perf_counter_ns()
results = collection.search(query.tolist(), top_k=10)
elapsed_ns = time.perf_counter_ns() - t0
elapsed_us = elapsed_ns / 1_000

print(f"Search latency: {elapsed_us:.1f} µs")
```

### What to report

A single number is not meaningful. Report at minimum:

- `p50` (median): typical user experience
- `p95`: worst case for 19 out of 20 requests
- `p99`: near-worst case (often 2–5x p50 for HNSW)
- Throughput: total vectors / total wall time (not per-batch time)

```python
import numpy as np

latencies_ns = []
for _ in range(200):
    t0 = time.perf_counter_ns()
    collection.search(query.tolist(), top_k=10)
    latencies_ns.append(time.perf_counter_ns() - t0)

arr = np.array(latencies_ns) / 1_000  # convert to µs
print(f"p50: {np.percentile(arr, 50):.1f} µs")
print(f"p95: {np.percentile(arr, 95):.1f} µs")
print(f"p99: {np.percentile(arr, 99):.1f} µs")
```

---

## 5. Common Antipatterns

### Antipattern 1 — Python `list` vectors instead of `numpy`

```python
# Slow: per-element Python float -> f32 conversion at the FFI boundary
query = [0.1, 0.2, 0.3, ...]  # list of Python floats
results = collection.search(query, top_k=10)

# Fast: float32 array, no per-element conversion
query = np.array([0.1, 0.2, 0.3, ...], dtype=np.float32)
results = collection.search(query, top_k=10)  # accepts numpy arrays directly
```

### Antipattern 2 — `upsert()` or `upsert_bulk()` loop instead of `upsert_bulk_numpy()`

```python
# Slow: Python dict construction + per-element parsing for every point
for i, vec in enumerate(vectors):
    collection.upsert([{"id": i, "vector": vec.tolist()}])

# Also slow: upsert_bulk with dict conversion
points = [{"id": i, "vector": vectors[i].tolist()} for i in range(N)]
collection.upsert_bulk(points)

# Fast: zero-copy path, GIL released for the entire insert
ids = list(range(N))
collection.upsert_bulk_numpy(vectors, ids)
```

`upsert_bulk_numpy()` avoids:
1. The Python list comprehension constructing N dicts
2. Per-row `Vec<f32>` allocation inside Rust
3. The flat buffer copy that `upsert_bulk()` must perform

### Antipattern 3 — Repeated `search()` instead of `batch_search()`

```python
# Slow: N separate GIL release/acquire cycles, N separate Python calls
results = []
for query in query_batch:
    results.append(collection.search(query.tolist(), top_k=10))

# Fast: one GIL release, all traversals dispatched together
searches = [{"vector": q.tolist(), "top_k": 10} for q in query_batch]
results = collection.batch_search(searches)
```

The overhead difference grows linearly with batch size. For 32 queries, `batch_search`
eliminates 31 redundant GIL release/acquire and argument-parsing cycles.

### Antipattern 4 — Timing with `time.time()` for µs-level measurements

```python
# Unreliable: time.time() may have only millisecond resolution
import time
t0 = time.time()
collection.search(query.tolist(), top_k=10)
print(f"Latency: {(time.time() - t0) * 1e6:.1f} µs")  # may read 0 µs

# Correct: perf_counter_ns() has nanosecond resolution
t0 = time.perf_counter_ns()
collection.search(query.tolist(), top_k=10)
print(f"Latency: {(time.perf_counter_ns() - t0) / 1_000:.1f} µs")
```

### Antipattern 5 — Measuring only the first call

```python
# Unreliable: first call includes cold-start overhead (page faults, branch predictor)
t0 = time.perf_counter_ns()
results = collection.search(query.tolist(), top_k=10)
print(f"{(time.perf_counter_ns() - t0) / 1_000:.1f} µs")  # 3-10x the steady-state

# Correct: warm up, then measure over many iterations
for _ in range(5):
    collection.search(query.tolist(), top_k=10)

times = [
    time.perf_counter_ns()
    - (t0 := time.perf_counter_ns())
    or (collection.search(query.tolist(), top_k=10), time.perf_counter_ns() - t0)[1]
    for _ in range(100)
]
```

A cleaner version of the same pattern:

```python
for _ in range(5):  # warm up
    collection.search(query.tolist(), top_k=10)

times = []
for _ in range(100):  # measure
    t0 = time.perf_counter_ns()
    collection.search(query.tolist(), top_k=10)
    times.append(time.perf_counter_ns() - t0)
```

---

## Summary Table

| Change | Effort | Expected gain |
|--------|--------|---------------|
| `dtype=np.float32` on all vectors | 1 line | 5–15% on search latency |
| `upsert_bulk_numpy()` instead of dict loop | 3 lines | 1.5–3x insert throughput |
| Batch size 1 000–5 000 | 1 constant | Eliminates per-call overhead |
| `search_with_quality("fast")` vs default | 1 argument | 2x lower latency at ~92% recall |
| `batch_search()` for multiple queries | Refactor | Eliminates N-1 GIL cycles per batch |
| `ThreadPoolExecutor` for parallel search | ~10 lines | Near-linear scaling up to core count |

See also: [TUNING_GUIDE.md](TUNING_GUIDE.md) for HNSW parameter tuning and
[SEARCH_MODES.md](SEARCH_MODES.md) for the full `SearchQuality` reference.
