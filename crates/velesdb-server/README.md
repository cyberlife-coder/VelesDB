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
# Build from the repository root
docker build -t velesdb .
docker run -p 8080:8080 -v velesdb_data:/data velesdb
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
velesdb-server --port 9000 --data-dir ./my_vectors

# With logging
RUST_LOG=info velesdb-server
```

### Quick Start Flow

After starting the server, follow this sequence:

1. **Create a collection** — `POST /collections` (define dimension and metric)
2. **Insert vectors** — `POST /collections/{name}/points` (with optional payloads)
3. **Search** — `POST /collections/{name}/search` (send a query vector, get top-k results)
4. **Add filters** — Add metadata conditions to narrow results
5. **Tune** — Adjust `ef_search` or use `SearchQuality::Adaptive` for production

The data directory auto-creates if it doesn't exist. Default: `./velesdb_data`.

## API Reference

### Collections

```bash
# Create collection (default: full precision, cosine)
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "documents", "dimension": 768, "metric": "cosine"}'
```

Response (`201 Created`):
```json
{"name": "documents", "dimension": 768, "metric": "cosine", "point_count": 0, "storage_mode": "full"}
```

```bash
# Create collection with quantization (SQ8 = 4x memory reduction)
# Aliases: "sq8" or "int8"
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "compressed", "dimension": 768, "metric": "cosine", "storage_mode": "sq8"}'

# Create binary collection (Hamming + Binary = 32x compression)
# Aliases: "binary" or "bit"
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
    "filter": {"type": "eq", "field": "category", "value": "tech"}
  }'

# Search with explicit search quality mode
curl -X POST http://localhost:8080/collections/documents/search \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.15, 0.25, ...],
    "top_k": 10,
    "mode": "accurate"
  }'

# Search with custom ef_search (fine-grained control)
curl -X POST http://localhost:8080/collections/documents/search \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.15, 0.25, ...],
    "top_k": 10,
    "mode": "custom:256"
  }'

# Search with adaptive ef_search (auto-escalation for hard queries)
curl -X POST http://localhost:8080/collections/documents/search \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.15, 0.25, ...],
    "top_k": 10,
    "mode": "adaptive:32:512"
  }'
```

The `mode` parameter accepts the following values:

| Value | Description |
|-------|-------------|
| `fast` | Low latency (~92% recall) |
| `balanced` | Default (~99% recall) |
| `accurate` | High precision (~99.5% recall) |
| `perfect` | Exhaustive (100% recall) |
| `autotune` | Auto-computed ef from collection size |
| `custom:<ef>` | Fixed ef_search (e.g., `custom:256`) |
| `adaptive:<min>:<max>` | Two-phase adaptive (e.g., `adaptive:32:512`) |

Response:
```json
{
  "results": [
    {"id": 1, "score": 0.9523, "payload": {"title": "Hello", "category": "tech"}},
    {"id": 2, "score": 0.8712, "payload": {"title": "World", "category": "tech"}}
  ]
}
```

```bash
# BM25 full-text search
curl -X POST http://localhost:8080/collections/documents/search/text \
  -H "Content-Type: application/json" \
  -d '{"query": "rust programming", "top_k": 10}'
```

Response:
```json
{
  "results": [
    {"id": 5, "score": 2.134, "payload": {"title": "Rust Programming Guide"}},
    {"id": 12, "score": 1.892, "payload": {"title": "Systems Programming in Rust"}}
  ]
}
```

```bash
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
    "strategy": "rrf",
    "rrf_k": 60
  }'

# Weighted fusion strategy
curl -X POST http://localhost:8080/collections/documents/search/multi \
  -H "Content-Type: application/json" \
  -d '{
    "vectors": [[...], [...], [...]],
    "top_k": 10,
    "strategy": "weighted",
    "avg_weight": 0.6,
    "max_weight": 0.3,
    "hit_weight": 0.1
  }'

# VelesQL query
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT * FROM documents WHERE VECTOR NEAR $v LIMIT 5",
    "params": {"v": [0.15, 0.25, ...]}
  }'
```

Response:
```json
{
  "results": [
    {"id": 1, "score": 0.95, "title": "Hello"},
    {"id": 3, "score": 0.88, "title": "World"}
  ],
  "timing_ms": 0.42,
  "took_ms": 1,
  "rows_returned": 2,
  "meta": {"velesql_contract_version": "3.0.0", "count": 2}
}
```

```bash
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

Response:
```json
{
  "query": "SELECT * FROM documents WHERE VECTOR NEAR $v LIMIT 5",
  "query_type": "select",
  "collection": "documents",
  "plan": [
    {"step": 1, "operation": "VectorSearch", "description": "HNSW nearest-neighbor scan, ef_search=128, limit=5"}
  ],
  "estimated_cost": {
    "uses_index": true,
    "index_name": "hnsw",
    "selectivity": 0.005,
    "complexity": "O(log n)"
  },
  "features": {},
  "cache_hit": false,
  "plan_reuse_count": 0
}
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

### MATCH Query API (Cypher-like graph pattern matching)

```bash
# Execute a MATCH graph traversal query
curl -X POST http://localhost:8080/collections/documents/match \
  -H "Content-Type: application/json" \
  -d '{
    "query": "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name LIMIT 10"
  }'

# MATCH with vector similarity scoring
curl -X POST http://localhost:8080/collections/documents/match \
  -H "Content-Type: application/json" \
  -d '{
    "query": "MATCH (a:Person)-[:KNOWS]->(b) RETURN a, b LIMIT 10",
    "vector": [0.1, 0.2, 0.3],
    "threshold": 0.5
  }'
```

Response format:
```json
[
  {
    "node_id": 20,
    "depth": 1,
    "path": [1],
    "bindings": {"a": 10, "b": 20},
    "score": 0.85,
    "projected": {"a.name": "Alice", "b.name": "Bob"}
  }
]
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

## Error Responses

All error responses include an `error` field with a human-readable message. When the
error maps to a structured VelesDB error code, the response also includes a `code` field:

```json
{"error": "Vector dimension mismatch: expected 768, got 384", "code": "VELES-004"}
```

The `code` field is optional and omitted when no structured code applies. Use it for
programmatic error handling (e.g., retry on `VELES-006`, display user hint on `VELES-004`).
See [ERROR_CODES.md](../../docs/reference/ERROR_CODES.md) for the full list.

## Authentication

VelesDB supports optional API key authentication. When no keys are configured, the server runs in **local dev mode** (all requests are accepted). When one or more keys are configured, every request must include a valid `Authorization: Bearer <key>` header.

### Enabling Authentication

**Via environment variable** (comma-separated list):

```bash
VELESDB_API_KEYS="sk-prod-abc123,sk-prod-def456" velesdb-server
```

**Via `velesdb.toml`**:

```toml
[auth]
api_keys = ["sk-prod-abc123", "sk-prod-def456"]
```

You can configure as many keys as you need. This is useful for rotating keys without downtime: add the new key, deploy, then remove the old key.

### Making Authenticated Requests

```bash
curl -X GET http://localhost:8080/collections \
  -H "Authorization: Bearer sk-prod-abc123"
```

If authentication is enabled and the header is missing or invalid, the server returns `401 Unauthorized`:

```json
{"error": "Unauthorized", "message": "missing Authorization header"}
```

### Public Endpoints (No Auth Required)

The following endpoints bypass authentication even when API keys are configured:

| Endpoint | Purpose |
|----------|---------|
| `GET /health` | Liveness probe |
| `GET /ready` | Readiness probe |
| `GET /metrics` | Prometheus metrics (if enabled) |

This allows load balancers and orchestrators to probe the server without credentials.

## TLS / HTTPS

VelesDB supports native TLS via rustls (no OpenSSL dependency). When both a certificate and private key are provided, the server binds with HTTPS instead of HTTP.

### Generating Self-Signed Certificates (Development)

```bash
openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem \
  -days 365 -nodes -subj "/CN=localhost"
```

### Configuring TLS

**Via environment variables**:

```bash
VELESDB_TLS_CERT=./cert.pem VELESDB_TLS_KEY=./key.pem velesdb-server
```

**Via CLI flags**:

```bash
velesdb-server --tls-cert ./cert.pem --tls-key ./key.pem
```

**Via `velesdb.toml`**:

```toml
[tls]
cert = "/etc/velesdb/cert.pem"
key  = "/etc/velesdb/key.pem"
```

Both `cert` and `key` must be provided together. The server will refuse to start if only one is set or if the files do not exist.

### Making Requests Over HTTPS

With a self-signed certificate:

```bash
curl --cacert cert.pem https://localhost:8080/health
```

Or skip verification during development (not for production):

```bash
curl -k https://localhost:8080/health
```

## Graceful Shutdown

VelesDB performs a clean shutdown when it receives **SIGINT** (Ctrl+C) or **SIGTERM** (on Unix). The shutdown sequence:

1. **Stop accepting new connections** -- the listening socket is closed immediately.
2. **Drain in-flight requests** -- the server waits up to `shutdown_timeout_secs` (default: 30 seconds) for active connections to complete.
3. **Flush all WALs** -- every collection's Write-Ahead Log is flushed to disk, ensuring no acknowledged writes are lost.
4. **Exit** -- the process terminates cleanly.

If the drain timeout expires with connections still active, the server logs a warning and proceeds to the WAL flush.

### Configuring the Drain Timeout

**Via `velesdb.toml`**:

```toml
[server]
shutdown_timeout_secs = 60
```

The default is 30 seconds, which is sufficient for most workloads.

## Health & Readiness Probes

### `GET /health` -- Liveness Probe

Always returns `200 OK` as long as the process is running. Use this for container liveness checks.

```bash
curl http://localhost:8080/health
```

Response:

```json
{"status": "ok", "version": "1.10.0"}
```

### `GET /ready` -- Readiness Probe

Returns `200 OK` once the database has finished loading all collections from disk. Returns `503 Service Unavailable` while loading.

```bash
curl http://localhost:8080/ready
```

Response (ready):

```json
{"status": "ready", "version": "1.10.0"}
```

Response (not ready):

```json
{"status": "not_ready", "version": "1.10.0"}
```

### Kubernetes Example

```yaml
livenessProbe:
  httpGet:
    path: /health
    port: 8080
  initialDelaySeconds: 5
  periodSeconds: 10

readinessProbe:
  httpGet:
    path: /ready
    port: 8080
  initialDelaySeconds: 2
  periodSeconds: 5
```

## Distance Metrics

| Metric | API Value | Aliases | Use Case |
|--------|-----------|---------|----------|
| Cosine | `cosine` | | Text embeddings |
| Euclidean | `euclidean` | | Spatial data |
| Dot Product | `dot` | `dotproduct`, `inner`, `ip` | Pre-normalized vectors |
| Hamming | `hamming` | | Binary vectors |
| Jaccard | `jaccard` | | Set similarity |

## Performance

- **Cosine similarity**: ~33.1 ns per operation (768d)
- **Dot product**: ~19.8 ns per operation (768d)
- **Search latency**: 47.0 µs for 10K vectors (768D, Balanced mode)
- **Throughput**: 38.8 Gelem/s (dot product, 768D)

## Configuration Reference

VelesDB loads configuration with the following priority (highest wins):

**CLI flags > Environment variables > `velesdb.toml` file > Built-in defaults**

A custom config file path can be specified with `--config /path/to/velesdb.toml` or `VELESDB_CONFIG`.

### Environment Variables and CLI Flags

| Environment Variable | CLI Flag | Default | Description |
|---------------------|----------|---------|-------------|
| `VELESDB_HOST` | `--host` | `127.0.0.1` | Bind address. Use `0.0.0.0` for network access. |
| `VELESDB_PORT` | `--port` / `-p` | `8080` | Server port. |
| `VELESDB_DATA_DIR` | `--data-dir` / `-d` | `./velesdb_data` | Data directory for persistent storage. |
| `VELESDB_CONFIG` | `--config` / `-c` | `./velesdb.toml` | Path to TOML configuration file (optional). |
| `VELESDB_API_KEYS` | -- | *(empty)* | Comma-separated API keys. When set, enables Bearer token auth. |
| `VELESDB_TLS_CERT` | `--tls-cert` | *(none)* | Path to TLS certificate file (PEM). Requires `VELESDB_TLS_KEY`. |
| `VELESDB_TLS_KEY` | `--tls-key` | *(none)* | Path to TLS private key file (PEM). Requires `VELESDB_TLS_CERT`. |
| `RUST_LOG` | -- | `info` | Log level filter (e.g. `warn`, `info`, `debug`, `trace`). |
| `VELESDB_NO_UPDATE_CHECK` | -- | *(unset)* | Set to `1` to disable startup update check. |

### TOML Configuration File

```toml
[server]
host = "0.0.0.0"
port = 8080
data_dir = "/var/lib/velesdb"
shutdown_timeout_secs = 30

[auth]
api_keys = ["sk-prod-abc123", "sk-prod-def456"]

[tls]
cert = "/etc/velesdb/cert.pem"
key  = "/etc/velesdb/key.pem"

# Startup update check (enabled by default, no PII collected)
[update_check]
enabled = true       # set to false to disable
# timeout_ms = 2000  # network timeout in ms
```

All sections and fields are optional. Only include what you need to override.

### Update Check (v1.9.2+)

On startup, the server performs a non-blocking version check against `velesdb.com/api/check`. This sends only: version, OS, architecture, and a non-reversible SHA256 instance hash. No personal data is collected.

Disable via environment variable or config:

```bash
export VELESDB_NO_UPDATE_CHECK=1
```

## License

VelesDB Core License 1.0

See [LICENSE](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE) for details.
