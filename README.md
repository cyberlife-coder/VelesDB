<p align="center">
  <img src="docs/assets/velesdb-logo.svg" alt="VelesDB Logo" width="200"/>
</p>

<h1 align="center">VelesDB</h1>

<p align="center">
  <strong>The Open-Source Vector Database for AI Applications</strong><br/>
  <em>Fast • Simple • Production-Ready</em>
</p>

<p align="center">
  <a href="https://github.com/cyberlife-coder/VelesDB/actions"><img src="https://img.shields.io/github/actions/workflow/status/cyberlife-coder/VelesDB/ci.yml?branch=main&style=flat-square" alt="Build Status"></a>
  <a href="https://github.com/cyberlife-coder/VelesDB/releases"><img src="https://img.shields.io/github/v/release/cyberlife-coder/VelesDB?style=flat-square&color=green" alt="Release"></a>
  <a href="https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg?style=flat-square" alt="License"></a>
  <a href="https://github.com/cyberlife-coder/VelesDB"><img src="https://img.shields.io/github/stars/cyberlife-coder/VelesDB?style=flat-square" alt="GitHub Stars"></a>
</p>

<p align="center">
  <a href="#-quick-start">Quick Start</a> •
  <a href="#-features">Features</a> •
  <a href="#-api-reference">API Reference</a> •
  <a href="#-velesql-query-language">VelesQL</a> •
  <a href="#-performance">Performance</a> •
  <a href="#-contributing">Contributing</a>
</p>

---

## 🎯 What is VelesDB?

VelesDB is a **high-performance vector database** built entirely in Rust. It's designed for AI applications that need fast similarity search — semantic search, RAG pipelines, recommendation engines, and more.

### Why Choose VelesDB?

| Feature | VelesDB | Others |
|---------|---------|--------|
| **Language** | 🦀 Pure Rust | C++/Go |
| **Setup** | Single binary | Complex deps |
| **Query Language** | SQL-like (VelesQL) | Custom DSL |
| **Memory** | 4x reduction (SQ8) | Varies |
| **Latency** | Sub-millisecond | ~1-5ms |

### 🐺 Why "Veles"?

**Veles** (Велес) is a major Slavic deity — the god of wisdom, magic, and knowledge. As the guardian of sacred knowledge and keeper of memories, Veles embodies what a vector database does: storing, organizing, and retrieving the essence of information.

Just as Veles bridges the earthly and mystical realms, VelesDB bridges raw data and meaningful AI-powered insights.

---

## ✨ Features

- 🚀 **Built in Rust** — Memory-safe, fast, and reliable
- ⚡ **Blazing Fast Search** — SIMD-optimized similarity (2.3x faster than baseline)
- 🧠 **SQ8 Quantization** — 4x memory reduction with >95% recall accuracy
- 🔍 **Metadata Filtering** — Filter results by payload (eq, gt, lt, in, contains...)
- 💾 **Persistent Storage** — HNSW index with WAL for durability
- 🔌 **Simple REST API** — Easy integration with any language
- 📦 **Single Binary** — No dependencies, easy deployment
- 🐳 **Docker Ready** — Run anywhere in seconds

---

## 🚀 Quick Start

### Option 1: Pre-built Binary (Recommended)

```bash
# Download latest release
curl -L https://github.com/cyberlife-coder/VelesDB/releases/latest/download/velesdb-server-linux-amd64 -o velesdb-server
chmod +x velesdb-server

# Run the server
./velesdb-server --data-dir ./data --port 8080
```

### Option 2: Using Cargo

```bash
# Install from source
cargo install --git https://github.com/cyberlife-coder/VelesDB velesdb-server

# Run the server
velesdb-server --data-dir ./data --port 8080
```

### Option 3: Build from Source

```bash
# Clone the repository
git clone https://github.com/cyberlife-coder/VelesDB.git
cd VelesDB

# Build in release mode
cargo build --release --package velesdb-server

# Run
./target/release/velesdb-server --data-dir ./data --port 8080
```

### Option 4: Docker

```bash
# Run with Docker
docker run -d -p 8080:8080 -v velesdb_data:/data ghcr.io/cyberlife-coder/velesdb:latest

# Test the connection
curl http://localhost:8080/health
```

### Verify Installation

```bash
curl http://localhost:8080/health
# {"status":"healthy","version":"0.1.0"}
```

---

## 📖 Your First Vector Search

```bash
# 1. Create a collection
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "documents", "dimension": 4, "metric": "cosine"}'

# 2. Insert vectors with metadata
curl -X POST http://localhost:8080/collections/documents/points \
  -H "Content-Type: application/json" \
  -d '{
    "points": [
      {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "AI Introduction", "category": "tech"}},
      {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"title": "ML Basics", "category": "tech"}},
      {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"title": "History of Computing", "category": "history"}}
    ]
  }'

# 3. Search for similar vectors
curl -X POST http://localhost:8080/collections/documents/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [0.9, 0.1, 0.0, 0.0], "top_k": 2}'

# 4. Or use VelesQL (SQL-like queries)
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT * FROM documents WHERE vector NEAR $v AND category = '\''tech'\'' LIMIT 5",
    "params": {"v": [0.9, 0.1, 0.0, 0.0]}
  }'
```

---

## 🔌 API Reference

### Collections

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/collections` | `GET` | List all collections |
| `/collections` | `POST` | Create a collection |
| `/collections/{name}` | `GET` | Get collection info |
| `/collections/{name}` | `DELETE` | Delete a collection |

### Points

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/collections/{name}/points` | `POST` | Upsert points |
| `/collections/{name}/points/{id}` | `GET` | Get a point by ID |
| `/collections/{name}/points/{id}` | `DELETE` | Delete a point |

### Search

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/collections/{name}/search` | `POST` | Vector similarity search |
| `/collections/{name}/search/batch` | `POST` | Batch search (multiple queries) |
| `/query` | `POST` | Execute VelesQL query |

### Health

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | `GET` | Health check |

### Request/Response Examples

<details>
<summary><b>Create Collection</b></summary>

```bash
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my_vectors",
    "dimension": 768,
    "metric": "cosine"  # Options: cosine, euclidean, dot
  }'
```

**Response:**
```json
{"message": "Collection created", "name": "my_vectors"}
```
</details>

<details>
<summary><b>Upsert Points</b></summary>

```bash
curl -X POST http://localhost:8080/collections/my_vectors/points \
  -H "Content-Type: application/json" \
  -d '{
    "points": [
      {
        "id": 1,
        "vector": [0.1, 0.2, 0.3, ...],
        "payload": {"title": "Document 1", "tags": ["ai", "ml"]}
      }
    ]
  }'
```

**Response:**
```json
{"message": "Points upserted", "count": 1}
```
</details>

<details>
<summary><b>Vector Search</b></summary>

```bash
curl -X POST http://localhost:8080/collections/my_vectors/search \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.1, 0.2, 0.3, ...],
    "top_k": 10
  }'
```

**Response:**
```json
{
  "results": [
    {"id": 1, "score": 0.95, "payload": {"title": "Document 1"}},
    {"id": 42, "score": 0.87, "payload": {"title": "Document 42"}}
  ]
}
```
</details>

<details>
<summary><b>Batch Search</b></summary>

```bash
curl -X POST http://localhost:8080/collections/my_vectors/search/batch \
  -H "Content-Type: application/json" \
  -d '{
    "searches": [
      {"vector": [0.1, 0.2, ...], "top_k": 5},
      {"vector": [0.3, 0.4, ...], "top_k": 5}
    ]
  }'
```

**Response:**
```json
{
  "results": [
    {"results": [{"id": 1, "score": 0.95, "payload": {...}}]},
    {"results": [{"id": 2, "score": 0.89, "payload": {...}}]}
  ],
  "timing_ms": 1.23
}
```
</details>

<details>
<summary><b>VelesQL Query</b></summary>

```bash
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT * FROM my_vectors WHERE vector NEAR $v LIMIT 10",
    "params": {"v": [0.1, 0.2, 0.3, ...]}
  }'
```

**Response:**
```json
{
  "results": [
    {"id": 1, "score": 0.95, "payload": {"title": "Document 1"}}
  ],
  "timing_ms": 2.34,
  "rows_returned": 1
}
```
</details>

---

## ⚡ Performance

VelesDB is built for speed. All critical paths are SIMD-optimized.

| Operation | Time (768d vectors) | Throughput |
|-----------|---------------------|------------|
| Cosine Similarity | ~325 ns | **3M ops/sec** |
| Euclidean Distance | ~155 ns | **6.5M ops/sec** |
| Dot Product | ~140 ns | **7M ops/sec** |
| Metadata Filter | ~13 µs/1k items | **77k batches/sec** |

### Memory Efficiency with SQ8

| Configuration | RAM per 1M vectors (768d) |
|---------------|---------------------------|
| Full Precision (f32) | **3 GB** |
| SQ8 Quantized (u8) | **0.75 GB** (4x reduction) |

---

## 🔍 Metadata Filtering

Filter search results by payload attributes:

```rust
// Filter: category = "tech" AND price > 100
let filter = Filter::new(Condition::and(vec![
    Condition::eq("category", "tech"),
    Condition::gt("price", 100),
]));
```

Supported operators: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `in`, `contains`, `is_null`, `and`, `or`, `not`

---

## 📝 VelesQL Query Language

VelesQL is a **SQL-like query language** designed specifically for vector search. If you know SQL, you already know VelesQL.

### Basic Syntax

```sql
SELECT * FROM documents 
WHERE vector NEAR $query_vector
  AND category = 'tech'
  AND price > 100
LIMIT 10;
```

### REST API Usage

```bash
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT * FROM documents WHERE vector NEAR $v AND category = '\''tech'\'' LIMIT 10",
    "params": {"v": [0.1, 0.2, 0.3, ...]}
  }'
```

### Supported Features

| Feature | Example | Description |
|---------|---------|-------------|
| **Vector search** | `vector NEAR $v` | Find similar vectors |
| **Distance metrics** | `vector NEAR COSINE $v` | `COSINE`, `EUCLIDEAN`, `DOT` |
| **Comparisons** | `price > 100` | `=`, `!=`, `>`, `<`, `>=`, `<=` |
| **IN clause** | `category IN ('tech', 'ai')` | Match any value in list |
| **BETWEEN** | `price BETWEEN 10 AND 100` | Range queries |
| **LIKE** | `title LIKE '%rust%'` | Pattern matching |
| **NULL checks** | `deleted_at IS NULL` | `IS NULL`, `IS NOT NULL` |
| **Logical ops** | `A AND B OR C` | With proper precedence |
| **Parameters** | `$param_name` | Safe, injection-free binding |
| **Nested fields** | `metadata.author = 'John'` | Dot notation for JSON |

### Parser Performance

| Query Type | Time | Throughput |
|------------|------|------------|
| Simple SELECT | ~755 ns | **1.3M queries/sec** |
| Vector search | ~1.2 µs | **800K queries/sec** |
| Complex (multi-filter) | ~4.8 µs | **200K queries/sec** |

---

## ⚙️ Configuration

### Server Options

```bash
velesdb-server [OPTIONS]

Options:
  -d, --data-dir <PATH>   Data directory [default: ./data] [env: VELESDB_DATA_DIR]
      --host <HOST>       Host to bind [default: 0.0.0.0] [env: VELESDB_HOST]
  -p, --port <PORT>       Port to listen on [default: 8080] [env: VELESDB_PORT]
  -h, --help              Print help
  -V, --version           Print version
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `VELESDB_DATA_DIR` | Data storage directory | `./data` |
| `VELESDB_HOST` | Server bind address | `0.0.0.0` |
| `VELESDB_PORT` | Server port | `8080` |
| `RUST_LOG` | Log level | `info` |

### Example: Production Setup

```bash
export VELESDB_DATA_DIR=/var/lib/velesdb
export VELESDB_PORT=6333
export RUST_LOG=info,tower_http=debug

velesdb-server
```

---

## 🏗️ Use Cases

### Semantic Search
Build search experiences that understand meaning, not just keywords.
```sql
SELECT * FROM articles WHERE vector NEAR $query LIMIT 10
```

### RAG Applications
Enhance LLM applications with relevant context retrieval.
```sql
SELECT * FROM knowledge_base 
WHERE vector NEAR $question 
  AND source = 'documentation'
LIMIT 5
```

### Recommendations
Power "similar items" and personalized recommendations.
```sql
SELECT * FROM products 
WHERE vector NEAR $user_embedding 
  AND category = 'electronics'
  AND price < 500
LIMIT 20
```

### Image Search
Find visually similar images using embedding vectors.
```sql
SELECT * FROM images WHERE vector NEAR $image_embedding LIMIT 10
```

---

## 🔧 Using as a Rust Library

Add to your `Cargo.toml`:

```toml
[dependencies]
velesdb-core = "0.1"
```

### Example

```rust
use velesdb_core::{Database, DistanceMetric, Point};

fn main() -> anyhow::Result<()> {
    // Open database
    let db = Database::open("./my_data")?;
    
    // Create collection
    db.create_collection("documents", 768, DistanceMetric::Cosine)?;
    
    // Get collection and insert points
    let collection = db.get_collection("documents").unwrap();
    collection.upsert(vec![
        Point::new(1, vec![0.1, 0.2, ...], Some(json!({"title": "Doc 1"}))),
    ])?;
    
    // Search
    let results = collection.search(&query_vector, 10)?;
    
    Ok(())
}
```

---

## 🤝 Contributing

We welcome contributions! Here's how to get started:

### Development Setup

```bash
# Clone the repo
git clone https://github.com/cyberlife-coder/VelesDB.git
cd VelesDB

# Run tests
cargo test --all-features

# Run with checks (before committing)
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
```

### Project Structure

```
VelesDB/
├── crates/
│   ├── velesdb-core/     # Core engine library
│   │   ├── src/
│   │   │   ├── collection/   # Collection management
│   │   │   ├── index/        # HNSW index
│   │   │   ├── storage/      # Persistence layer
│   │   │   ├── velesql/      # Query language parser
│   │   │   └── simd/         # SIMD optimizations
│   │   └── tests/
│   └── velesdb-server/   # REST API server
│       ├── src/
│       └── tests/
├── benches/              # Benchmarks
└── docs/                 # Documentation
```

### Good First Issues

Looking for a place to start? Check out issues labeled [`good first issue`](https://github.com/cyberlife-coder/VelesDB/labels/good%20first%20issue).

---

## 📊 Roadmap

### v0.1.0 ✅ (Current)
- [x] HNSW vector index
- [x] REST API (11 endpoints)
- [x] VelesQL query language
- [x] SIMD-optimized distance calculations
- [x] SQ8 quantization

### v0.2.0 (Planned)
- [ ] Python bindings (PyO3)
- [ ] CLI / REPL for VelesQL
- [ ] Rate limiting
- [ ] OpenAPI/Swagger docs

### v0.3.0 (Future)
- [ ] Hybrid search (BM25 + vector)
- [ ] Distributed mode
- [ ] RBAC / Multi-tenancy

---

## 📜 License

VelesDB is licensed under the [Apache License 2.0](LICENSE).

---

<p align="center">
  <strong>Built with ❤️ and 🦀 Rust</strong><br/>
  <a href="https://github.com/cyberlife-coder/VelesDB">GitHub</a> •
  <a href="https://github.com/cyberlife-coder/VelesDB/issues">Issues</a> •
  <a href="https://github.com/cyberlife-coder/VelesDB/releases">Releases</a>
</p>
