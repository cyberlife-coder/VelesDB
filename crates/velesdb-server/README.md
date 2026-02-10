# VelesDB Server

[![Crates.io](https://img.shields.io/crates/v/velesdb-server.svg)](https://crates.io/crates/velesdb-server)
[![License](https://img.shields.io/badge/license-ELv2-blue)](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE)

REST API server for VelesDB — a high-performance vector + graph database for AI agents.

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

# Production: with authentication and rate limiting
VELESDB_API_KEY=my-secret-key \
VELESDB_RATE_LIMIT=200 \
VELESDB_CORS_ORIGIN=https://app.example.com \
RUST_LOG=info \
velesdb-server
```

## Authentication

When `VELESDB_API_KEY` is set, all endpoints except `/health` and `/swagger-ui` require the `Authorization` header:

```bash
curl -H "Authorization: Bearer my-secret-key" http://localhost:8080/collections
```

When unset, the server runs in **dev mode** (no authentication required).

## Rate Limiting

Per-IP rate limiting is active by default (100 req/s, burst 50). When exceeded, the server returns **HTTP 429 Too Many Requests**.

Configure via environment variables:

```bash
VELESDB_RATE_LIMIT=200   # requests per second per IP
VELESDB_RATE_BURST=100   # burst capacity
```

The `/health` endpoint is **not** rate limited (safe for monitoring probes).

Rate limiting uses `SmartIpKeyExtractor` — supports `X-Forwarded-For`, `X-Real-IP`, and peer address fallback (works behind reverse proxies).

## API Reference

> Interactive documentation available at [http://localhost:8080/swagger-ui](http://localhost:8080/swagger-ui)

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

# Create metadata-only collection (graph without vectors)
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "knowledge", "collection_type": "metadata_only"}'

# List collections
curl http://localhost:8080/collections

# Get collection info
curl http://localhost:8080/collections/documents

# Check if collection is empty
curl http://localhost:8080/collections/documents/empty

# Flush collection to disk
curl -X POST http://localhost:8080/collections/documents/flush

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
      {"id": 1, "vector": [0.1, 0.2, 0.3], "payload": {"title": "Hello"}},
      {"id": 2, "vector": [0.4, 0.5, 0.6], "payload": {"title": "World"}}
    ]
  }'

# Get a single point by ID
curl http://localhost:8080/collections/documents/points/1

# Delete a single point by ID
curl -X DELETE http://localhost:8080/collections/documents/points/1
```

### Search

```bash
# Vector similarity search
curl -X POST http://localhost:8080/collections/documents/search \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.15, 0.25, 0.35],
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
    "vector": [0.15, 0.25, 0.35],
    "query": "rust programming",
    "top_k": 10,
    "vector_weight": 0.7
  }'

# Batch search (multiple queries in parallel)
curl -X POST http://localhost:8080/collections/documents/search/batch \
  -H "Content-Type: application/json" \
  -d '{
    "searches": [
      {"vector": [0.1, 0.2, 0.3], "top_k": 5},
      {"vector": [0.3, 0.4, 0.5], "top_k": 5}
    ]
  }'

# Multi-query fusion search (for RAG)
curl -X POST http://localhost:8080/collections/documents/search/multi \
  -H "Content-Type: application/json" \
  -d '{
    "vectors": [[0.1, 0.2, 0.3], [0.3, 0.4, 0.5], [0.5, 0.6, 0.7]],
    "top_k": 10,
    "fusion": "rrf",
    "fusion_params": {"k": 60}
  }'

# Weighted fusion strategy
curl -X POST http://localhost:8080/collections/documents/search/multi \
  -H "Content-Type: application/json" \
  -d '{
    "vectors": [[0.1, 0.2, 0.3], [0.3, 0.4, 0.5]],
    "top_k": 10,
    "fusion": "weighted",
    "fusion_params": {"avgWeight": 0.6, "maxWeight": 0.3, "hitWeight": 0.1}
  }'
```

### VelesQL Query

```bash
# Vector search via VelesQL
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT * FROM documents WHERE VECTOR NEAR $v LIMIT 5",
    "params": {"v": [0.15, 0.25, 0.35]}
  }'

# Full-text MATCH via VelesQL
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT * FROM documents WHERE content MATCH '\''rust'\'' LIMIT 10",
    "params": {}
  }'

# Explain query plan
curl -X POST http://localhost:8080/query/explain \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT * FROM documents WHERE VECTOR NEAR $v LIMIT 5"}'
```

### Graph Pattern Matching (MATCH)

```bash
# Execute a graph MATCH query with similarity scoring
curl -X POST http://localhost:8080/collections/knowledge/match \
  -H "Content-Type: application/json" \
  -d '{
    "query": "MATCH (a:Person)-[r:KNOWS]->(b:Person) RETURN a, b",
    "params": {},
    "vector": [0.1, 0.2, 0.3],
    "threshold": 0.7
  }'
```

### Graph Operations

Graph data lives in the Collection's EdgeStore (in-memory; disk persistence planned).

```bash
# Add an edge
curl -X POST http://localhost:8080/collections/knowledge/graph/edges \
  -H "Content-Type: application/json" \
  -d '{
    "id": 1,
    "source": 100,
    "target": 200,
    "label": "KNOWS",
    "properties": {"since": "2024"}
  }'

# Get edges by label (label query param required)
curl "http://localhost:8080/collections/knowledge/graph/edges?label=KNOWS"

# Traverse graph (BFS/DFS)
curl -X POST http://localhost:8080/collections/knowledge/graph/traverse \
  -H "Content-Type: application/json" \
  -d '{
    "source": 100,
    "strategy": "bfs",
    "max_depth": 3,
    "limit": 100,
    "rel_types": ["KNOWS", "FOLLOWS"]
  }'

# Stream traversal via SSE (Server-Sent Events)
curl -N "http://localhost:8080/collections/knowledge/graph/traverse/stream?start_node=100&algorithm=bfs&max_depth=5&limit=1000&relationship_types=KNOWS,FOLLOWS"

# Get node degree (in/out edge count)
curl http://localhost:8080/collections/knowledge/graph/nodes/100/degree
```

### Property Indexes

```bash
# Create a hash index on a property
curl -X POST http://localhost:8080/collections/knowledge/indexes \
  -H "Content-Type: application/json" \
  -d '{"label": "Person", "property": "email", "index_type": "hash"}'

# Create a range index
curl -X POST http://localhost:8080/collections/knowledge/indexes \
  -H "Content-Type: application/json" \
  -d '{"label": "Person", "property": "age", "index_type": "range"}'

# List all indexes
curl http://localhost:8080/collections/knowledge/indexes

# Delete an index
curl -X DELETE http://localhost:8080/collections/knowledge/indexes/Person/email
```

### Health & OpenAPI

```bash
# Health check (not rate limited, no auth required)
curl http://localhost:8080/health

# OpenAPI spec (JSON)
curl http://localhost:8080/api-docs/openapi.json

# Swagger UI (interactive docs)
# Open in browser: http://localhost:8080/swagger-ui
```

## API Routes Summary

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Health check |
| `GET` | `/collections` | List collections |
| `POST` | `/collections` | Create collection |
| `GET` | `/collections/{name}` | Get collection info |
| `DELETE` | `/collections/{name}` | Delete collection |
| `GET` | `/collections/{name}/empty` | Check if empty |
| `POST` | `/collections/{name}/flush` | Flush to disk |
| `POST` | `/collections/{name}/points` | Upsert points |
| `GET` | `/collections/{name}/points/{id}` | Get point by ID |
| `DELETE` | `/collections/{name}/points/{id}` | Delete point by ID |
| `POST` | `/collections/{name}/search` | Vector search |
| `POST` | `/collections/{name}/search/batch` | Batch search |
| `POST` | `/collections/{name}/search/multi` | Multi-query fusion |
| `POST` | `/collections/{name}/search/text` | Full-text search |
| `POST` | `/collections/{name}/search/hybrid` | Hybrid search |
| `GET` | `/collections/{name}/indexes` | List indexes |
| `POST` | `/collections/{name}/indexes` | Create index |
| `DELETE` | `/collections/{name}/indexes/{label}/{property}` | Delete index |
| `POST` | `/query` | VelesQL query |
| `POST` | `/query/explain` | Query plan |
| `POST` | `/collections/{name}/match` | Graph MATCH query |
| `GET` | `/collections/{name}/graph/edges` | Get edges by label |
| `POST` | `/collections/{name}/graph/edges` | Add edge |
| `POST` | `/collections/{name}/graph/traverse` | Graph traversal |
| `GET` | `/collections/{name}/graph/traverse/stream` | SSE stream traversal |
| `GET` | `/collections/{name}/graph/nodes/{node_id}/degree` | Node degree |

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

## CORS Configuration

By default, the server uses permissive CORS (dev mode). For production, restrict allowed origins:

```bash
# Single origin
VELESDB_CORS_ORIGIN=https://app.example.com

# Multiple origins (comma-separated)
VELESDB_CORS_ORIGIN=https://app.example.com,https://admin.example.com
```

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `VELESDB_PORT` | `8080` | Server port |
| `VELESDB_HOST` | `0.0.0.0` | Bind address |
| `VELESDB_DATA_DIR` | `./data` | Data directory |
| `VELESDB_API_KEY` | *(none)* | API key for auth. Unset = dev mode (no auth) |
| `VELESDB_RATE_LIMIT` | `100` | Requests per second per IP |
| `VELESDB_RATE_BURST` | `50` | Burst capacity per IP |
| `VELESDB_CORS_ORIGIN` | *(none)* | Allowed CORS origins (comma-separated). Unset = permissive |
| `RUST_LOG` | `warn` | Log level (`info`, `debug`, `trace`) |

## License

Elastic License 2.0 (ELv2)

See [LICENSE](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE) for details.
