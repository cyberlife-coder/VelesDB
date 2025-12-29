# VelesDB Benchmark Kit

Benchmark suite comparing VelesDB against pgvector (HNSW).

## 🚀 v0.5.0 Results: VelesDB 3.2x Faster Than pgvector

### Insertion Performance (5,000 vectors, 768D, Docker)

| Metric | pgvector | VelesDB | Result |
|--------|----------|---------|--------|
| **Insert + Index** | 8.54s | **2.63s** | **3.2x faster** |
| **Recall@10** | 100.0% | 99.7% | Comparable |
| **Search P50** | 3.0ms | 4.0ms | Comparable |

### Key Optimizations in v0.5.0

- **SIMD-accelerated HNSW** - AVX2/SSE distance calculations via `simdeez_f`
- **Parallel insertion** - Native Rayon-based graph construction
- **Deferred index save** - No disk I/O during batch operations
- **Async-safe server** - `spawn_blocking` for bulk operations

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

### HNSW Parameters

Both databases use equivalent parameters:

| Parameter | VelesDB | pgvector |
|-----------|---------|----------|
| M (connections) | 16 | 16 |
| ef_construction | 200 | 200 |

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
