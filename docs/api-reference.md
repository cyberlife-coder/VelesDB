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
| metric | string | No | Distance metric: `cosine` (default), `euclidean`, `dotproduct` |

**Example:**
```json
{
  "name": "documents",
  "dimension": 768,
  "metric": "cosine"
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
