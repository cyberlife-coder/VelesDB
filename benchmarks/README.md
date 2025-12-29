# VelesDB Benchmark Kit

Comprehensive benchmark suite comparing VelesDB against other vector databases.

## Quick Start

```bash
# 1. Start pgvectorscale (Docker required)
docker-compose up -d

# 2. Install dependencies
pip install -r requirements.txt

# 3. Run benchmark
python benchmark.py
```

## Benchmarks Included

| Competitor | Algorithm | Notes |
|------------|-----------|-------|
| pgvectorscale | DiskANN | PostgreSQL extension by Timescale |
| pgvector | IVFFlat/HNSW | Base PostgreSQL vector extension |

## Test Configuration

| Parameter | Default | Description |
|-----------|---------|-------------|
| `N_VECTORS` | 10,000 | Number of vectors to insert |
| `DIM` | 768 | Vector dimension (BERT-sized) |
| `N_QUERIES` | 100 | Number of search queries |
| `TOP_K` | 10 | Results per query |
| `METRIC` | cosine | Distance metric |

## Methodology

### Data Generation
- Vectors are randomly generated and L2-normalized (simulating real embeddings)
- Same dataset used for all databases

### Insertion
- **pgvectorscale**: Batch INSERT with `::vector` cast, 1000 vectors/batch
- **VelesDB**: `upsert_bulk()` with parallel HNSW insertion

### Indexing
- **pgvectorscale**: `CREATE INDEX USING diskann (embedding vector_cosine_ops)`
- **VelesDB**: HNSW index built on-the-fly during insertion

### Search
- 10 warmup queries (excluded from timing)
- 100 timed queries measuring latency

## Metrics Collected

| Metric | Description |
|--------|-------------|
| Insert Time | Time to insert all vectors |
| Index Build Time | Time to create search index |
| Avg Latency | Mean search latency (ms) |
| P50 Latency | Median search latency (ms) |
| P95 Latency | 95th percentile latency (ms) |
| P99 Latency | 99th percentile latency (ms) |
| QPS | Queries per second throughput |

## Hardware Requirements

- **Minimum**: 8GB RAM, 4 CPU cores
- **Recommended**: 16GB RAM, 8+ CPU cores, NVMe SSD

## Results Format

Results are printed in a comparison table:

```
================================================================================
FINAL COMPARISON - VelesDB vs pgvectorscale
================================================================================
Dataset: 10000 vectors, 768 dimensions, 100 queries
--------------------------------------------------------------------------------
Metric                 | PGVectorScale    | VelesDB          | Speedup
--------------------------------------------------------------------------------
Total Ingest (s)       | 22.317           | 3.034            | 7.4x (VelesDB)
Avg Latency (ms)       | 52.78            | 4.05             | 13.0x (VelesDB)
P95 Latency (ms)       | 61.92            | 5.04             | 12.3x (VelesDB)
Throughput (QPS)       | 18.9             | 246.8            | 13.0x (VelesDB)
================================================================================
```

## Contributing Results

We welcome benchmark results from the community! Please include:

1. **Hardware specs**: CPU model, cores, RAM, storage type
2. **OS**: Windows/Linux/macOS + version
3. **Dataset size**: N_VECTORS, DIM
4. **Full output**: Copy the comparison table

Submit results via GitHub Issues or Discussions.

## License

Elastic License 2.0 (ELv2)
