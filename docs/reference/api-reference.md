# VelesDB API Reference

Complete REST API documentation for VelesDB.

## Base URL

```
http://localhost:8080
```

---

## Health Check

### GET /health

Check server health status.

**Response:**
```json
{
  "status": "healthy",
  "version": "0.1.0"
}
```

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
| dimension | integer | Yes | Vector dimension (e.g., 768) |
| metric | string | No | Distance metric (see table below) |

**Distance Metrics:**

| Metric | Description | Best For |
|--------|-------------|----------|
| `cosine` | Cosine similarity (default) | Text embeddings, semantic search |
| `euclidean` | L2 distance | Spatial data, image features |
| `dotproduct` | Inner product (MIPS) | Recommendations, ranking |
| `hamming` | Bit difference count | Binary embeddings, fingerprints |
| `jaccard` | Set intersection/union | Tags, preferences, document similarity |

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

### DELETE /collections/:name

Delete a collection and all its data.

**Response:**
```json
{
  "message": "Collection deleted",
  "name": "documents"
}
```

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

---

## Search

### POST /collections/:name/search

Search for similar vectors.

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| vector | array[float] | Yes | Query vector |
| top_k | integer | No | Number of results (default: 10) |

**Example:**
```json
{
  "vector": [0.15, 0.25, 0.35, ...],
  "top_k": 5
}
```

**Response:**
```json
{
  "results": [
    {
      "id": 1,
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
      "id": 1,
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
      "id": 1,
      "score": 0.0312,
      "payload": {"content": "Rust programming guide"}
    }
  ],
  "timing_ms": 2.45
}
```

---

## Error Responses

All errors return a JSON object with an `error` field:

```json
{
  "error": "Collection 'documents' not found"
}
```

### HTTP Status Codes

| Code | Description |
|------|-------------|
| 200 | Success |
| 201 | Created |
| 400 | Bad Request (invalid input) |
| 404 | Not Found |
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
    {"results": [{"id": 1, "score": 0.98, "payload": {...}}]},
    {"results": [{"id": 2, "score": 0.95, "payload": {...}}]}
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
    {"id": 1, "score": 0.0312, "payload": {...}},
    {"id": 2, "score": 0.0298, "payload": {...}}
  ],
  "timing_ms": 3.45
}
```

**Use Cases:**
- Multi-modal search (text + image embeddings)
- Query expansion with multiple query variants
- Ensemble retrieval with different embedding models

---

## VelesQL Query

### POST /query

Execute a VelesQL query.

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| query | string | Yes | VelesQL query string |
| params | object | No | Bound parameters (e.g., vectors) |

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
  "rows_returned": 1
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

# Points
collection.upsert([{"id": 1, "vector": [...], "payload": {...}}])
point = collection.get(1)
collection.delete([1, 2, 3])

# Search (supports numpy arrays)
results = collection.search(query_vector, top_k=10)
results = collection.search(np.array([...], dtype=np.float32), top_k=10)
```
