# VelesDB API Reference

Complete REST API documentation for VelesDB.

> **Last updated**: 2026-06-12 (VelesDB v2.0.0). The machine-readable source of
> truth is [`docs/openapi.yaml`](../openapi.yaml), regenerated from the server's
> annotated handlers; this page is the human-readable companion.

## Base URL

```
http://localhost:8080
```

### API Versioning

All routes are available under two prefixes:

| Prefix | Status | Example |
|--------|--------|---------|
| `/v1/` | **Canonical** (recommended) | `POST /v1/collections` |
| `/` (no prefix) | **Legacy** (deprecated) | `POST /collections` |

Legacy (unversioned) routes return deprecation headers in every response:

| Header | Value |
|--------|-------|
| `deprecation` | `true` |
| `x-api-deprecated` | `Use /v1/ prefix` |

New integrations should use the `/v1/` prefix exclusively. Legacy routes will be removed in a future major version.

### Rate Limiting

Per-IP rate limiting is enabled by default (100 requests/second per IP). When a client exceeds the limit, the server responds with `429 Too Many Requests`.

**Rate-limit response headers** (present on every response when rate limiting is enabled):

| Header | Type | Description |
|--------|------|-------------|
| `x-ratelimit-limit` | integer | Maximum requests allowed per second |
| `x-ratelimit-remaining` | integer | Remaining requests in the current window |
| `x-ratelimit-after` | integer | Seconds until the bucket refills |
| `retry-after` | integer | Seconds to wait before retrying (only on 429 responses) |

**Configuration**: See [CONFIGURATION.md](../guides/CONFIGURATION.md#rate-limiting) for CLI, environment variable, and TOML options.

---

## Health Check

### GET /health

Check server health status.

**Response:**
```json
{
  "status": "ok",
  "version": "3.3.0"
}
```

### GET /ready

Readiness probe. Returns `200` once the database is fully loaded, `503` before
that. Use `/health` for liveness and `/ready` for load-balancer readiness gates.

---

## Collections

### GET /collections

List all collections.

**Response:**
```json
{
  "collections": ["documents", "products", "images"]
}
```

### POST /collections

Create a new collection.

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| name | string | Yes | Unique collection name |
| dimension | integer | Yes (vector/graph) | Vector dimension (e.g., 768). Omit for `metadata_only` |
| metric | string | No | Distance metric (see table below) |
| storage_mode | string | No | `full` (default), `sq8`, or `binary` quantization |
| collection_type | string | No | `vector` (default), `metadata_only`, or `graph` (see [CollectionType](#collectiontype-descriptor)) |
| hnsw_m | integer | No | Tuned HNSW: bi-directional links per node |
| hnsw_ef_construction | integer | No | Tuned HNSW: candidate list size during build |
| hnsw_alpha | float | No | VAMANA neighbor-diversification factor (≥ 1.0) |
| hnsw_max_elements | integer | No | Initial HNSW capacity (pre-size for bulk import) |

**Distance Metrics:**

| Metric | Aliases | Description | Best For |
|--------|---------|-------------|----------|
| `cosine` | | Cosine similarity (default) | Text embeddings, semantic search |
| `euclidean` | | L2 distance | Spatial data, image features |
| `dotproduct` | `dot`, `inner`, `ip` | Inner product (MIPS) | Recommendations, ranking |
| `hamming` | | Bit difference count | Binary embeddings, fingerprints |
| `jaccard` | | Set intersection/union | Tags, preferences, document similarity |

**Example (standard embeddings):**
```json
{
  "name": "documents",
  "dimension": 768,
  "metric": "cosine"
}
```

**Example (binary vectors with Hamming):**
```json
{
  "name": "image_hashes",
  "dimension": 64,
  "metric": "hamming"
}
```

**Example (set similarity with Jaccard):**
```json
{
  "name": "user_preferences",
  "dimension": 100,
  "metric": "jaccard"
}
```

**Example (tuned HNSW for higher recall):**

Any of `hnsw_m`, `hnsw_ef_construction`, `hnsw_alpha`, or `hnsw_max_elements`
present switches collection creation onto the tuned-parameters path; omitted
fields keep the engine defaults (auto-derived from `dimension`). Larger `hnsw_m`
and `hnsw_ef_construction` raise recall and index size at a build-time cost;
`hnsw_max_elements` only pre-sizes capacity for bulk imports (the index still
grows automatically if exceeded). Out-of-range tunables (e.g. `hnsw_alpha < 1.0`
or non-finite) are rejected with `400`.

```json
{
  "name": "documents",
  "dimension": 768,
  "metric": "cosine",
  "hnsw_m": 48,
  "hnsw_ef_construction": 600,
  "hnsw_alpha": 1.2
}
```

**Example (metadata-only collection):**

A `metadata_only` collection stores payload rows with no vectors and no HNSW
index. It supports CRUD and VelesQL queries over the payload but not vector
search. `dimension` is ignored and may be omitted.

```json
{
  "name": "catalog",
  "collection_type": "metadata_only"
}
```

#### CollectionType descriptor

The `collection_type` field selects the runtime descriptor for the collection:

| `collection_type` | Vectors / HNSW | Use case |
|-------------------|----------------|----------|
| `vector` (default) | Yes — HNSW index over `dimension`-d vectors | Semantic search, RAG, recommendations |
| `metadata_only` | No | Reference tables, catalogs, payload-only stores; CRUD + VelesQL on payload, no vector search |
| `graph` | Optional node embeddings (`dimension` may be null) + typed edges | Knowledge graphs, agentic memory, entity-relationship storage. Supply `graph_schema` for a strict schema; absent means schemaless |

**Response (201 Created):**
```json
{
  "message": "Collection created",
  "name": "documents"
}
```

### GET /collections/:name

Get collection details.

**Response:**
```json
{
  "name": "documents",
  "dimension": 768,
  "metric": "cosine",
  "point_count": 1000
}
```

**Field notes:**

| Field | Description |
|-------|-------------|
| `point_count` | Number of points in storage. During batch upsert or deferred indexing, this may temporarily exceed the HNSW-indexed count. All stored points are searchable once indexing completes. |

### DELETE /collections/:name

Delete a collection and all its data.

**Response:**
```json
{
  "message": "Collection deleted",
  "name": "documents"
}
```

### GET /collections/:name/config

Get detailed collection configuration: HNSW parameters, storage mode, schema,
deferred-indexing and async-index-builder settings. Returns a
`CollectionConfigResponse` (`name`, `dimension`, `metric`, `storage_mode`,
`point_count`, `metadata_only`, optional `embedding_dimension`,
`deferred_indexing`, `async_index_builder`).

### Collection health diagnostics

Three read-only endpoints surface collection health for onboarding and
troubleshooting, ordered cheapest to richest: `GET …/empty` (single boolean),
`GET …/sanity` (live readiness + hints, no `ANALYZE` required), and
`GET …/stats` (cached statistics from the last `ANALYZE`).

### GET /collections/:name/empty

Check whether a collection contains no points. Returns `200` with an empty/has-points
status object, `404` when the collection does not exist.

### GET /collections/:name/sanity

Quick onboarding/troubleshooting check, computed live (no `ANALYZE` needed):
reports point count, whether the index is search-ready, and actionable hints.
Returns a status object (`200`) or `404`.

**Response (200):**
```json
{
  "collection": "documents",
  "dimension": 768,
  "metric": "cosine",
  "point_count": 1000,
  "is_empty": false,
  "checks": {
    "has_vectors": true,
    "search_ready": true,
    "dimension_configured": true
  },
  "diagnostics": {
    "search_requests_total": 42,
    "dimension_mismatch_total": 0,
    "empty_search_results_total": 1,
    "filter_parse_errors_total": 0
  },
  "hints": [
    "Run a search without strict filters first, then tighten filters progressively."
  ]
}
```

### GET /collections/:name/stats

Get **cached** collection statistics computed by the last `ANALYZE`. Returns `404`
if the collection was never analyzed — run `POST /collections/:name/analyze` first.

**Response** (`CollectionStatsResponse`):
```json
{
  "total_points": 50000,
  "total_size_bytes": 104857600,
  "row_count": 49500,
  "deleted_count": 500,
  "avg_row_size_bytes": 2048,
  "payload_size_bytes": 5120000,
  "column_stats": {},
  "index_stats": {},
  "last_analyzed_epoch_ms": 1765500000000
}
```

### POST /collections/:name/analyze

Analyze a collection: computes, persists, and returns the statistics served by
`GET /collections/:name/stats` (same `CollectionStatsResponse` shape).

### POST /collections/:name/flush

Flush pending changes (WAL, payload log, index) to disk. Returns `200` on
success, `404`/`500` on error.

---

## Points

### POST /collections/:name/points

Insert or update points (upsert).

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| points | array | Yes | Array of points to upsert |
| points[].id | integer | Yes | Unique point ID |
| points[].vector | array[float] | Yes | Vector embedding |
| points[].payload | object | No | JSON metadata |

**Example:**
```json
{
  "points": [
    {
      "id": 1,
      "vector": [0.1, 0.2, 0.3, ...],
      "payload": {"title": "Hello World", "category": "greeting"}
    }
  ]
}
```

**Response:**
```json
{
  "message": "Points upserted",
  "count": 1
}
```

**Metadata-only / payload upsert:**

For a `metadata_only` collection there are no vectors — upsert points carrying
only `id` and `payload`, with an empty `vector`. The point is stored and is
queryable via VelesQL over its payload, but is not added to any HNSW index.
(In `metadata_only` collections, vector search is unavailable by design.)

```json
{
  "points": [
    {
      "id": 1,
      "vector": [],
      "payload": {"sku": "A-100", "category": "books", "in_stock": true}
    }
  ]
}
```

### POST /collections/:name/points/raw

Bulk-upsert points via a compact binary wire format (`application/octet-stream`)
for zero-copy, high-throughput ingestion. This avoids the per-point JSON
overhead of `POST /collections/:name/points`. **Payloads are not carried on this
path** — use the JSON endpoint when you need them.

**Wire format (`VRB1`, little-endian).** All multi-byte integers and `f32`s are
little-endian:

| Offset | Size | Field |
|--------|------|-------|
| 0 | 4 | magic `b"VRB1"` (Veles Raw Bulk v1) |
| 4 | 4 | `count` : u32 (number of points) |
| 8 | 4 | `dim` : u32 (vector dimension) |
| 12 | 1 | `id_width` : u8 (must be `8` → u64) |
| 13 | 3 | reserved (must be `0`) |
| 16 | `count * 8` | `ids` : packed `[u64; count]` |
| 16 + `count*8` | `count * dim * 4` | `vectors` : packed `[f32; count*dim]` (row-major) |

The body length must be **exactly** `16 + count*8 + count*dim*4` bytes; any
mismatch, a bad magic, an unsupported `id_width`, or a `dim` that differs from
the collection returns `400 Bad Request`. The encoding is deterministic: a
given batch always serialises to the same bytes.

**Response:**
```json
{
  "message": "Points upserted",
  "count": 1000
}
```

The TypeScript SDK encodes this format for you via
`client.upsertBatchRaw(collection, docs)`.

### GET /collections/:name/points/:id

Get a single point by ID.

**Response:**
```json
{
  "id": 1,
  "vector": [0.1, 0.2, 0.3, ...],
  "payload": {"title": "Hello World"}
}
```

### DELETE /collections/:name/points/:id

Delete a point by ID.

**Response:**
```json
{
  "message": "Point deleted",
  "id": 1
}
```

### POST /collections/:name/points/delete

Bulk-delete points by ID in a single call (idempotent: missing IDs are skipped,
`{"ids": []}` is a successful no-op). Batches above 10 000 IDs return `400`.

**Request Body:**
```json
{
  "ids": [1, 2, 3]
}
```

**Response:** `200` with the number of points submitted for deletion.

### POST /collections/:name/points/scroll

Cursor-based pagination over all points of a collection (ascending ID order).

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| batch_size | integer | No | Points per batch (1–10 000, default: 100) |
| cursor | integer/null | No | Resume **after** this point ID (exclusive); omit to start from the beginning |
| filter | object | No | Optional canonical filter expression |

**Response:**
```json
{
  "points": [
    {"id": "1", "vector": [0.1, 0.2, ...], "payload": {"title": "Doc 1"}}
  ],
  "next_cursor": 1
}
```

`next_cursor` is `null` once iteration is complete. Point IDs are serialized as
strings (see the Point ID encoding note in [Search](#search)).

### POST /collections/:name/points/stream

Stream-upsert points as NDJSON (`application/x-ndjson`, one JSON point per line).
Points are accumulated into micro-batches and flushed via bulk upsert. The
response includes a `network_errors` count: a non-zero value means the HTTP body
stream was truncated and fewer points than sent may have been received.

### POST /collections/:name/stream/enable

Enable the bounded streaming-ingestion channel on a collection. Call this once
before `POST /collections/:name/stream/insert`. Every field is optional; omitted
fields fall back to the server defaults.

**Request Body:**

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| buffer_size | integer | No | 10000 | Bounded ingestion channel capacity |
| batch_size | integer | No | 128 | Points flushed to the index per batch |
| flush_interval_ms | integer | No | 50 | Max milliseconds before a partial batch is flushed |

```json
{ "buffer_size": 4096, "batch_size": 64, "flush_interval_ms": 25 }
```

**Status codes:** `200` enabled (response `{ "message", "collection" }`); `404`
collection not found.

TypeScript SDK: `await db.enableStreaming('docs', { bufferSize: 4096 })` (REST
backend; the WASM backend throws `NOT_SUPPORTED`).

### POST /collections/:name/stream/insert

Insert a **single** point through the bounded streaming-ingestion channel.

**Request Body:**
```json
{
  "id": 1,
  "vector": [0.1, 0.2, 0.3],
  "payload": {"title": "Doc 1"}
}
```

**Status codes:** `202` accepted into the buffer; `404` collection not found;
`409` streaming not configured for the collection; `429` buffer full (with
`Retry-After: 1`); `503` drain task has exited.

### PATCH /collections/:name/points/:id/ttl

Set (or refresh) the **durable TTL** of a point. The expiry is persisted as the
reserved `_veles_expires_at` payload field (epoch seconds), so it survives a
restart.

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| ttl_seconds | integer | Yes | Seconds from now until expiry; `0` expires the point immediately |

**Response:** `204 No Content` on success; `400` when the point's payload is not
a JSON object; `404` when the collection or point does not exist.

**TTL semantics:** expired points are excluded from **all** read surfaces
(search, get, scroll, VelesQL query, and `MATCH`); refreshing an already-expired
point returns `404`; storage is reclaimed lazily (expired entries are swept
later, e.g. by the agent-memory `auto_expire` sweep), not at expiry time.

---

## Search

> **Point ID encoding.** Search, `search/ids`, and `scroll` responses serialize
> point IDs as JSON **strings** (`"id": "1"`). A `u64` ID above
> `Number.MAX_SAFE_INTEGER` (2^53 − 1) would silently lose precision when parsed
> as a JavaScript number, so these payload-bearing result sets quote the ID.
> Other endpoints — `GET /collections/:name/points/:id`, point insert, and the
> VelesQL `POST /query` projected rows — return the ID in its native **integer**
> form. Both string and number are accepted on input.

### POST /collections/:name/search

Search for similar vectors.

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| vector | array[float] | Yes | Query vector |
| top_k | integer | No | Number of results (default: 10) |
| filter | object | No | Optional metadata filter (see shape below) |

**Example:**
```json
{
  "vector": [0.15, 0.25, 0.35, ...],
  "top_k": 5
}
```

**Example with a metadata filter:**

The `filter` uses the canonical VelesDB filter shape:
`{"condition": {"type": <op>, "field": ..., "value"/"values"/"pattern"/"conditions": ...}}`.
Operators: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `in`, `contains`, `like`, `ilike`,
`is_null`, `is_not_null`, `array_contains`, `array_contains_any`, `array_contains_all`,
`geo_distance`, `geo_bbox`, and `and`/`or`/`not` for composition. A malformed filter
returns `400`.

```json
{
  "vector": [0.1, 0.2, 0.3],
  "top_k": 5,
  "filter": {"condition": {"type": "eq", "field": "category", "value": "tech"}}
}
```

**Response:**
```json
{
  "results": [
    {
      "id": "1",
      "score": 0.98,
      "payload": {"title": "Hello World"}
    }
  ]
}
```

### POST /collections/:name/search/text

BM25 full-text search across document payloads.

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| query | string | Yes | Text search query |
| top_k | integer | No | Number of results (default: 10) |

**Example:**
```json
{
  "query": "rust programming language",
  "top_k": 10
}
```

**Response:**
```json
{
  "results": [
    {
      "id": "1",
      "score": 2.45,
      "payload": {"content": "Learn Rust programming"}
    }
  ],
  "timing_ms": 1.23
}
```

### POST /collections/:name/search/hybrid

Hybrid search combining vector similarity and BM25 text relevance using Reciprocal Rank Fusion (RRF).

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| vector | array[float] | Yes | Query vector |
| query | string | Yes | Text search query |
| top_k | integer | No | Number of results (default: 10) |
| vector_weight | float | No | Weight for vector results (0.0-1.0, default: 0.5) |

**Example:**
```json
{
  "vector": [0.1, 0.2, 0.3, ...],
  "query": "rust programming",
  "top_k": 10,
  "vector_weight": 0.7
}
```

**Response:**
```json
{
  "results": [
    {
      "id": "1",
      "score": 0.0312,
      "payload": {"content": "Rust programming guide"}
    }
  ],
  "timing_ms": 2.45
}
```

### POST /collections/:name/search/ids

Lightweight search returning only IDs and scores — no payload hydration.
Accepts the same request body as `POST /collections/:name/search` (dense,
sparse, and hybrid modes; `filter`, `ef_search`, `mode`, `fusion` are honored).

**Response:**
```json
{
  "results": [
    {"id": "1", "score": 0.98}
  ]
}
```

---

## Error Responses

All errors return a JSON object with an `error` field and an optional `code` field
containing the structured VELES-XXX error code (when applicable):

```json
{
  "error": "Vector dimension mismatch: expected 768, got 384",
  "code": "VELES-004"
}
```

The `code` field is omitted when no structured error code applies (e.g., generic
validation errors). See [ERROR_CODES.md](ERROR_CODES.md) for the full list of codes.

For VelesQL semantic/runtime errors (`/query`, `/aggregate`, `/query/explain`), payload is standardized:

```json
{
  "error": {
    "code": "VELESQL_COLLECTION_NOT_FOUND",
    "message": "Collection 'documents' not found",
    "hint": "Create the collection first or correct the collection name"
  }
}
```

### HTTP Status Codes

| Code | Description |
|------|-------------|
| 200 | Success |
| 201 | Created |
| 400 | Bad Request (invalid input) |
| 404 | Not Found |
| 429 | Too Many Requests (rate limit exceeded or streaming backpressure) |
| 500 | Internal Server Error |

---

## Batch Search

### POST /collections/:name/search/batch

Execute multiple searches in a single request.

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| searches | array | Yes | Array of search requests |
| searches[].vector | array[float] | Yes | Query vector |
| searches[].top_k | integer | No | Results per query (default: 10) |

**Example:**
```json
{
  "searches": [
    {"vector": [0.1, 0.2, 0.3, ...], "top_k": 5},
    {"vector": [0.4, 0.5, 0.6, ...], "top_k": 5}
  ]
}
```

**Response:**
```json
{
  "results": [
    {"results": [{"id": "1", "score": 0.98, "payload": {...}}]},
    {"results": [{"id": "2", "score": 0.95, "payload": {...}}]}
  ],
  "timing_ms": 2.34
}
```

### POST /collections/:name/search/multi

Execute multiple vector queries and merge results using Reciprocal Rank Fusion (RRF).

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| queries | array | Yes | Array of query vectors |
| top_k | integer | No | Results per query (default: 10) |

**Example:**
```json
{
  "queries": [
    [0.1, 0.2, 0.3, ...],
    [0.4, 0.5, 0.6, ...]
  ],
  "top_k": 10
}
```

**Response:**
```json
{
  "results": [
    {"id": "1", "score": 0.0312, "payload": {...}},
    {"id": "2", "score": 0.0298, "payload": {...}}
  ],
  "timing_ms": 3.45
}
```

**Use Cases:**
- Multi-modal search (text + image embeddings)
- Query expansion with multiple query variants
- Ensemble retrieval with different embedding models

---

### POST /collections/:name/search/multi/ids

Same fusion as `/search/multi`, but returns only ids and scores (no payloads).
Lighter on the server (skips payload hydration). Metadata filters are **not**
supported on this endpoint — use `/search/multi` for filtered fusion.

**Request Body:** identical to `/search/multi` (`vectors`, `top_k`, `strategy`,
and the fusion params), minus `filter`.

**Response:**
```json
{
  "results": [
    {"id": "1", "score": 0.0312},
    {"id": "2", "score": 0.0298}
  ]
}
```

SDK: `client.multiQuerySearchIds(collection, vectors, options)` (TypeScript).

---

## VelesQL Query

### POST /query

Execute a VelesQL query.

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| query | string | Yes | VelesQL query string |
| params | object | No | Bound parameters (e.g., vectors) |
| collection | string | Conditional | Required for top-level `MATCH ...` queries sent to `/query` |

**Example:**
```json
{
  "query": "SELECT * FROM documents WHERE vector NEAR $v AND category = 'tech' LIMIT 10",
  "params": {"v": [0.1, 0.2, 0.3, ...]}
}
```

**Response:**
```json
{
  "results": [
    {"id": 1, "score": 0.98, "payload": {"title": "AI Guide", "category": "tech"}}
  ],
  "timing_ms": 1.56,
  "took_ms": 2,
  "rows_returned": 1,
  "meta": {
    "velesql_contract_version": "3.0.0",
    "count": 1
  }
}
```

**Contract note:** top-level `MATCH` on `/query` requires `collection` in request body.  
**Default LIMIT:** a SELECT without an explicit `LIMIT` clause returns at most
10 rows (engine default). `MATCH ... RETURN` and compound queries
(UNION/INTERSECT/EXCEPT) have no implicit limit and are bounded only by the
server-wide 100 000-row ceiling — specify `LIMIT` explicitly for
predictable result sizes.  
Canonical reference: [`VELESQL_CONTRACT.md`](./VELESQL_CONTRACT.md)

### POST /aggregate

Execute aggregation-only VelesQL queries.

`/aggregate` accepts GROUP BY/HAVING/aggregate workloads and rejects row/search/graph queries.

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| query | string | Yes | Aggregation VelesQL query string |
| params | object | No | Named parameters |
| collection | string | Conditional | Optional fallback when query omits `FROM <collection>` |

**Example:**

```json
{
  "query": "SELECT category, COUNT(*) FROM documents GROUP BY category",
  "params": {}
}
```

### VelesQL Syntax Reference

| Feature | Syntax | Example |
|---------|--------|---------|
| Vector search | `vector NEAR $param` | `WHERE vector NEAR $query` |
| Distance metric | `vector NEAR COSINE $param` | `COSINE`, `EUCLIDEAN`, `DOT` |
| Equality | `field = value` | `category = 'tech'` |
| Comparison | `field > value` | `price > 100` |
| IN clause | `field IN (...)` | `status IN ('active', 'pending')` |
| BETWEEN | `field BETWEEN a AND b` | `price BETWEEN 10 AND 100` |
| LIKE | `field LIKE pattern` | `title LIKE '%rust%'` |
| NULL check | `field IS NULL` | `deleted_at IS NULL` |
| Logical | `AND`, `OR` | `a = 1 AND b = 2` |
| Full-text | `field MATCH 'query'` | `content MATCH 'rust'` |
| Limit | `LIMIT n` | `LIMIT 10` |

### VelesQL v2.0 Features

| Feature | Syntax | Example |
|---------|--------|---------|
| GROUP BY | `GROUP BY col1, col2` | `GROUP BY category` |
| HAVING | `HAVING agg > val` | `HAVING COUNT(*) > 5` |
| HAVING AND/OR | `HAVING a AND b` | `HAVING COUNT(*) > 5 AND AVG(price) > 50` |
| Aggregates | `COUNT`, `SUM`, `AVG`, `MIN`, `MAX` | `SELECT COUNT(*), AVG(price)` |
| ORDER BY multi | `ORDER BY col1, col2` | `ORDER BY category, price DESC` |
| ORDER BY similarity | `ORDER BY similarity(field, $v)` | `ORDER BY similarity(vector, $query) DESC` |
| JOIN | `JOIN table ON condition` | `JOIN prices ON prices.id = p.id` |
| LEFT/RIGHT/FULL JOIN | `LEFT JOIN table ON ...` | Parser/spec variants exist, runtime support pending |
| JOIN USING | `JOIN table USING (col)` | Parser support only, runtime support pending |
| UNION | `query1 UNION query2` | `SELECT * FROM a UNION SELECT * FROM b` |
| INTERSECT | `query1 INTERSECT query2` | Set intersection |
| EXCEPT | `query1 EXCEPT query2` | Set difference |
| USING FUSION | `USING FUSION(strategy)` | `USING FUSION(strategy='rrf', k=60)` |
| WITH options | `WITH (max_groups=N)` | `WITH (max_groups=100)` |

**VelesQL v2.0 Examples:**

```sql
-- Analytics with aggregation
SELECT category, COUNT(*), AVG(price) 
FROM products 
GROUP BY category 
HAVING COUNT(*) > 5 AND AVG(price) > 50

-- Multi-column ORDER BY with similarity
SELECT * FROM docs 
WHERE vector NEAR $query 
ORDER BY similarity(vector, $query) DESC, created_at DESC 
LIMIT 20

-- Cross-store JOIN
SELECT p.name, pr.amount 
FROM products AS p 
JOIN prices AS pr ON pr.product_id = p.id 
WHERE pr.amount < 100

-- Hybrid search with fusion (USING FUSION is a trailing clause: after LIMIT)
SELECT * FROM docs 
LIMIT 20 USING FUSION(strategy='rrf', k=60)

-- Set operations
SELECT * FROM active_users 
UNION 
SELECT * FROM archived_users
```

### POST /collections/:name/match

Execute collection-scoped graph `MATCH` queries.

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| query | string | Yes | VelesQL `MATCH ... RETURN ...` query |
| params | object | No | Named query params |
| vector | array[float] | No | Optional vector for similarity scoring |
| threshold | float | No | Similarity threshold in `[0.0, 1.0]` |

**Response:**
```json
{
  "results": [
    {
      "bindings": {"doc": "123", "author": "456"},
      "score": 0.95,
      "depth": 1,
      "projected": {"author.name": "John Doe"}
    }
  ],
  "took_ms": 15,
  "count": 1,
  "meta": {"velesql_contract_version": "3.0.0"}
}
```

---

## EXPLAIN (Query Plan)

### POST /query/explain

Analyze query execution plan without running the query.

**Request Body:**
```json
{
  "query": "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10",
  "params": {"v": [0.1, 0.2, 0.3]}
}
```

**Response:** the plan is a flat, ordered list of steps, single-sourced from
the engine's query plan (the same plan the CLI `.explain` renders):
```json
{
  "query": "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10",
  "query_type": "SELECT",
  "collection": "docs",
  "plan": [
    {
      "step": 1,
      "operation": "VectorSearch",
      "description": "ANN search using HNSW index with NEAR clause",
      "estimated_rows": null
    },
    {
      "step": 2,
      "operation": "Limit",
      "description": "Apply LIMIT 10 OFFSET 0",
      "estimated_rows": 10
    }
  ],
  "estimated_cost": {
    "uses_index": true,
    "index_name": "HNSW",
    "selectivity": 0.01,
    "complexity": "O(log n)"
  },
  "features": { "has_vector_search": true, "has_filter": false }
}
```

A non-vector `WHERE` predicate (e.g. `... vector NEAR $v AND price > 100 ...`)
adds a `Filter` step between `VectorSearch` and `Limit`, carrying
`estimated_rows` and `estimation_method` (`"histogram"`/`"cardinality"`) when
collection statistics are available.

Set `"analyze": true` to execute the query and add `actual_time_ms`,
`actual_stats`, and per-node `node_stats` to the response.

**`operation` values:**
- `FullScan` - full collection scan (no index)
- `VectorSearch` - HNSW approximate nearest neighbor
- `IndexLookup` - property index lookup
- `Filter` - metadata filtering
- `{Type}Join` - cross-store join (`InnerJoin`, `LeftJoin`, `RightJoin`, `FullJoin`)
- `GroupBy` - GROUP BY grouping
- `Aggregate` - aggregate computation (COUNT, SUM, ...)
- `Sort` - ORDER BY sort
- `Limit` - result limiting (folds OFFSET into its description)
- `MatchTraversal` - MATCH graph traversal

---

## Graph API

### GET /collections/:name/graph/nodes

List node IDs present in a graph collection.

**Response:**
```json
{
  "node_ids": [1, 2, 3],
  "count": 3
}
```

### GET /collections/:name/graph/nodes/:id/payload

Get the JSON payload attached to a graph node.

### PUT /collections/:name/graph/nodes/:id/payload

Create or replace the JSON payload attached to a graph node. Returns `204` on success.

**Request Body:**
```json
{
  "payload": {"_labels": ["Document"], "title": "AI Guide"}
}
```

### GET /collections/:name/graph/edges

List edges filtered by label (`?label=KNOWS`). Returns an `EdgesResponse`
(`edges` array of `{id, source, target, label, properties}` plus `count`).

### POST /collections/:name/graph/edges

Add edges between nodes.

**Request Body:**
```json
{
  "id": 100,
  "source": 1,
  "target": 2,
  "label": "AUTHORED_BY",
  "properties": {"year": 2026}
}
```

`id`, `source`, and `target` accept either JSON numbers or strings. Responses serialize graph IDs as strings to preserve full `u64` precision in JavaScript clients.

### DELETE /collections/:name/graph/edges/:edge_id

Remove an edge by ID. Returns `204` on success, `404` when the edge or
collection does not exist.

### GET /collections/:name/graph/edges/count

Get the total number of edges in the graph.

**Response:**
```json
{
  "count": 42
}
```

### GET /collections/:name/graph/nodes/:id/edges

List the edges of a specific node, with optional `?direction=in|out|both` and
`?label=...` query filters. Returns the same `EdgesResponse` shape as
`GET .../graph/edges`.

### POST /collections/:name/graph/traverse

Traverse the graph using BFS or DFS.

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| source | integer/string | Yes | Starting node ID |
| strategy | string | No | `bfs` or `dfs` (default: `bfs`) |
| max_depth | integer | No | Maximum traversal depth (default: 3) |
| limit | integer | No | Maximum number of results (default: 100) |
| rel_types | array[string] | No | Filter by relationship labels |

**Example:**
```json
{
  "source": 1,
  "strategy": "bfs",
  "max_depth": 2,
  "rel_types": ["AUTHORED_BY"]
}
```

**Response:**
```json
{
  "results": [
    {"target_id": "2", "depth": 1, "path": ["100"]}
  ],
  "has_more": false,
  "stats": {"visited": 1, "depth_reached": 1}
}
```

### GET /collections/:name/graph/nodes/:id/degree

Get node degree (in/out edge counts).

**Response:**
```json
{
  "node_id": "doc1",
  "in_degree": 5,
  "out_degree": 3,
  "total_degree": 8
}
```

### POST /collections/:name/graph/traverse/parallel

Parallel multi-source BFS traversal. Same response shape as
`POST .../graph/traverse`; the request takes a `sources` array instead of a
single `source`.

### GET /collections/:name/graph/traverse/stream

Stream traversal results as Server-Sent Events (SSE). Query parameters:
`start_node` (required), `algorithm` (`bfs`/`dfs`), `max_depth`, `limit`,
`relationship_types` (comma-separated). Emits `node`, periodic `stats`, `done`,
and `error` events.

### POST /collections/:name/graph/search

Search graph nodes by embedding similarity.

**Request Body:**
```json
{
  "vector": [0.1, 0.2, 0.3],
  "top_k": 10
}
```

**Response:**
```json
{
  "results": [
    {"id": "1", "score": 0.97, "payload": {"_labels": ["Document"]}}
  ]
}
```

---

## Relations and Durable TTL

Relation endpoints work on **any** collection type (vector, graph, or metadata):
edges live on the collection's embedded edge store, independently of the
payload/vector layer. They back the agent-memory `relate()`/`unrelate()` SDK
surface.

### POST /collections/:name/relations

Create a relation edge between two points. The edge ID is auto-assigned.

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| source | integer/string | Yes | Source point ID (string form for u64 > 2^53−1) |
| target | integer/string | Yes | Target point ID |
| rel_type | string | Yes | Relationship type label (e.g. `"KNOWS"`) |
| properties | object | No | Optional edge properties |

**Response (201 Created):**
```json
{
  "edge_id": "7"
}
```

### DELETE /collections/:name/relations/:edge_id

Remove a relation edge by ID. Returns `204` on success, `404` when the edge or
collection does not exist.

### GET /collections/:name/points/:id/relations

List the outgoing relation edges of a point.

**Response:**
```json
{
  "edges": [
    {"id": "7", "source": "1", "target": "2", "rel_type": "KNOWS", "properties": null}
  ],
  "count": 1
}
```

For the durable-TTL endpoint, see
[`PATCH /collections/:name/points/:id/ttl`](#patch-collectionsnamepointsidttl)
in the Points section.

---

## Property Indexes

### GET /collections/:name/indexes

List all property indexes on a collection.

### POST /collections/:name/indexes

Create a property index on a graph collection.

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| label | string | Yes | Node label to index (e.g. `Person`) |
| property | string | Yes | Property name to index (e.g. `email`) |
| index_type | string | No | `hash` (equality, O(1)) or `range` (range queries, O(log n)) |

**Response (201 Created):** index descriptor (`label`, `property`, `index_type`,
`cardinality`, `memory_bytes`).

### DELETE /collections/:name/indexes/:label/:property

Delete a property index. Returns `200` on success, `404` when the index or
collection does not exist.

---

## Maintenance

### POST /collections/:name/index/rebuild

Rebuild the HNSW index of a vector collection: reclaims memory held by
tombstoned entries and produces a fresh graph from the current vector storage.
Blocking — may take several seconds on large collections. The response includes
the number of compacted entries.

### POST /collections/:name/vacuum

Semantically equivalent to `POST .../index/rebuild`, exposed under a
maintenance-oriented name. Blocking.

### POST /collections/:name/compact

Compact the vector storage: rewrites active vectors into a contiguous layout
and reclaims disk space from deleted entries. Blocking; may involve significant
I/O on large, fragmented collections.

---

## Guardrails

### GET /guardrails

Get the current query guard-rails configuration.

**Response:**
```json
{
  "max_depth": 10,
  "max_cardinality": 100000,
  "memory_limit_bytes": 1073741824,
  "timeout_ms": 30000,
  "rate_limit_qps": 100,
  "circuit_failure_threshold": 5,
  "circuit_recovery_seconds": 30
}
```

### PUT /guardrails

Partially update the guard-rails configuration. Accepts any subset of the
fields above; returns the updated configuration.

---

## Monitoring

### GET /metrics

Prometheus exposition-format metrics (text/plain), including plan-cache
statistics. Served by default in released binaries (the server's `prometheus`
cargo feature is a default feature). Unlike `/health` and `/ready`, `/metrics`
**requires authentication** when API keys are configured.

---

## Python API

### Installation

```bash
cd crates/velesdb-python
pip install maturin
maturin develop --release
```

### Quick Reference

```python
import velesdb
import numpy as np

# Database
db = velesdb.Database("./data")

# Collection
collection = db.create_collection("docs", dimension=768, metric="cosine")
collection = db.get_collection("docs")
db.delete_collection("docs")
collections = db.list_collections()

# Tuned HNSW at creation (typed options)
from velesdb import HnswOptions
collection = db.create_collection(
    "docs_hi_recall",
    dimension=768,
    hnsw=HnswOptions(m=48, ef_construction=600),
)
# Auto-tuned for an expected dataset size:
collection = db.create_collection(
    "big",
    dimension=128,
    hnsw=HnswOptions.for_dataset_size(128, 1_000_000),
)

# Points
collection.upsert([{"id": 1, "vector": [...], "payload": {...}}])
points = collection.get([1])
collection.delete([1, 2, 3])

# Search (supports numpy arrays)
results = collection.search_request(velesdb.SearchOptions(vector=query_vector, top_k=10))
results = collection.search_request(velesdb.SearchOptions(vector=np.array([...], dtype=np.float32), top_k=10))
```
