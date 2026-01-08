# VelesDB Search Modes

> Understanding search quality presets and the recall/latency tradeoff.

## Overview

VelesDB provides pre-configured search modes that balance **recall** (accuracy) and **latency** (speed). The `ef_search` parameter controls the size of the candidate pool during HNSW graph traversal.

## Search Modes

| Mode | ef_search | Expected Recall | Latency | Use Case |
|------|-----------|-----------------|---------|----------|
| **Fast** | 64 | ~90% | Lowest | Real-time autocomplete, high QPS |
| **Balanced** | 128 | ~98% | Low | Default, most applications |
| **Accurate** | 256 | ~99% | Medium | Production RAG, semantic search |
| **HighRecall** | 1024 | ~99.7% | Higher | Critical applications |
| **Perfect** | ∞ (bruteforce) | 100% | Highest | Benchmarking, small datasets |

## Mode Details

### Fast Mode

```sql
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (mode = 'fast');
```

- **ef_search**: 64 (minimum: k × 2)
- **Best for**: High-throughput scenarios where slight accuracy loss is acceptable
- **Typical latency**: <100µs for 10K vectors

### Balanced Mode (Default)

```sql
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10;
-- or explicitly:
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (mode = 'balanced');
```

- **ef_search**: 128 (minimum: k × 4)
- **Best for**: General-purpose semantic search, chatbots, RAG
- **Typical latency**: 100-200µs for 10K vectors

### Accurate Mode

```sql
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (mode = 'accurate');
```

- **ef_search**: 256 (minimum: k × 8)
- **Best for**: Production applications requiring high accuracy
- **Typical latency**: 200-400µs for 10K vectors

### High Recall Mode

```sql
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (mode = 'high_recall');
```

- **ef_search**: 1024 (minimum: k × 32)
- **Best for**: Critical applications, legal/medical search
- **Typical latency**: 500µs-1ms for 10K vectors

### Perfect Mode

```sql
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (mode = 'perfect');
```

- **ef_search**: MAX (bruteforce with SIMD)
- **Best for**: Benchmarking, ground truth, small datasets (<50K vectors)
- **Typical latency**: 1-10ms for 10K vectors

## Custom ef_search

Override the mode's default ef_search:

```sql
-- Use balanced mode but with custom ef_search
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (ef_search = 512);
```

Valid range: 16 to 4096

## Adaptive ef_search

VelesDB automatically adjusts ef_search based on `k` (number of results requested):

```
effective_ef_search = max(mode_ef_search, k × multiplier)
```

| Mode | Multiplier |
|------|------------|
| Fast | 2 |
| Balanced | 4 |
| Accurate | 8 |
| HighRecall | 32 |
| Perfect | 50 |

**Example**: Requesting k=100 in Balanced mode:
```
ef_search = max(128, 100 × 4) = 400
```

## Configuration

### In TOML config

```toml
[search]
default_mode = "balanced"
ef_search = 256  # Optional override
```

### In REPL session

```bash
velesdb> \set mode accurate
velesdb> \set ef_search 512
velesdb> \show
  mode = accurate
  ef_search = 512
```

### Via REST API

```bash
curl -X POST http://localhost:8080/collections/docs/search \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.1, 0.2, ...],
    "top_k": 10,
    "params": {
      "mode": "accurate",
      "ef_search": 512
    }
  }'
```

## Performance Characteristics

### Recall vs ef_search (10K vectors, 768 dims)

```
ef_search=32   → ~85% recall, ~50µs
ef_search=64   → ~90% recall, ~80µs
ef_search=128  → ~98% recall, ~120µs
ef_search=256  → ~99% recall, ~200µs
ef_search=512  → ~99.5% recall, ~350µs
ef_search=1024 → ~99.7% recall, ~600µs
ef_search=2048 → ~99.9% recall, ~1.1ms
```

### Scaling with Dataset Size

Latency grows approximately as O(log N) for HNSW:

| Vectors | Balanced Mode Latency |
|---------|----------------------|
| 1K | ~50µs |
| 10K | ~120µs |
| 100K | ~200µs |
| 1M | ~350µs |
| 10M | ~600µs |

## Recommendations

| Scenario | Recommended Mode |
|----------|-----------------|
| Autocomplete, typeahead | Fast |
| General semantic search | Balanced |
| Production RAG pipelines | Accurate |
| Legal/medical/financial | HighRecall |
| Benchmarking, testing | Perfect |
| <1000 vectors | Perfect (low overhead) |

## Reranking

For quantized storage modes (SQ8, Binary), VelesDB automatically applies exact reranking:

```sql
-- Disable reranking (faster but lower accuracy for quantized)
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (rerank = false);
```

## License

ELv2 (Elastic License 2.0)
