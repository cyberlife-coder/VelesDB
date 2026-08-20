# Getting Started with VelesDB

This guide will help you get VelesDB up and running in just a few minutes.

> **Onboarding timing reference**
>
> The reproducible harness and its frozen 2026-05-01 reference run live in
> [`docs/quickstart/timing-results.md`](quickstart/timing-results.md). Those
> numbers describe that historical run; they are directionally useful, not
> current absolute timings. Run `bash scripts/dx-timing/run_all.sh` in your
> environment for fresh results.

## Fastest path: Python

If you just want to see VelesDB work, you only need Python ≥ 3.9.

```bash
pip install velesdb
curl -O https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/examples/python/hello_velesdb.py
python hello_velesdb.py
```

Expected output:

```
Query: "tech"
  score=1.000  Rust 1.89 release notes
  score=0.600  AI-generated jazz: the new wave
  score=0.000  Best ramen in Tokyo

Query: "tech + music"
  score=0.990  AI-generated jazz: the new wave
  score=0.707  Rust 1.89 release notes
  score=0.707  Miles Davis discography
```

No server, no JSON, no embedding model. The full script is [`examples/python/hello_velesdb.py`](../examples/python/hello_velesdb.py) — read it, it is ~25 lines.

### Next step: search your own text

Use VelesDB's opt-in local adapter when your input is text rather than vectors:

```bash
pip install "velesdb[embed-sentence-transformers]"
curl -O https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/examples/python/hello_velesdb_text.py
python hello_velesdb_text.py
```

The first run downloads `all-MiniLM-L6-v2`; VelesDB does not bundle an embedding model. The example derives the collection dimension from the model, stores ordinary sentences, and searches them without a server or API key.

Expected output:

```
Query: "How do I find documents with similar meaning?"
  Semantic search finds documents with similar meaning.
```

The embedding model determines vector dimension, while your similarity semantics determine the metric. Both are fixed when the collection is created; to change either, create a new collection and re-index your documents.

Want to keep going in Python? See [`examples/python/`](../examples/python/) for fusion, graph traversal, VelesQL, and hybrid queries.

The rest of this page is the **REST API** path — useful if you want to drive VelesDB from a non-Python language, or run it as a shared service.

## Prerequisites (REST API path)

- Docker (recommended) or Rust 1.90+
- curl or any HTTP client for testing

## Installation

### Using Docker (Recommended)

Pull the published multi-architecture image from GitHub Container Registry:

```bash
docker pull ghcr.io/cyberlife-coder/velesdb:latest
docker run -d \
  --name velesdb \
  -p 8080:8080 \
  -v velesdb_data:/data \
  ghcr.io/cyberlife-coder/velesdb:latest
```

#### Build the image from source

Use a local build when you need to test an unpublished checkout:

```bash
git clone https://github.com/cyberlife-coder/VelesDB.git
cd VelesDB
docker build -t velesdb .
docker run -d \
  --name velesdb \
  -p 8080:8080 \
  -v velesdb_data:/data \
  velesdb
```

### Using Cargo

If you prefer to build from source:

```bash
# Install from crates.io
cargo install velesdb-server

# Or build from source
git clone https://github.com/cyberlife-coder/VelesDB.git
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
  "status": "ok",
  "version": "5.1.0"
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
    {"id": "1", "score": 0.98, "payload": {"title": "Introduction to AI"}},
    {"id": "2", "score": 0.85, "payload": {"title": "Machine Learning Guide"}}
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

### 7. VelesQL with ORDER BY similarity()

Query with semantic ordering:

```bash
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT id, title FROM my_documents ORDER BY similarity([0.1, 0.2, ...]) LIMIT 5",
    "params": {}
  }'
```

### 8. Knowledge Graph

VelesDB supports graph relationships between vectors:

#### Add an Edge

```bash
curl -X POST http://localhost:8080/collections/my_documents/graph/edges \
  -H "Content-Type: application/json" \
  -d '{
    "id": 1,
    "source": 100,
    "target": 200,
    "label": "RELATES_TO",
    "properties": {"weight": 0.8}
  }'
```

#### Traverse the Graph

```bash
curl -X POST http://localhost:8080/collections/my_documents/graph/traverse \
  -H "Content-Type: application/json" \
  -d '{
    "source": 100,
    "strategy": "bfs",
    "max_depth": 3,
    "limit": 50
  }'
```

Response:
```json
{
  "results": [
    {"target_id": 200, "depth": 1, "path": [100, 200]},
    {"target_id": 300, "depth": 2, "path": [100, 200, 300]}
  ],
  "stats": {"visited": 2, "depth_reached": 2}
}
```

### 9. Cross-Collection MATCH

Combine graph traversal with data from multiple collections using `@collection`:

```bash
# Create a graph collection with edges
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "catalog", "type": "graph", "dimension": 4, "metric": "cosine"}'

# Create a metadata collection with pricing
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "pricing", "type": "metadata"}'

# Query: traverse catalog graph, enrich with pricing data
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "MATCH (p:Product)-[:STORED_IN]->(w:Warehouse@pricing) RETURN p.name, w.price LIMIT 10",
    "collection": "catalog",
    "params": {}
  }'
```

> **Full guide:** [Graph Patterns Guide](guides/GRAPH_PATTERNS.md#cross-collection-match-collection)

## Next Steps

- Read the [API Reference](reference/api-reference.md) for complete endpoint documentation
- Read the [VelesQL Specification](VELESQL_SPEC.md) for query language reference
- Learn about [Configuration](guides/CONFIGURATION.md) options
- Explore [Architecture](reference/ARCHITECTURE.md) to understand VelesDB internals
- Check out [Examples](../examples/) for real-world use cases
- Follow the [Tauri RAG Tutorial](tutorials/tauri-rag-app/) to build a desktop AI app

## Getting Help

- [**GitHub Issues**](https://github.com/cyberlife-coder/VelesDB/issues): Report bugs or request features
- [**GitHub Discussions**](https://github.com/cyberlife-coder/VelesDB/discussions): Ask questions and share ideas

---
Last updated: 2026-08-20 · Applies to: velesdb-core 5.1.0
