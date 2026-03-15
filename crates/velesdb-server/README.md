# VelesDB Server

[![Crates.io](https://img.shields.io/crates/v/velesdb-server.svg)](https://crates.io/crates/velesdb-server)
[![License](https://img.shields.io/badge/license-VelesDB_Core_1.0-blue)](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE)

REST API server for VelesDB - a high-performance vector database.

## Installation

### From crates.io

```bash
cargo install velesdb-server
```

### Docker

```bash
docker run -p 8080:8080 -v ./data:/data ghcr.io/cyberlife-coder/velesdb:latest
```

### From source

```bash
git clone https://github.com/cyberlife-coder/VelesDB
cd VelesDB
cargo build --release -p velesdb-server
```

## Usage

```bash
# Start server on default port 8080
velesdb-server

# Custom port and data directory
velesdb-server --port 9000 --data ./my_vectors

# With logging
RUST_LOG=info velesdb-server
```

## API Reference

### Collections

```bash
# Create collection (default: full precision, cosine)
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "documents", "dimension": 768, "metric": "cosine"}'

# Create collection with quantization (SQ8 = 4x memory reduction)
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "compressed", "dimension": 768, "metric": "cosine", "storage_mode": "sq8"}'

# Create binary collection (Hamming + Binary = 32x compression)
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "fingerprints", "dimension": 256, "metric": "hamming", "storage_mode": "binary"}'

# List collections
curl http://localhost:8080/collections

# Get collection info
curl http://localhost:8080/collections/documents

# Delete collection
curl -X DELETE http://localhost:8080/collections/documents
```

### Points (Vectors)

```bash
# Upsert points
curl -X POST http://localhost:8080/collections/documents/points \
  -H "Content-Type: application/json" \
  -d '{
    "points": [
      {"id": 1, "vector": [0.1, 0.2, ...], "payload": {"title": "Hello"}}
    ]
  }'

# Get a point by ID
curl http://localhost:8080/collections/documents/points/1

# Delete a point by ID
curl -X DELETE http://localhost:8080/collections/documents/points/1
```

### Search

```bash
# Vector similarity search
curl -X POST http://localhost:8080/collections/documents/search \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.15, 0.25, ...],
    "top_k": 5,
    "filter": {"category": {"$eq": "tech"}}
  }'

# BM25 full-text search
curl -X POST http://localhost:8080/collections/documents/search/text \
  -H "Content-Type: application/json" \
  -d '{"query": "rust programming", "top_k": 10}'

# Hybrid search (vector + text)
curl -X POST http://localhost:8080/collections/documents/search/hybrid \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.15, 0.25, ...],
    "query": "rust programming",
    "top_k": 10,
    "vector_weight": 0.7
  }'

# Batch search (multiple queries in parallel)
curl -X POST http://localhost:8080/collections/documents/search/batch \
  -H "Content-Type: application/json" \
  -d '{
    "searches": [
      {"vector": [0.1, 0.2, ...], "top_k": 5},
      {"vector": [0.3, 0.4, ...], "top_k": 5}
    ]
  }'

# Multi-query fusion search (MQG for RAG)
curl -X POST http://localhost:8080/collections/documents/search/multi \
  -H "Content-Type: application/json" \
  -d '{
    "vectors": [[0.1, 0.2, ...], [0.3, 0.4, ...], [0.5, 0.6, ...]],
    "top_k": 10,
    "fusion": "rrf",
    "fusion_params": {"k": 60}
  }'

# Weighted fusion strategy
curl -X POST http://localhost:8080/collections/documents/search/multi \
  -H "Content-Type: application/json" \
  -d '{
    "vectors": [[...], [...], [...]],
    "top_k": 10,
    "fusion": "weighted",
    "fusion_params": {"avgWeight": 0.6, "maxWeight": 0.3, "hitWeight": 0.1}
  }'

# VelesQL query
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT * FROM documents WHERE VECTOR NEAR $v LIMIT 5",
    "params": {"v": [0.15, 0.25, ...]}
  }'

# VelesQL with MATCH (full-text)
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT * FROM documents WHERE content MATCH '\''rust'\'' LIMIT 10",
    "params": {}
  }'

# Aggregation-only VelesQL endpoint
curl -X POST http://localhost:8080/aggregate \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT category, COUNT(*) FROM documents GROUP BY category",
    "params": {}
  }'

# Explain query plan
curl -X POST http://localhost:8080/query/explain \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT * FROM documents WHERE VECTOR NEAR $v LIMIT 5",
    "params": {"v": [0.15, 0.25, 0.35, 0.45]}
  }'
```

### Graph API

```bash
# List edges filtered by label (label is required)
curl http://localhost:8080/collections/documents/graph/edges?label=related

# Add an edge (id, source, target, label are required)
curl -X POST http://localhost:8080/collections/documents/graph/edges \
  -H "Content-Type: application/json" \
  -d '{"id": 1, "source": 1, "target": 2, "label": "related"}'

# Traverse graph from a node (source is required)
curl -X POST http://localhost:8080/collections/documents/graph/traverse \
  -H "Content-Type: application/json" \
  -d '{"source": 1, "max_depth": 3}'

# Stream graph traversal (start_node is required)
curl "http://localhost:8080/collections/documents/graph/traverse/stream?start_node=1"

# Get node degree
curl http://localhost:8080/collections/documents/graph/nodes/1/degree
```

### Index API

```bash
# List indexes on a collection
curl http://localhost:8080/collections/documents/indexes

# Create an index
curl -X POST http://localhost:8080/collections/documents/indexes \
  -H "Content-Type: application/json" \
  -d '{"label": "category", "property": "name"}'

# Delete an index
curl -X DELETE http://localhost:8080/collections/documents/indexes/category/name
```

### Health & OpenAPI

```bash
# Health check
curl http://localhost:8080/health

# OpenAPI spec and Swagger UI (requires --features swagger-ui at build time)
curl http://localhost:8080/api-docs/openapi.json
# Open in browser: http://localhost:8080/swagger-ui
```

## Distance Metrics

| Metric | API Value | Use Case |
|--------|-----------|----------|
| Cosine | `cosine` | Text embeddings |
| Euclidean | `euclidean` | Spatial data |
| Dot Product | `dot` | Pre-normalized vectors |
| Hamming | `hamming` | Binary vectors |
| Jaccard | `jaccard` | Set similarity |

## Performance

- **Cosine similarity**: ~93 ns per operation (768d)
- **Dot product**: ~36 ns per operation (768d)
- **Search latency**: < 1ms for 100k vectors
- **Throughput**: 28M+ distance calculations/sec

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `VELESDB_PORT` | 8080 | Server port |
| `VELESDB_HOST` | 127.0.0.1 | Bind address (use 0.0.0.0 for network access) |
| `VELESDB_DATA_DIR` | ./data | Data directory |
| `RUST_LOG` | warn | Log level |

## License

VelesDB Core License 1.0

See [LICENSE](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE) for details.
