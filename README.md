<p align="center">
  <img src="docs/assets/velesdb-icon.png" alt="VelesDB Logo" width="200"/>
</p>

<h1 align="center">🐺 VelesDB</h1>

<h3 align="center">
  🧠 <strong>The Local Knowledge Engine for AI Agents</strong> 🧠<br/>
  <em>Vector + Graph Fusion • 57µs Search • Single Binary • Privacy-First</em>
</h3>

---

## 🌍 Full Ecosystem / Écosystème Complet

VelesDB is designed to run **where your agents live** — from cloud servers to mobile devices to browsers.

| Domain      | Component                          | Description                              | Install                     |
|-------------|------------------------------------|------------------------------------------|----------------------------|
| **🦀 Core** | [velesdb-core](crates/velesdb-core) | Core engine (HNSW, SIMD, VelesQL)        | `cargo add velesdb-core`   |
| **🌐 Server**| [velesdb-server](crates/velesdb-server) | REST API (11 endpoints, OpenAPI)         | `cargo install velesdb-server` |
| **💻 CLI**  | [velesdb-cli](crates/velesdb-cli)   | Interactive REPL for VelesQL             | `cargo install velesdb-cli` |
| **🐍 Python** | [velesdb-python](crates/velesdb-python) | PyO3 bindings + NumPy                    | `pip install velesdb`      |
| **📜 TypeScript** | [typescript-sdk](sdks/typescript) | Node.js & Browser SDK                    | `npm i @wiscale/velesdb`   |
| **🌍 WASM** | [velesdb-wasm](crates/velesdb-wasm) | Browser-side vector search               | `npm i @wiscale/velesdb-wasm` |
| **📱 Mobile** | [velesdb-mobile](crates/velesdb-mobile) | iOS (Swift) & Android (Kotlin)           | [Build instructions](#-mobile-build) |
| **🖥️ Desktop** | [tauri-plugin](crates/tauri-plugin-velesdb) | Tauri v2 AI-powered apps               | `cargo add tauri-plugin-velesdb` |
| **🦜 LangChain** | [langchain-velesdb](integrations/langchain) | Official VectorStore                   | `pip install langchain-velesdb` |
| **🦙 LlamaIndex** | [llamaindex-velesdb](integrations/llamaindex) | Document indexing                     | `pip install llama-index-vector-stores-velesdb` |
| **🔄 Migration** | [velesdb-migrate](crates/velesdb-migrate) | From Qdrant, Pinecone, Supabase        | `cargo install velesdb-migrate` |

---

## 🎯 Use Cases

| Use Case                      | VelesDB Feature                     |
|-------------------------------|-------------------------------------|
| **RAG Pipelines**             | Sub-ms retrieval                    |
| **AI Agents**                 | Embedded memory, local context      |
| **Desktop Apps (Tauri/Electron)** | Single binary, no server needed     |
| **Mobile AI (iOS/Android)**   | Native SDKs with 32x memory compression |
| **Browser-side Search**       | WASM module, zero backend           |
| **Edge/IoT Devices**          | 15MB footprint, ARM NEON optimized  |
| **On-Prem / Air-Gapped**      | No cloud dependency, full data sovereignty |

---

## 🚀 Quick Start

### Option 1: Linux Package (.deb) ⭐ Recommended for Linux

Download from [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases):

```bash
# Install
sudo dpkg -i velesdb-1.1.0-amd64.deb

# Binaries installed to /usr/bin
velesdb --version
velesdb-server --version
```

### Option 2: One-liner Script

**Linux / macOS:**
```bash
curl -fsSL https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/scripts/install.sh | bash
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/scripts/install.ps1 | iex
```

### Option 3: Python (from source)

```bash
# Build from source (requires Rust)
cd crates/velesdb-python
pip install maturin
maturin develop --release
```

```python
import velesdb

db = velesdb.Database("./my_vectors")
collection = db.create_collection("docs", dimension=768, metric="cosine")
collection.upsert([{"id": 1, "vector": [...], "payload": {"title": "Hello"}}])
results = collection.search([...], top_k=10)
```

```bash
# Install from PyPI
pip install velesdb
```

### Option 4: Rust (from source)

```bash
# Clone and build
git clone https://github.com/cyberlife-coder/VelesDB.git
cd VelesDB
cargo build --release

# Binaries in target/release/
./target/release/velesdb-server --help
```

```bash
# Install from crates.io
cargo install velesdb-cli
```

### Option 5: Docker (build locally)

```bash
# Build and run locally
git clone https://github.com/cyberlife-coder/VelesDB.git
cd VelesDB
docker build -t velesdb .
docker run -d -p 8080:8080 -v velesdb_data:/data velesdb
```

```bash
# Pull from GitHub Container Registry
docker pull ghcr.io/cyberlife-coder/velesdb:latest
```

### Option 6: Portable Archives

Download from [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases):

| Platform | File |
|----------|------|
| Windows | `velesdb-windows-x86_64.zip` |
| Linux | `velesdb-linux-x86_64.tar.gz` |
| macOS (ARM) | `velesdb-macos-arm64.tar.gz` |
| macOS (Intel) | `velesdb-macos-x86_64.tar.gz` |

### Start Using VelesDB

```bash
# Start the REST API server (data persisted in ./data)
velesdb-server --data-dir ./my_data

# Or use the interactive CLI with VelesQL REPL
velesdb repl

# Verify server is running
curl http://localhost:8080/health
# {"status":"healthy","version":"1.1.0"}
```

📖 **Full installation guide:** [docs/INSTALLATION.md](docs/INSTALLATION.md)

<a name="-mobile-build"></a>
### 📱 Mobile Build (iOS/Android)

```bash
# iOS (macOS required)
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo build --release --target aarch64-apple-ios -p velesdb-mobile

# Android (NDK required)
cargo install cargo-ndk
cargo ndk -t arm64-v8a -t armeabi-v7a build --release -p velesdb-mobile
```

📖 **Full mobile guide:** [crates/velesdb-mobile/README.md](crates/velesdb-mobile/README.md)

---

## 📖 Your First Vector Search

```bash
# 1. Create a collection
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "my_vectors", "dimension": 4, "metric": "cosine"}'

# 2. Insert vectors with metadata
curl -X POST http://localhost:8080/collections/my_vectors/points \
  -H "Content-Type: application/json" \
  -d '{
    "points": [
      {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "AI Introduction", "category": "tech"}},
      {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"title": "ML Basics", "category": "tech"}},
      {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"title": "History of Computing", "category": "history"}}
    ]
  }'

# 3. Search for similar vectors
curl -X POST http://localhost:8080/collections/my_vectors/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [0.9, 0.1, 0.0, 0.0], "top_k": 2}'

# 4. Or use VelesQL (SQL-like queries)
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT * FROM my_vectors WHERE vector NEAR $v AND category = '\''tech'\'' LIMIT 5",
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


### 🔥 Core Vector Operations (768D - BERT/OpenAI dimensions)

| Operation | Latency | Throughput | vs. Naive |
|-----------|---------|------------|----------|
| **Dot Product (1536D)** | **66 ns** | **15M ops/sec** | 🚀 **8x faster** |
| **Euclidean (768D)** | **44 ns** | **23M ops/sec** | 🚀 **6x faster** |
| **Cosine (768D)** | **78 ns** | **13M ops/sec** | 🚀 **4x faster** |
| **Hamming (Binary)**| **6 ns** | **164M ops/sec** | 🚀 **10x faster** |

### 📊 System Performance (10K Vectors, Local)

| Benchmark | Result | Details |
|-----------|--------|---------|
| **HNSW Search** | **57 µs** | p50 latency (Cosine) |
| **VelesQL Parsing**| **554 ns** | Simple SELECT |
| **VelesQL Cache Hit**| **48 ns** | HashMap pre-allocation |
| **Recall@10** | **100%** | Perfect mode (brute-force SIMD) |
| **BM25 Search** | **33 µs** | Adaptive PostingList (10K docs) |

### 🎯 Search Quality (Recall)

| Mode | Recall@10 | Latency (128D) | Use Case |
|------|-----------|----------------|----------|
| Fast | 92.2% | ~26µs | Real-time, high throughput |
| Balanced | 98.8% | ~39µs | Production recommended |
| Accurate | 100% | ~67µs | High precision |
| **Perfect** | **100%** | ~220µs | Brute-force SIMD |

### 🛠️ Optimizations Under the Hood

- **SIMD**: AVX-512/AVX2 auto-detection with 32-wide FMA
- **Prefetch**: CPU cache warming for HNSW traversal (+12% throughput)
- **Contiguous Layout**: 64-byte aligned memory for cache efficiency
- **Batch WAL**: Single disk write per batch import
- **Zero-Copy**: Memory-mapped files for fast startup

> 📊 Full benchmarks: [docs/BENCHMARKS.md](docs/BENCHMARKS.md)

### 📦 Vector Quantization (Memory Reduction)

Reduce memory usage by **4-32x** with minimal recall loss:

| Method | Compression | Recall Loss | Use Case |
|--------|-------------|-------------|----------|
| **SQ8** (8-bit) | **4x** | < 2% | General purpose, Edge |
| **Binary** (1-bit) | **32x** | ~10-15% | Fingerprints, IoT |

```rust
use velesdb_core::quantization::{QuantizedVector, dot_product_quantized_simd};

// Compress 768D vector: 3072 bytes → 776 bytes (4x reduction)
let quantized = QuantizedVector::from_f32(&embedding);

// SIMD-optimized search (only ~30% slower than f32)
let similarity = dot_product_quantized_simd(&query, &quantized);
```

> 📖 Full guide: [docs/QUANTIZATION.md](docs/QUANTIZATION.md)

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
| **License** | **ELv2** | Apache 2.0 | Apache 2.0 | Closed | PostgreSQL |
| **Best For** | **Embedded / Edge / Speed** | Multimodal / Lakehouse | Scale / Cloud | Managed SaaS | Relational + Vector |

### 🎯 VelesDB Characteristics

#### ⚡ Low Latency
- **~66ns** per vector distance (1536D with native intrinsics)
- **57µs** HNSW search p50 on 10K vectors
- **SIMD-optimized** (AVX-512, AVX2, NEON native intrinsics)

#### 📝 SQL-Native Queries (VelesQL)
```sql
-- SQL-like syntax
SELECT * FROM docs WHERE vector NEAR $v AND category = 'tech' LIMIT 10
```

#### 📦 Zero-Config Simplicity
- **Single binary** (~15MB) — no Docker, no dependencies
- **WASM support** for browser-side search
- **Tauri plugin** for AI-powered desktop apps

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
| **Vector search** | `vector NEAR $v` | Find similar vectors (uses collection's metric) |
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
| Simple SELECT | **554 ns** | **1.8M queries/sec** |
| Vector search | **873 ns** | **1.1M queries/sec** |
| Complex (multi-filter) | **3.5 µs** | **280K queries/sec** |

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
velesdb-core = "1.1"
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
tauri-plugin-velesdb = "1.1"
```

```bash
# Frontend (npm / pnpm / yarn)
npm install @wiscale/tauri-plugin-velesdb
# pnpm add @wiscale/tauri-plugin-velesdb
# yarn add @wiscale/tauri-plugin-velesdb
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

See [tauri-plugin-velesdb](./crates/tauri-plugin-velesdb) for full documentation.

---

## 📱 Mobile SDK (iOS & Android)

Native bindings for mobile platforms via [UniFFI](https://mozilla.github.io/uniffi-rs/).

### Features

- **Native Performance** — Direct Rust bindings, no FFI overhead
- **Binary Quantization** — 32x memory reduction for constrained devices
- **ARM NEON SIMD** — Optimized for mobile processors (Apple A-series, Snapdragon)
- **Offline-First** — Full functionality without network connectivity
- **Thread-Safe** — Safe to use from multiple threads/queues

### Swift (iOS)

```swift
import VelesDB

// Open database
let db = try VelesDatabase.open(path: documentsPath + "/velesdb")

// Create collection (384D for MiniLM)
try db.createCollection(name: "documents", dimension: 384, metric: .cosine)

// Get collection and insert
let collection = try db.getCollection(name: "documents")!
let point = VelesPoint(id: 1, vector: embedding, payload: "{\"title\": \"Hello\"}")
try collection.upsert(point: point)

// Search
let results = try collection.search(vector: queryEmbedding, limit: 10)
```

### Kotlin (Android)

```kotlin
import com.velesdb.mobile.*

// Open database
val db = VelesDatabase.open("${context.filesDir}/velesdb")

// Create collection
db.createCollection("documents", 384u, DistanceMetric.COSINE)

// Get collection and insert
val collection = db.getCollection("documents")!!
val point = VelesPoint(id = 1uL, vector = embedding, payload = "{\"title\": \"Hello\"}")
collection.upsert(point)

// Search
val results = collection.search(queryEmbedding, 10u)
```

### Storage Modes (IoT/Edge)

| Mode | Compression | Memory/dim | Recall Loss | Use Case |
|------|-------------|------------|-------------|----------|
| `Full` | 1x | 4 bytes | 0% | Best quality |
| `Sq8` | 4x | 1 byte | ~1% | **Recommended for mobile** |
| `Binary` | 32x | 1 bit | ~5-10% | Extreme IoT constraints |

```swift
// iOS - SQ8 compression (4x memory reduction)
try db.createCollectionWithStorage(
    name: "embeddings", dimension: 384, metric: .cosine, storageMode: .sq8
)
```

📖 **Full documentation:** [crates/velesdb-mobile/README.md](crates/velesdb-mobile/README.md)

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
VelesQL REPL v1.1.0
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

## ⭐ Support VelesDB

<p align="center">
  <strong>🌟 If VelesDB helps you build faster AI applications, give us a star!</strong><br/>
  <em>Si VelesDB vous aide à créer des applications IA plus rapides, offrez-nous une étoile !</em>
</p>

<p align="center">
  <a href="https://github.com/cyberlife-coder/VelesDB/stargazers">
    <img src="https://img.shields.io/github/stars/cyberlife-coder/VelesDB?style=for-the-badge&logo=github&color=yellow" alt="GitHub Stars"/>
  </a>
</p>

### ☕ Buy Me A Coffee

If you find this project useful, you can support its development by buying me a coffee!

<a href="https://buymeacoffee.com/wiscale" target="_blank">
    <img src="https://cdn.buymeacoffee.com/buttons/v2/default-yellow.png" alt="Buy Me A Coffee" style="height: 60px; width: 217px;" >
</a>

```
