# VelesDB Server — REST Tour (curl cookbook)

A hands-on, copy-pasteable tour of the `velesdb-server` HTTP API. Every block
below runs against a server started with:

```bash
velesdb-server --port 8080 --data-dir ./velesdb_data
```

> **Scope.** This is a *cookbook*: short recipes you can paste into a terminal.
> For the exhaustive endpoint specification (every field, every status code),
> use [`docs/reference/api-reference.md`](../reference/api-reference.md) or the
> machine-readable [`docs/openapi.yaml`](../openapi.yaml).

## Route prefixes

All routes are served twice:

| Prefix | Status | Example |
|--------|--------|---------|
| `/v1/` | **Canonical** — use this | `POST /v1/collections` |
| `/` | Legacy, still functional; responses carry `deprecation: true` and `x-api-deprecated: Use /v1/ prefix` | `POST /collections` |

The recipes below use the canonical `/v1/` prefix.

---

## 1. Collections

```bash
# Create a vector collection (default: full precision, cosine)
curl -X POST http://localhost:8080/v1/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "documents", "dimension": 768, "metric": "cosine"}'
```

Response (`201 Created`):

```json
{"message":"Collection created","name":"documents","type":"vector","warnings":["Collection dimension and metric are immutable after creation. If your embedding model changes, create a new collection and reindex data.","For first queries, start without strict filters/thresholds, then tighten progressively."]}
```

```bash
# Quantized collection (SQ8 = 4x memory reduction). Aliases: "sq8", "int8"
curl -X POST http://localhost:8080/v1/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "compressed", "dimension": 768, "metric": "cosine", "storage_mode": "sq8"}'

# Binary collection (Hamming + binary storage = 32x compression). Aliases: "binary", "bit"
curl -X POST http://localhost:8080/v1/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "fingerprints", "dimension": 256, "metric": "hamming", "storage_mode": "binary"}'

# List collections
curl http://localhost:8080/v1/collections

# Collection details (name, dimension, metric, point_count, storage_mode)
curl http://localhost:8080/v1/collections/documents

# Delete a collection
curl -X DELETE http://localhost:8080/v1/collections/documents
```

### Collection types

`collection_type` accepts `"vector"` (the default), `"metadata_only"`, or
`"graph"`.

```bash
# Metadata-only collection (no vectors) — reference data, lookups
curl -X POST http://localhost:8080/v1/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "entities", "collection_type": "metadata_only"}'

# Graph collection. Omit `graph_schema` entirely for a schemaless graph
# that accepts any node/edge type:
curl -X POST http://localhost:8080/v1/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "kg", "collection_type": "graph"}'
```

For a strict schema, pass the full `graph_schema` object. `schemaless` is
required; `node_types` / `edge_types` are typed objects (not bare strings), and
each `properties` map declares the allowed property types (`{}` for none):

```bash
curl -X POST http://localhost:8080/v1/collections \
  -H "Content-Type: application/json" \
  -d '{
        "name": "kg_strict",
        "collection_type": "graph",
        "graph_schema": {
          "schemaless": false,
          "node_types": [
            {"name": "Person",  "properties": {"name": "string"}},
            {"name": "Company", "properties": {}}
          ],
          "edge_types": [
            {"name": "WORKS_AT", "from_type": "Person", "to_type": "Company", "properties": {}},
            {"name": "KNOWS",    "from_type": "Person", "to_type": "Person",  "properties": {}}
          ]
        }
      }'
```

### Optional HNSW tuning parameters

For vector collections, four optional fields override the auto-tuned index
parameters. Omit them to let VelesDB derive values from the vector dimension:

| Field | Meaning | Default (auto-tuned) |
|-------|---------|----------------------|
| `hnsw_m` | Max neighbor connections per node | 24 (≤256 dim) / 32 (>256 dim) |
| `hnsw_ef_construction` | Build-time search breadth | 300 (≤256 dim) / 400 (>256 dim) |
| `hnsw_alpha` | VAMANA neighbor-diversification factor | 1.2 |
| `hnsw_max_elements` | Initial capacity hint (pre-sizing for bulk import) | 100000 |

```bash
curl -X POST http://localhost:8080/v1/collections \
  -H "Content-Type: application/json" \
  -d '{
        "name": "tuned",
        "dimension": 768,
        "metric": "cosine",
        "hnsw_m": 48,
        "hnsw_ef_construction": 800,
        "hnsw_max_elements": 500000
      }'
```

See [TUNING_GUIDE.md](TUNING_GUIDE.md) for how to choose these values.

---

## 2. Points

The examples below use a 4-dimensional collection so they can be pasted as-is:

```bash
curl -X POST http://localhost:8080/v1/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "demo", "dimension": 4, "metric": "cosine"}'

# Upsert points (insert or replace by ID)
curl -X POST http://localhost:8080/v1/collections/demo/points \
  -H "Content-Type: application/json" \
  -d '{
        "points": [
          {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "Hello", "category": "tech"}},
          {"id": 2, "vector": [0.9, 0.4, 0.0, 0.0], "payload": {"title": "World", "category": "tech"}}
        ]
      }'
```

Response: `{"count":2,"message":"Points upserted"}`

```bash
# Get a point by ID
curl http://localhost:8080/v1/collections/demo/points/1

# Delete a point by ID
curl -X DELETE http://localhost:8080/v1/collections/demo/points/1
```

Bulk ingestion with backpressure uses
`POST /v1/collections/{name}/stream/insert`; see the
[API reference](../reference/api-reference.md#post-collectionsnamestreaminsert).

---

## 3. Search

```bash
# Vector similarity search, with a metadata filter
curl -X POST http://localhost:8080/v1/collections/demo/search \
  -H "Content-Type: application/json" \
  -d '{
        "vector": [1.0, 0.0, 0.0, 0.0],
        "top_k": 5,
        "filter": {"condition": {"type": "eq", "field": "category", "value": "tech"}}
      }'
```

Response (point IDs are serialized as strings in payload-bearing result sets):

```json
{"results":[{"id":"1","score":1.0,"payload":{"title":"Hello","category":"tech"}},{"id":"2","score":0.91381156,"payload":{"title":"World","category":"tech"}}]}
```

### The `mode` parameter

```bash
# Named quality mode
curl -X POST http://localhost:8080/v1/collections/demo/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [1.0, 0.0, 0.0, 0.0], "top_k": 10, "mode": "accurate"}'

# Fixed ef_search
curl -X POST http://localhost:8080/v1/collections/demo/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [1.0, 0.0, 0.0, 0.0], "top_k": 10, "mode": "custom:256"}'

# Two-phase adaptive ef_search (auto-escalation for hard queries)
curl -X POST http://localhost:8080/v1/collections/demo/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [1.0, 0.0, 0.0, 0.0], "top_k": 10, "mode": "adaptive:32:512"}'
```

| Value | Description |
|-------|-------------|
| `fast` | Low latency (~92% recall) |
| `balanced` | Default (~99% recall) |
| `accurate` | High precision (~99.5% recall) |
| `perfect` | Exhaustive (100% recall) |
| `autotune` (aliases `auto`, `auto_tune`) | ef computed from collection size |
| `custom:<ef>` | Fixed `ef_search`, e.g. `custom:256` |
| `adaptive:<min>:<max>` | Two-phase adaptive, e.g. `adaptive:32:512` |

Recall figures come from [SEARCH_MODES.md](SEARCH_MODES.md), which explains the
latency/recall trade-off in detail. An unrecognized `mode` string is ignored
(the collection default applies).

### Full-text (BM25)

```bash
curl -X POST http://localhost:8080/v1/collections/documents/search/text \
  -H "Content-Type: application/json" \
  -d '{"query": "rust programming", "top_k": 10}'
```

```json
{
  "results": [
    {"id": "5", "score": 2.134, "payload": {"title": "Rust Programming Guide"}},
    {"id": "12", "score": 1.892, "payload": {"title": "Systems Programming in Rust"}}
  ]
}
```

### Sparse vectors (learned sparse / SPLADE-style)

`sparse_vector` accepts the parallel-array form shown here or the
Qdrant-compatible dict form `{"42": 0.5, "1337": 1.2}`.

```bash
curl -X POST http://localhost:8080/v1/collections/documents/search \
  -H "Content-Type: application/json" \
  -d '{"sparse_vector": {"indices": [42, 1337], "values": [0.5, 1.2]}, "top_k": 10}'

# Named sparse indexes: send `sparse_vectors` (a map) plus `sparse_index`
# to select which one to query when more than one is defined.
curl -X POST http://localhost:8080/v1/collections/documents/search \
  -H "Content-Type: application/json" \
  -d '{
        "sparse_vectors": {"splade": {"indices": [42], "values": [0.9]}},
        "sparse_index": "splade",
        "top_k": 10
      }'
```

### Hybrid, batch, multi-query fusion

```bash
# Hybrid search (dense vector + BM25 text)
curl -X POST http://localhost:8080/v1/collections/documents/search/hybrid \
  -H "Content-Type: application/json" \
  -d '{
        "vector": [1.0, 0.0, 0.0, 0.0],
        "query": "rust programming",
        "top_k": 10,
        "vector_weight": 0.7
      }'

# Batch search (several queries evaluated in parallel)
curl -X POST http://localhost:8080/v1/collections/demo/search/batch \
  -H "Content-Type: application/json" \
  -d '{
        "searches": [
          {"vector": [1.0, 0.0, 0.0, 0.0], "top_k": 5},
          {"vector": [0.0, 1.0, 0.0, 0.0], "top_k": 5}
        ]
      }'

# Multi-query fusion (multi-query generation for RAG), RRF strategy
curl -X POST http://localhost:8080/v1/collections/demo/search/multi \
  -H "Content-Type: application/json" \
  -d '{
        "vectors": [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
        "top_k": 10,
        "strategy": "rrf",
        "rrf_k": 60
      }'

# Multi-query fusion, weighted strategy
curl -X POST http://localhost:8080/v1/collections/demo/search/multi \
  -H "Content-Type: application/json" \
  -d '{
        "vectors": [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]],
        "top_k": 10,
        "strategy": "weighted",
        "avg_weight": 0.6,
        "max_weight": 0.3,
        "hit_weight": 0.1
      }'
```

### Distance metrics

| Metric | API value | Aliases | Use case |
|--------|-----------|---------|----------|
| Cosine | `cosine` | | Text embeddings |
| Euclidean | `euclidean` | | Spatial data |
| Dot product | `dot` | `dotproduct`, `inner`, `ip` | Pre-normalized vectors |
| Hamming | `hamming` | | Binary vectors |
| Jaccard | `jaccard` | | Set similarity |

---

## 4. VelesQL

```bash
# SELECT with vector search
curl -X POST http://localhost:8080/v1/query \
  -H "Content-Type: application/json" \
  -d '{
        "query": "SELECT * FROM demo WHERE VECTOR NEAR $v LIMIT 5",
        "params": {"v": [1.0, 0.0, 0.0, 0.0]}
      }'
```

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
# Full-text MATCH predicate
curl -X POST http://localhost:8080/v1/query \
  -H "Content-Type: application/json" \
  -d '{
        "query": "SELECT * FROM documents WHERE content MATCH '\''rust'\'' LIMIT 10",
        "params": {}
      }'

# Aggregation-only endpoint
curl -X POST http://localhost:8080/v1/aggregate \
  -H "Content-Type: application/json" \
  -d '{
        "query": "SELECT category, COUNT(*) FROM documents GROUP BY category",
        "params": {}
      }'

# Query plan
curl -X POST http://localhost:8080/v1/query/explain \
  -H "Content-Type: application/json" \
  -d '{
        "query": "SELECT * FROM demo WHERE VECTOR NEAR $v LIMIT 5",
        "params": {"v": [1.0, 0.0, 0.0, 0.0]}
      }'
```

```json
{
  "query": "SELECT * FROM demo WHERE VECTOR NEAR $v LIMIT 5",
  "query_type": "select",
  "collection": "demo",
  "plan": [
    {"step": 1, "operation": "VectorSearch", "description": "HNSW nearest-neighbor scan, ef_search=160, limit=5"}
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

Language reference: [VELESQL_SPEC.md](../VELESQL_SPEC.md) and the
[VelesQL cheatsheet](../reference/VELESQL_CHEATSHEET.md).

---

## 5. Graph API

```bash
# List edges filtered by label (label is required)
curl "http://localhost:8080/v1/collections/kg/graph/edges?label=KNOWS"

# Add an edge (id, source, target, label are required)
curl -X POST http://localhost:8080/v1/collections/kg/graph/edges \
  -H "Content-Type: application/json" \
  -d '{"id": 1, "source": 1, "target": 2, "label": "KNOWS"}'

# Remove an edge by ID
curl -X DELETE http://localhost:8080/v1/collections/kg/graph/edges/1

# Total edge count
curl http://localhost:8080/v1/collections/kg/graph/edges/count

# List all node IDs
curl http://localhost:8080/v1/collections/kg/graph/nodes

# Edges of one node (direction: in, out, both)
curl "http://localhost:8080/v1/collections/kg/graph/nodes/1/edges?direction=out"

# Node degree (in + out)
curl http://localhost:8080/v1/collections/kg/graph/nodes/1/degree

# Store and read a node payload
curl -X PUT http://localhost:8080/v1/collections/kg/graph/nodes/1/payload \
  -H "Content-Type: application/json" \
  -d '{"payload": {"name": "Alice", "role": "engineer"}}'
curl http://localhost:8080/v1/collections/kg/graph/nodes/1/payload

# Traversal from one node (BFS or DFS)
curl -X POST http://localhost:8080/v1/collections/kg/graph/traverse \
  -H "Content-Type: application/json" \
  -d '{"source": 1, "strategy": "bfs", "max_depth": 3, "limit": 100}'

# Parallel multi-source BFS
curl -X POST http://localhost:8080/v1/collections/kg/graph/traverse/parallel \
  -H "Content-Type: application/json" \
  -d '{"sources": [1, 5, 10], "max_depth": 3, "limit": 100}'

# Streaming traversal (Server-Sent Events)
curl "http://localhost:8080/v1/collections/kg/graph/traverse/stream?start_node=1"

# Nearest graph nodes by embedding similarity
curl -X POST http://localhost:8080/v1/collections/kg/graph/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [0.1, 0.2, 0.3], "top_k": 10}'
```

### MATCH (Cypher-style pattern matching)

```bash
curl -X POST http://localhost:8080/v1/collections/kg/match \
  -H "Content-Type: application/json" \
  -d '{"query": "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name LIMIT 10"}'

# MATCH combined with vector similarity scoring
curl -X POST http://localhost:8080/v1/collections/kg/match \
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

More patterns: [GRAPH_PATTERNS.md](GRAPH_PATTERNS.md).

---

## 6. Property indexes

```bash
# List indexes on a collection
curl http://localhost:8080/v1/collections/documents/indexes

# Create an index
curl -X POST http://localhost:8080/v1/collections/documents/indexes \
  -H "Content-Type: application/json" \
  -d '{"label": "category", "property": "name"}'

# Delete an index
curl -X DELETE http://localhost:8080/v1/collections/documents/indexes/category/name
```

---

## 7. Health, metrics and OpenAPI

```bash
# Liveness
curl http://localhost:8080/v1/health

# Readiness (503 while collections are still loading)
curl http://localhost:8080/v1/ready

# Prometheus metrics (feature `prometheus`, enabled by default)
curl http://localhost:8080/v1/metrics

# OpenAPI document, and Swagger UI (requires --features swagger-ui at build time)
curl http://localhost:8080/api-docs/openapi.json
# then open http://localhost:8080/swagger-ui in a browser
```

Both probes are also served unprefixed (`/health`, `/ready`), and both stay
public when API keys are configured.

`/v1/health` always answers `200` while the process is up:

```json
{"status": "ok", "version": "6.0.0"}
```

`/v1/ready` answers `200` once every collection is loaded from disk:

```json
{"status": "ready", "version": "6.0.0"}
```

…and `503` until then, which is what makes it usable as a Kubernetes
readiness probe:

```json
{"status": "not_ready", "version": "6.0.0"}
```

---

## 8. Errors

Every error response carries an `error` field with a human-readable message.
When the failure maps to a structured VelesDB error, the body also carries the
`code` field:

```json
{"error":"[VELES-002] Collection 'nope' not found","code":"VELES-002"}
```

```json
{"error":"Vector dimension mismatch for collection 'demo': expected 4, got 2. Hint: use embeddings with the same dimension as the collection or create a new collection with the target dimension.","code":"VELES-004"}
```

Use `code` for programmatic handling (retry, user hint); the full list lives in
[ERROR_CODES.md](../reference/ERROR_CODES.md).

---

## 9. Performance reference

Numbers match the canonical contract in
[`docs/reference/promise-contract.json`](../reference/promise-contract.json)
(i9-14900KF, AVX2, `--release`, `target-cpu=native`):

- **Cosine similarity**: ~33 ns per operation (768D)
- **Dot product**: ~21.7 ns per operation (768D), ~35 Gelem/s
- **HNSW search (index only)**: ~55 µs (10K vectors, 768D, Balanced mode, k=10)
- **End-to-end search p50**: ~450 µs (10K/384D, WAL on, recall ≥ 96%)

---

## See also

- [velesdb-server README](../../crates/velesdb-server/README.md) — install and 60-second start
- [Server deployment](SERVER_DEPLOYMENT.md) — Docker, Kubernetes probes, rate limiting, CORS
- [Server security](SERVER_SECURITY.md) — API keys, TLS, graceful shutdown
- [Configuration](CONFIGURATION.md) — every option, env var, and TOML key
- [API reference](../reference/api-reference.md) — exhaustive endpoint specification

---

Last updated: 2026-08-08 · Applies to: velesdb-core 6.0.0
