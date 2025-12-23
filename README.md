<p align="center">
  <img src="docs/assets/velesdb-logo.svg" alt="VelesDB Logo" width="200"/>
</p>

<h1 align="center">VelesDB</h1>

<p align="center">
  <strong>The Open-Source Vector Database for AI Applications</strong><br/>
  <em>Fast • Simple • Production-Ready</em>
</p>

<p align="center">
  <a href="https://crates.io/crates/velesdb-core"><img src="https://img.shields.io/crates/v/velesdb-core.svg?style=flat-square" alt="Crates.io"></a>
  <a href="https://pypi.org/project/velesdb/"><img src="https://img.shields.io/pypi/v/velesdb?style=flat-square" alt="PyPI"></a>
  <a href="https://docs.rs/velesdb-core"><img src="https://img.shields.io/docsrs/velesdb-core?style=flat-square" alt="docs.rs"></a>
  <a href="https://deepwiki.com/cyberlife-coder/VelesDB/"><img src="https://img.shields.io/badge/docs-DeepWiki-blue?style=flat-square" alt="DeepWiki"></a>
  <a href="https://github.com/cyberlife-coder/VelesDB/actions"><img src="https://img.shields.io/github/actions/workflow/status/cyberlife-coder/VelesDB/ci.yml?branch=main&style=flat-square" alt="Build Status"></a>
  <a href="https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE"><img src="https://img.shields.io/crates/l/velesdb-core?style=flat-square" alt="License"></a>
  <a href="https://github.com/cyberlife-coder/VelesDB"><img src="https://img.shields.io/github/stars/cyberlife-coder/VelesDB?style=flat-square" alt="GitHub Stars"></a>
</p>

<p align="center">
  <a href="#-quick-start">Quick Start</a> •
  <a href="#-features">Features</a> •
  <a href="#-api-reference">API Reference</a> •
  <a href="#-velesql-query-language">VelesQL</a> •
  <a href="#-performance">Performance</a> •
  <a href="https://deepwiki.com/cyberlife-coder/VelesDB/">📖 Docs</a> •
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
| **Latency** | Sub-millisecond (~39-81ns) | ~1-5ms |

### 🐺 Why "Veles"?

**Veles** (Велес) is a major Slavic deity — the god of wisdom, magic, and knowledge. As the guardian of sacred knowledge and keeper of memories, Veles embodies what a vector database does: storing, organizing, and retrieving the essence of information.

Just as Veles bridges the earthly and mystical realms, VelesDB bridges raw data and meaningful AI-powered insights.

---

## ✨ Features

- 🚀 **Built in Rust** — Memory-safe, fast, and reliable
- ⚡ **Blazing Fast Search** — SIMD-optimized similarity (4x faster with explicit SIMD)
- 🎯 **5 Distance Metrics** — Cosine, Euclidean, Dot Product, **Hamming**, **Jaccard**
- 🗂️ **ColumnStore Filtering** — 122x faster than JSON filtering at scale
- 🧠 **SQ8 Quantization** — 4x memory reduction with >95% recall accuracy
- 🔍 **Metadata Filtering** — Filter results by payload (eq, gt, lt, in, contains...)
- 📝 **BM25 Full-Text Search** — Hybrid search combining vectors + text relevance
- 💾 **Persistent Storage** — HNSW index with WAL for durability
- 🔌 **Simple REST API** — Easy integration with any language
- 📦 **Single Binary** — No dependencies, easy deployment
- 🐳 **Docker Ready** — Run anywhere in seconds

### 📐 Distance Metrics

VelesDB supports **5 distance metrics** for different use cases:

| Metric | Best For | Use Case |
|--------|----------|----------|
| **Cosine** | Text embeddings | Semantic search, RAG pipelines |
| **Euclidean** | Spatial data | Geolocation, image features |
| **Dot Product** | MIPS | Recommendation systems |
| **Hamming** | Binary vectors | Image hashing, fingerprints, duplicate detection |
| **Jaccard** | Sets/Tags | Recommendations, document similarity |

#### 🔥 Binary Embeddings with Hamming

For **ultra-fast similarity search** on binary data:

```bash
# Create collection with Hamming metric
curl -X POST http://localhost:8080/collections \
  -d '{"name": "fingerprints", "dimension": 64, "metric": "hamming"}'

# Insert binary vectors (values > 0.5 = 1, else = 0)
curl -X POST http://localhost:8080/collections/fingerprints/points \
  -d '{"points": [{"id": 1, "vector": [1, 0, 1, 0, ...]}]}'
```

**Why Hamming?** Compare 64 bits in a single CPU operation (XOR + popcount) — orders of magnitude faster than floating-point comparisons.

#### 🏷️ Set Similarity with Jaccard

For **recommendation systems** based on shared attributes:

```bash
# Create collection with Jaccard metric  
curl -X POST http://localhost:8080/collections \
  -d '{"name": "user_tags", "dimension": 100, "metric": "jaccard"}'

# Insert user preferences as binary vectors
# [1,1,0,0,...] = user likes categories 0,1 but not 2,3
```

**Why Jaccard?** Measures overlap between sets — perfect for "users who liked X also liked Y".

---

## 🚀 Quick Start

### Option 1: One-liner Install (Recommended)

**Linux / macOS:**
```bash
curl -fsSL https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/scripts/install.sh | bash
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/scripts/install.ps1 | iex
```

Then start using VelesDB:
```bash
# Start the server
velesdb-server

# Or use the interactive CLI
velesdb
```

### Option 2: Python (pip)

```bash
pip install velesdb
```

```python
import velesdb

db = velesdb.Database("./my_vectors")
collection = db.create_collection("docs", dimension=768, metric="cosine")
collection.upsert([{"id": 1, "vector": [...], "payload": {"title": "Hello"}}])
results = collection.search([...], top_k=10)
```

### Option 3: Rust (cargo)

```bash
# Add to Cargo.toml
cargo add velesdb-core

# Or install CLI/Server
cargo install velesdb-cli
cargo install velesdb-server
```

### Option 4: Docker

```bash
docker run -d -p 8080:8080 -v velesdb_data:/data ghcr.io/cyberlife-coder/velesdb:latest
```

### Verify Installation

```bash
curl http://localhost:8080/health
# {"status":"healthy","version":"0.1.1"}
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
| `/collections/{name}/search/text` | `POST` | BM25 full-text search |
| `/collections/{name}/search/hybrid` | `POST` | Hybrid vector + text search |
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

### Core Vector Operations (768d)

| Operation | Time | Throughput | Implementation |
|-----------|------|------------|----------------|
| **Dot Product** | **~39 ns** | **26M ops/sec** | AVX2 f32x8 FMA |
| **Euclidean** | **~49 ns** | **20M ops/sec** | AVX2 f32x8 FMA |
| **Hamming (Binary)** | **~6 ns** | **164M ops/sec** | POPCNT + SIMD |
| **Cosine** | **~81 ns** | **12M ops/sec** | Single-pass SIMD Fused |

### Query Performance

- **Metadata Filtering (ColumnStore)**: ~122x faster than JSON at 100k items
- **VelesQL Parsing**: ~1.9M queries/sec
- **Index Latency**: Sub-millisecond (p95) for <1M vectors

> See full results in [docs/BENCHMARKS.md](docs/BENCHMARKS.md)

---

## 🆚 Comparison vs Competitors

| Feature | 🐺 VelesDB | 🦁 LanceDB | 🦀 Qdrant | 🐿️ Pinecone | 🐘 pgvector |
|---------|-----------|------------|-----------|-------------|-------------|
| **Core Language** | **Rust** | Rust | Rust | C++/Go (Proprietary) | C |
| **Deployment** | **Single Binary** | Embedded/Cloud | Docker/Cloud | SaaS Only | PostgreSQL Extension |
| **Vector Types** | **Float32, Binary, Set** | Float32, Float16 | Float32, Binary | Float32 | Float32, Float16 |
| **Query Language** | **SQL-like (VelesQL)** | Python SDK/SQL | JSON DSL | JSON/SDK | SQL |
| **Full Text Search** | ✅ BM25 + Hybrid | ✅ Hybrid | ✅ | ❌ | ✅ (via Postgres) |
| **Quantization** | **SQ8 (Scalar)** | IVF-PQ, RaBitQ | Binary/SQ | Proprietary | IVFFlat/HNSW |
| **License** | **BSL-1.1** | Apache 2.0 | Apache 2.0 | Closed | PostgreSQL |
| **Best For** | **Embedded / Edge / Speed** | Multimodal / Lakehouse | Scale / Cloud | Managed SaaS | Relational + Vector |

### 🎯 Why Choose VelesDB?

#### ⚡ Microsecond Latency
- **~39-81ns** per vector operation (768D) vs milliseconds for competitors
- **122x faster filtering** with ColumnStore (RoaringBitmap) vs JSON-based filtering
- **SIMD-optimized** distance calculations (AVX2/SSE4.2)

#### 📝 SQL-Native Queries (VelesQL)
```sql
-- Clean, familiar syntax - no JSON DSL to learn
SELECT * FROM docs WHERE vector NEAR $v AND category = 'tech' LIMIT 10
```

#### 📦 Zero-Config Simplicity
- **Single binary** (~15MB) — no Docker, no dependencies
- **13k lines of code** vs 50k+ (LanceDB) — less complexity, fewer bugs
- Runs on **Edge, Desktop, Server, WASM** (browser-ready)

#### 🔧 Unique Features
| Feature | VelesDB | LanceDB | Others |
|---------|---------|---------|--------|
| **Jaccard Similarity** | ✅ Native | ❌ | ❌ |
| **Binary Quantization (1-bit)** | ✅ 32x compression | ❌ | Limited |
| **WASM/Browser Support** | ✅ | ❌ | ❌ |
| **Tauri Desktop Plugin** | ✅ | ❌ | ❌ |
| **REST API Built-in** | ✅ | ❌ (embedded only) | Varies |

#### 🎯 Best For These Use Cases
- **Edge/IoT** — Memory-constrained devices with latency requirements
- **Desktop Apps** — Tauri/Electron AI-powered applications
- **Browser/WASM** — Client-side vector search
- **RAG Pipelines** — Fast semantic retrieval for LLM context
- **Real-time Search** — Sub-millisecond response requirements


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
| **Full-text search** | `content MATCH 'query'` | BM25 text search |
| **Hybrid search** | `NEAR $v AND MATCH 'q'` | Vector + text fusion |

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

## 🐍 Python Bindings

VelesDB provides native Python bindings via PyO3.

### Installation

```bash
# From source (requires Rust)
cd crates/velesdb-python
pip install maturin
maturin develop --release
```

### Basic Usage

```python
import velesdb
import numpy as np

# Open database
db = velesdb.Database("./my_data")

# Create collection
collection = db.create_collection("documents", dimension=768, metric="cosine")

# Insert with NumPy arrays
vectors = np.random.rand(100, 768).astype(np.float32)
points = [{"id": i, "vector": vectors[i], "payload": {"title": f"Doc {i}"}} for i in range(100)]
collection.upsert(points)

# Search
query = np.random.rand(768).astype(np.float32)
results = collection.search(query, top_k=10)
```

### LangChain Integration

```python
from langchain_velesdb import VelesDBVectorStore
from langchain_openai import OpenAIEmbeddings

# Create vector store
vectorstore = VelesDBVectorStore(
    path="./my_data",
    collection_name="documents",
    embedding=OpenAIEmbeddings()
)

# Add documents
vectorstore.add_texts(["Hello world", "VelesDB is fast"])

# Search
results = vectorstore.similarity_search("greeting", k=2)

# Use as retriever for RAG
retriever = vectorstore.as_retriever(search_kwargs={"k": 5})
```

### Tauri Desktop Integration

Install the plugin in your Tauri project:

```toml
# Cargo.toml (backend)
[dependencies]
tauri-plugin-velesdb = "0.1"
```

```bash
# Frontend (npm / pnpm / yarn)
npm install tauri-plugin-velesdb
# pnpm add tauri-plugin-velesdb
# yarn add tauri-plugin-velesdb
```

Build AI-powered desktop apps with vector search:

```rust
// Rust - Plugin Registration
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_velesdb::init("./data"))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

```javascript
// JavaScript - Frontend API
import { invoke } from '@tauri-apps/api/core';

// Create collection
await invoke('plugin:velesdb|create_collection', {
  request: { name: 'documents', dimension: 768, metric: 'cosine' }
});

// Vector search
const results = await invoke('plugin:velesdb|search', {
  request: { collection: 'documents', vector: [...], topK: 10 }
});

// Hybrid search (vector + BM25)
const hybrid = await invoke('plugin:velesdb|hybrid_search', {
  request: { 
    collection: 'documents', 
    vector: [...], 
    query: 'AI tutorial',
    vectorWeight: 0.7 
  }
});
```

See [tauri-plugin-velesdb](./integrations/tauri-plugin-velesdb) for full documentation.

---

## 💻 VelesQL CLI

Interactive command-line interface for VelesQL queries.

```bash
# Start REPL
velesdb-cli repl

# Execute single query
velesdb-cli query "SELECT * FROM documents LIMIT 10"

# Show database info
velesdb-cli info ./data
```

**REPL Session:**
```
VelesQL REPL v0.2.0
Type 'help' for commands, 'quit' to exit.

velesql> SELECT * FROM documents WHERE category = 'tech' LIMIT 5;
┌────┬───────────────────┬──────────┐
│ id │ title             │ category │
├────┼───────────────────┼──────────┤
│ 1  │ AI Introduction   │ tech     │
│ 2  │ ML Basics         │ tech     │
└────┴───────────────────┴──────────┘
2 rows (1.23 ms)
```

---

## 📚 Documentation

Comprehensive documentation is available on **DeepWiki**:

<p align="center">
  <a href="https://deepwiki.com/cyberlife-coder/VelesDB/"><img src="https://img.shields.io/badge/📖_Full_Documentation-DeepWiki-blue?style=for-the-badge" alt="DeepWiki Documentation"></a>
</p>

### Documentation Index

| Section | Description |
|---------|-------------|
| [**Overview**](https://deepwiki.com/cyberlife-coder/VelesDB/) | Introduction, architecture diagrams, and component overview |
| [**System Architecture**](https://deepwiki.com/cyberlife-coder/VelesDB/1.1-system-architecture) | Layered architecture and component interactions |
| [**Deployment Patterns**](https://deepwiki.com/cyberlife-coder/VelesDB/1.2-deployment-patterns) | Library, Server, WASM, Tauri, and Docker deployments |
| [**Core Engine**](https://deepwiki.com/cyberlife-coder/VelesDB/3-core-engine-(velesdb-core)) | In-depth `velesdb-core` internals (HNSW, BM25, ColumnStore) |
| [**REST API Reference**](https://deepwiki.com/cyberlife-coder/VelesDB/4-rest-api-server) | Complete API documentation with all 11 endpoints |
| [**VelesQL Language**](https://deepwiki.com/cyberlife-coder/VelesDB/4.2-velesql-query-language) | SQL-like query syntax, operators, and examples |
| [**SIMD Optimizations**](https://deepwiki.com/cyberlife-coder/VelesDB/3.5-simd-optimizations) | Platform-specific SIMD (AVX2, NEON, WASM SIMD128) |
| [**Performance & Benchmarks**](https://deepwiki.com/cyberlife-coder/VelesDB/9-performance-and-benchmarks) | Detailed benchmarks and optimization guide |

### Tutorials

| Tutorial | Description |
|----------|-------------|
| [**Build a RAG Desktop App**](docs/tutorials/tauri-rag-app/) | Step-by-step guide to build a local RAG app with Tauri |

### Quick Links

- 📖 **[Full Documentation](https://deepwiki.com/cyberlife-coder/VelesDB/)** — Architecture, internals, and API reference
- 📊 **[Benchmarks](docs/BENCHMARKS.md)** — Performance metrics and comparisons
- 📝 **[VelesQL Specification](docs/VELESQL_SPEC.md)** — Complete language reference with BNF grammar
- 📝 **[Changelog](CHANGELOG.md)** — Version history and release notes
- 🏗️ **[Architecture](docs/ARCHITECTURE.md)** — Technical deep-dive

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

### v0.1.0 ✅ (Released)
- [x] HNSW vector index
- [x] REST API (11 endpoints)
- [x] VelesQL query language
- [x] SIMD-optimized distance calculations
- [x] SQ8 quantization
- [x] Metadata filtering

### v0.2.0 ✅ (Current)
- [x] Python bindings (PyO3) with NumPy support
- [x] CLI / REPL for VelesQL
- [x] LangChain integration (`langchain-velesdb`)
- [x] **New Metrics**: Hamming (Binary) & Jaccard (Sets)
- [x] OpenAPI/Swagger docs
- [x] **BM25 Full-Text Search** with hybrid search (vector + text)
- [x] **Tauri Desktop Plugin** for AI-powered desktop apps
- [x] **WASM Support** for browser-based vector search
- [x] **Advanced Search Metrics**: NDCG, Hit Rate, MAP, Precision@k
- [x] **Latency Percentiles**: p50, p95, p99 for production monitoring
- [x] **VelesQL Specification**: Complete language documentation with BNF grammar

### v0.3.0 (Planned)
- [ ] LlamaIndex integration
- [ ] Publish to crates.io & PyPI
- [ ] TypeScript SDK
- [ ] Multi-tenancy support

### v1.0.0 (Future)
- [ ] Production-ready stability
- [ ] Product Quantization (PQ)
- [ ] Sparse vector support
- [ ] API Authentication

---

## 💎 VelesDB Premium

Need enterprise features? **VelesDB Premium** extends Core with:

| Feature | Description |
|---------|-------------|
| **Encryption at Rest** | AES-256-GCM for data security |
| **Snapshots** | Atomic backup/restore |
| **RBAC / Multi-tenancy** | Role-based access control |
| **Distributed Mode** | Horizontal scaling |
| **Priority Support** | SLA-backed support |

👉 [Learn more about VelesDB Premium](https://github.com/cyberlife-coder/velesdb-premium)

---

## 📜 License

VelesDB is licensed under the [Business Source License 1.1 (BSL-1.1)](LICENSE).

The BSL is a source-available license that converts to Apache 2.0 after 4 years.

---

<p align="center">
  <strong>Built with ❤️ and 🦀 Rust</strong><br/>
  <a href="https://github.com/cyberlife-coder/VelesDB">GitHub</a> •
  <a href="https://deepwiki.com/cyberlife-coder/VelesDB/">Documentation</a> •
  <a href="https://github.com/cyberlife-coder/VelesDB/issues">Issues</a> •
  <a href="https://github.com/cyberlife-coder/VelesDB/releases">Releases</a>
</p>
