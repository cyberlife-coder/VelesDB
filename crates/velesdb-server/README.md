# VelesDB Server

[![Crates.io](https://img.shields.io/crates/v/velesdb-server.svg)](https://crates.io/crates/velesdb-server)
[![License](https://img.shields.io/crates/l/velesdb-server.svg)](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE)

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
# Create collection
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "documents", "dimension": 768, "metric": "cosine"}'

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

# Get points by IDs
curl -X POST http://localhost:8080/collections/documents/points/get \
  -d '{"ids": [1, 2, 3]}'

# Delete points
curl -X DELETE http://localhost:8080/collections/documents/points \
  -d '{"ids": [1, 2, 3]}'
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

# VelesQL query
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT * FROM documents WHERE VECTOR NEAR $v LIMIT 5",
    "params": {"v": [0.15, 0.25, ...]}
  }'
```

### Health & Info

```bash
# Health check
curl http://localhost:8080/health

# Server info
curl http://localhost:8080/info

# OpenAPI spec
curl http://localhost:8080/openapi.json
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

- **Cosine similarity**: ~76 ns per operation (768d)
- **Search latency**: < 1ms for 100k vectors
- **Throughput**: 13M+ distance calculations/sec

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `VELESDB_PORT` | 8080 | Server port |
| `VELESDB_HOST` | 0.0.0.0 | Bind address |
| `VELESDB_DATA_DIR` | ./data | Data directory |
| `RUST_LOG` | warn | Log level |

## License

Business Source License 1.1 (BSL-1.1)

See [LICENSE](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE) for details.
