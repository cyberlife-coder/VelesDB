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
| LEFT/RIGHT/FULL JOIN | `LEFT JOIN table ON ...` | `LEFT JOIN customers ON ...` |
| JOIN USING | `JOIN table USING (col)` | `JOIN customers USING (customer_id)` |
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

-- Hybrid search with fusion
SELECT * FROM docs 
USING FUSION(strategy='rrf', k=60) 
LIMIT 20

-- Set operations
SELECT * FROM active_users 
UNION 
SELECT * FROM archived_users
```

---

## EXPLAIN (Query Plan)

### POST /query with EXPLAIN

Analyze query execution plan without running the query.

**Request Body:**
```json
{
  "query": "EXPLAIN SELECT * FROM docs WHERE vector NEAR $v LIMIT 10",
  "params": {"v": [0.1, 0.2, 0.3]}
}
```

**Response:**
```json
{
  "plan": {
    "type": "VectorSearch",
    "collection": "docs",
    "metric": "cosine",
    "limit": 10,
    "estimated_cost": 0.05,
    "children": []
  },
  "timing_ms": 0.12
}
```

**Plan Node Types:**
- `TableScan` - Full collection scan
- `VectorSearch` - HNSW approximate nearest neighbor
- `IndexLookup` - Property index lookup
- `Filter` - Post-search filtering
- `Limit` - Result limiting
- `Offset` - Result offsetting

---

## Graph API

### POST /collections/:name/graph/nodes

Add nodes to the knowledge graph.

**Request Body:**
```json
{
  "nodes": [
    {"id": "doc1", "label": "Document", "properties": {"title": "AI Guide"}}
  ]
}
```

### POST /collections/:name/graph/edges

Add edges between nodes.

**Request Body:**
```json
{
  "edges": [
    {"source": "doc1", "target": "author1", "label": "AUTHORED_BY", "properties": {}}
  ]
}
```

### POST /collections/:name/graph/traverse

Traverse the graph using BFS or DFS.

**Request Body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| start_node | string | Yes | Starting node ID |
| direction | string | No | `outgoing`, `incoming`, or `both` (default) |
| max_depth | integer | No | Maximum traversal depth (default: 3) |
| edge_filter | string | No | Filter edges by label |

**Example:**
```json
{
  "start_node": "doc1",
  "direction": "outgoing",
  "max_depth": 2,
  "edge_filter": "AUTHORED_BY"
}
```

**Response:**
```json
{
  "nodes": [
    {"id": "doc1", "label": "Document", "depth": 0},
    {"id": "author1", "label": "Person", "depth": 1}
  ],
  "edges": [
    {"source": "doc1", "target": "author1", "label": "AUTHORED_BY"}
  ]
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
