# VelesDB Docker Benchmark Results

*Date: January 2, 2026*
*Version: 0.7.3*

## Test Environment

| Component | Configuration |
|-----------|---------------|
| **VelesDB** | Docker (optimized: LTO + target-cpu=native) |
| **pgvector** | Docker (pgvector/pgvector:pg16) |
| **Memory limit** | 4GB per container |
| **Vector dimension** | 768D (typical for embedding models) |
| **Distance metric** | Cosine similarity |
| **Data distribution** | 50 clusters (realistic) |

---

## Results: 100K Vectors (Final)

**HNSW Parameters**: M=64, ef_construction=800 (tuned for 768D, ≥95% recall)

| Mode | ef_search | Recall@10 | Latency P50 | Latency P99 |
|------|-----------|-----------|-------------|-------------|
| Fast | 64 | 34.2% | 59.3ms | 64.3ms |
| Balanced | 128 | 48.8% | 60.9ms | 71.5ms |
| Accurate | 256 | 67.6% | 78.3ms | 88.8ms |
| HighRecall | 1024 | **96.1%** ✅ | 73.0ms | 90.6ms |
| Perfect | 2048 | **100.0%** | 42.1ms | 92.1ms |

### ✅ Issue #28 Fixed - Target ≥95% Achieved

HNSW parameters optimized for high recall at scale:
- **M**: 16 → 64 for 768D vectors
- **ef_construction**: 200 → 800 for 768D vectors
- **ef_search (HighRecall)**: 512 → 1024

Recall improvement at 100K vectors:
| Mode | Initial | Final | Improvement |
|------|---------|-------|-------------|
| HighRecall | 47.7% | **96.1%** | **+48.4%** |

---

## Adaptive Parameters for Scale (NEW)

Use `HnswParams::for_dataset_size(dimension, expected_vectors)` for optimal recall:

| Dataset Size | M | ef_construction | Target Recall |
|--------------|---|-----------------|---------------|
| ≤10K | 32 | 400 | ≥98% |
| ≤100K | 64 | 800 | ≥95% |
| ≤500K | 96 | 1200 | ≥95% |
| **≤1M** | **128** | **1600** | **≥95%** |

```rust
// For 1M vectors at 768D
let params = HnswParams::million_scale(768);
// M=128, ef_construction=1600
```

---

## Insert Performance

| Dataset | VelesDB | pgvector | Speedup |
|---------|---------|----------|---------|
| 10K vectors | ~5s | ~19s | **3.8x** |
| 100K vectors | ~52s | ~365s | **7x** |

VelesDB inserts are significantly faster because:
1. No separate index build phase
2. Incremental HNSW construction
3. Batch insert API

---

## Key Findings

### Network Latency Dominates
- Both VelesDB and pgvector show ~50ms latency via Docker/network
- This masks VelesDB's true advantage (~105µs embedded)

### Perfect Mode Works
- 100% recall guaranteed via brute-force SIMD
- Latency: 50-80ms (acceptable for many use cases)

### HNSW Parameters Need Tuning
- Current defaults optimized for small datasets (<10K)
- Larger datasets need higher `M` and `ef_construction`

---

## Recommendations

| Dataset Size | Recommended Mode | Expected Recall |
|--------------|------------------|-----------------|
| < 10K | Balanced (ef=128) | 60-90% |
| 10K - 50K | Accurate (ef=256) | 80-95% |
| 50K - 100K | HighRecall (ef=512) | 90-98% |
| > 100K | Perfect (ef=2048) or tune HNSW params | 100% |

---

## How to Reproduce

```bash
cd benchmarks
docker-compose up -d
python benchmark_all_modes.py --vectors 10000 --dim 768 --queries 50
```

## Files

- `benchmark_all_modes.py` - Multi-mode benchmark script
- `benchmark_docker.py` - VelesDB vs pgvector comparison
- `docker-compose.yml` - Container orchestration
- `Dockerfile.optimized` - VelesDB optimized build
