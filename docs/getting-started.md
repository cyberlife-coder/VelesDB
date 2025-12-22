# Getting Started with VelesDB

This guide will help you get VelesDB up and running in just a few minutes.

## Prerequisites

- Docker (recommended) or Rust 1.75+
- curl or any HTTP client for testing

## Installation

### Using Docker (Recommended)

The easiest way to get started is with Docker:

```bash
docker run -d \
  --name velesdb \
  -p 8080:8080 \
  -v velesdb_data:/data \
  velesdb/velesdb:latest
```

### Using Cargo

If you prefer to build from source:

```bash
# Install from crates.io
cargo install velesdb-server

# Or build from source
git clone https://github.com/YOUR_USERNAME/velesdb.git
cd velesdb
cargo build --release
./target/release/velesdb-server
```

## Verify Installation

Check that VelesDB is running:

```bash
curl http://localhost:8080/health
```

Expected response:
```json
{
  "status": "healthy",
  "version": "0.1.0"
}
```

## Quick Tutorial

### 1. Create a Collection

A collection is a container for vectors with the same dimension:

```bash
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my_documents",
    "dimension": 384,
    "metric": "cosine"
  }'
```

### 2. Insert Vectors

Add some vectors with metadata:

```bash
curl -X POST http://localhost:8080/collections/my_documents/points \
  -H "Content-Type: application/json" \
  -d '{
    "points": [
      {
        "id": 1,
        "vector": [0.1, 0.2, 0.3, ...],
        "payload": {"title": "Introduction to AI", "category": "tech"}
      },
      {
        "id": 2,
        "vector": [0.4, 0.5, 0.6, ...],
        "payload": {"title": "Machine Learning Guide", "category": "tech"}
      }
    ]
  }'
```

### 3. Search for Similar Vectors

Find the most similar vectors to a query:

```bash
curl -X POST http://localhost:8080/collections/my_documents/search \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.15, 0.25, 0.35, ...],
    "top_k": 5
  }'
```

Response:
```json
{
  "results": [
    {"id": 1, "score": 0.98, "payload": {"title": "Introduction to AI"}},
    {"id": 2, "score": 0.85, "payload": {"title": "Machine Learning Guide"}}
  ]
}
```

### 4. Full-Text Search (BM25)

Search documents by text content:

```bash
curl -X POST http://localhost:8080/collections/my_documents/search/text \
  -H "Content-Type: application/json" \
  -d '{
    "query": "machine learning",
    "top_k": 5
  }'
```

### 5. Hybrid Search (Vector + Text)

Combine vector similarity with text relevance:

```bash
curl -X POST http://localhost:8080/collections/my_documents/search/hybrid \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.15, 0.25, 0.35, ...],
    "query": "machine learning",
    "top_k": 5,
    "vector_weight": 0.7
  }'
```

### 6. VelesQL with MATCH

Use SQL-like syntax for full-text search:

```bash
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT * FROM my_documents WHERE title MATCH '\''AI'\'' LIMIT 10",
    "params": {}
  }'
```

## Next Steps

- Read the [API Reference](api-reference.md) for complete endpoint documentation
- Learn about [Configuration](configuration.md) options
- Explore [Architecture](architecture.md) to understand VelesDB internals
- Check out [Examples](../examples/) for real-world use cases

## Getting Help

- **Discord**: Join our community for real-time support
- **GitHub Issues**: Report bugs or request features
- **GitHub Discussions**: Ask questions and share ideas
