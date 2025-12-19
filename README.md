<p align="center">
  <img src="docs/assets/velesdb-logo.svg" alt="VelesDB Logo" width="200"/>
</p>

<h1 align="center">VelesDB</h1>

<p align="center">
  <strong>A High-Performance Vector Database, Built in Rust</strong>
</p>

<p align="center">
  <a href="https://github.com/cyberlife-coder/velesdb/actions"><img src="https://img.shields.io/github/actions/workflow/status/cyberlife-coder/velesdb/ci.yml?branch=main&style=flat-square" alt="Build Status"></a>
  <a href="https://crates.io/crates/velesdb"><img src="https://img.shields.io/crates/v/velesdb.svg?style=flat-square" alt="Crates.io"></a>
  <a href="https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg?style=flat-square" alt="License"></a>
</p>

<p align="center">
  <a href="#-features">Features</a> •
  <a href="#-quick-start">Quick Start</a> •
  <a href="#-documentation">Documentation</a> •
  <a href="#-contributing">Contributing</a>
</p>

---

## 🎯 What is VelesDB?

VelesDB is a **high-performance vector database** written entirely in Rust. It enables fast similarity search for AI applications, semantic search, and recommendation systems.

Built with performance and simplicity in mind, VelesDB provides an easy-to-use REST API for storing and querying vector embeddings.

### 🐺 Why "Veles"?

**Veles** (Велес) is a major Slavic deity — the god of earth, waters, forests, and the underworld. In Slavic mythology, he is associated with **wisdom, magic, and knowledge**. As the guardian of sacred knowledge and keeper of memories, Veles perfectly embodies what a vector database does: storing, organizing, and retrieving the essence of information.

Just as Veles bridges the earthly and mystical realms, VelesDB bridges the gap between raw data and meaningful AI-powered insights.

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

### Using Docker

```bash
# Pull and run VelesDB
docker run -d -p 8080:8080 -v velesdb_data:/data velesdb/velesdb:latest

# Test the connection
curl http://localhost:8080/health
```

### Using Cargo

```bash
# Install from crates.io
cargo install velesdb-server

# Run the server
velesdb-server --data-dir ./data --port 8080
```

### Your First Search

```bash
# Create a collection
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "documents", "dimension": 768, "metric": "cosine"}'

# Insert vectors
curl -X POST http://localhost:8080/collections/documents/points \
  -H "Content-Type: application/json" \
  -d '{
    "points": [
      {"id": 1, "vector": [0.1, 0.2, ...], "payload": {"title": "Introduction to AI"}},
      {"id": 2, "vector": [0.3, 0.4, ...], "payload": {"title": "Machine Learning Basics"}}
    ]
  }'

# Search for similar vectors
curl -X POST http://localhost:8080/collections/documents/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [0.15, 0.25, ...], "top_k": 5}'
```

---

## 📚 Documentation

| Resource | Description |
|----------|-------------|
| [Getting Started Guide](docs/getting-started.md) | Step-by-step tutorial |
| [API Reference](docs/api-reference.md) | Complete REST API documentation |
| [Configuration](docs/configuration.md) | Server configuration options |

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

VelesQL is a SQL-like query language designed for vector search operations.

### Syntax

```sql
SELECT * FROM documents 
WHERE vector NEAR $query_vector
  AND category = 'tech'
  AND price > 100
LIMIT 10;
```

### Supported Features

| Feature | Example |
|---------|---------|
| Vector search | `vector NEAR $v` |
| Distance metrics | `vector NEAR COSINE $v`, `EUCLIDEAN`, `DOT` |
| Comparisons | `=`, `!=`, `>`, `<`, `>=`, `<=` |
| IN clause | `category IN ('tech', 'science')` |
| BETWEEN | `price BETWEEN 10 AND 100` |
| LIKE | `title LIKE '%rust%'` |
| NULL checks | `deleted_at IS NULL`, `IS NOT NULL` |
| Logical ops | `AND`, `OR` with precedence |
| Parameters | `$param_name` for bound values |

### Parser Performance

| Query Type | Time | Throughput |
|------------|------|------------|
| Simple SELECT | ~755 ns | **1.3M queries/sec** |
| Vector search | ~1.2 µs | **800K queries/sec** |
| Complex (multi-filter) | ~4.8 µs | **200K queries/sec** |

---

## 🏗️ Use Cases

- **Semantic Search** — Build search experiences that understand meaning
- **Recommendations** — Power "similar items" features
- **RAG Applications** — Enhance LLM applications with vector retrieval
- **Image Search** — Find visually similar images

---

## 🤝 Contributing

We welcome contributions! Whether it's bug reports, feature requests, or code contributions.

Please read our [Contributing Guide](CONTRIBUTING.md) and [Code of Conduct](CODE_OF_CONDUCT.md) before getting started.

### Good First Issues

Looking for a place to start? Check out issues labeled [`good first issue`](https://github.com/cyberlife-coder/velesdb/labels/good%20first%20issue).

---

## 📜 License

VelesDB is licensed under the [Apache License 2.0](LICENSE).

---

<p align="center">
  Made with ❤️ and 🦀
</p>
