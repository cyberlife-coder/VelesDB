# VelesDB Benchmark Kit

Benchmark suite comparing VelesDB against pgvector (HNSW).

## 🚀 v0.7.3 Results: VelesDB Recall ≥95% Guaranteed

### Search Performance (100K vectors, 768D, Docker)

| Mode | ef_search | Recall@10 | Latency P50 |
|------|-----------|-----------|-------------|
| Fast | 64 | 34.2% | 59.3ms |
| Balanced | 128 | 48.8% | 60.9ms |
| Accurate | 256 | 67.6% | 78.3ms |
| **HighRecall** | **1024** | **96.1%** ✅ | 73.0ms |
| **Perfect** | **2048** | **100.0%** | 42.1ms |

### Insert Performance (100K vectors, 768D)

| Dataset | VelesDB | pgvector | Speedup |
|---------|---------|----------|--------|
| 10K | ~5s | ~19s | **3.8x** |
| 100K | ~52s | ~365s | **7x** |

### Key Optimizations in v0.7.x

- **SIMD AVX-512/AVX2** - 32-wide processing with FMA
- **Adaptive HNSW params** - `HnswParams::for_dataset_size()` for optimal recall
- **Parallel search** - Batch parallel with prefetching
- **Quantization** - SQ8 (4x) and Binary (32x) compression

## Benchmark Modes

### 1. Docker vs Docker (Fair comparison)

```bash
docker-compose up -d --build  # Start both servers
python benchmark_docker.py --vectors 5000 --clusters 25
```

| Database | Mode | What it measures |
|----------|------|------------------|
| VelesDB | REST API (Docker) | Client-server via HTTP |
| pgvector | Docker + PostgreSQL | Client-server via SQL |

### 2. Native vs Docker (Embedded advantage)

```bash
python benchmark_recall.py --vectors 10000
```

| Database | Mode | What it measures |
|----------|------|------------------|
| VelesDB | Native Python (PyO3) | Best-case embedded performance |
| pgvector | Docker + PostgreSQL | Client-server with SQL overhead |

## Quick Start

```bash
# 1. Start both servers (Docker required)
docker-compose up -d --build

# 2. Install dependencies
pip install -r requirements.txt

# 3. Run fair Docker benchmark
python benchmark_docker.py --vectors 5000 --clusters 25
```

## Options

```bash
# Both scripts support:
--vectors 5000     # Dataset size
--dim 768          # Vector dimension  
--queries 100      # Number of queries
--clusters 25      # Data clusters (realistic)

# Docker benchmark only:
--velesdb-url http://localhost:8080
```

## Methodology

### Fair Comparison

Both databases are measured with **total time including index construction**:

- **VelesDB**: Insert + inline HNSW indexing
- **pgvector**: Raw INSERT + separate CREATE INDEX time

This ensures an apples-to-apples comparison of the complete ingestion pipeline.

### HNSW Parameters (Adaptive)

VelesDB uses adaptive parameters based on dataset size:

| Dataset Size | M | ef_construction | Target Recall |
|--------------|---|-----------------|---------------|
| ≤10K | 32 | 400 | ≥98% |
| ≤100K | 64 | 800 | ≥95% |
| ≤500K | 96 | 1200 | ≥95% |
| **≤1M** | **128** | **1600** | **≥95%** |

```rust
// Automatic parameter selection
let params = HnswParams::for_dataset_size(768, 100_000);
// Or for 1M scale
let params = HnswParams::million_scale(768);
```

## When to Choose Each

| Use Case | Recommendation |
|----------|----------------|
| Bulk import speed | **VelesDB** ✅ (3.2x faster) |
| Embedded/Desktop apps | **VelesDB** ✅ |
| Real-time (<10ms) | **VelesDB** ✅ |
| Edge/IoT/WASM | **VelesDB** ✅ |
| Existing PostgreSQL | **pgvector** ✅ |
| SQL ecosystem | **pgvector** ✅ |

## License

Elastic License 2.0 (ELv2)
