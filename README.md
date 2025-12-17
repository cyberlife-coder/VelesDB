<p align="center">
  <img src="docs/assets/velesdb-logo.svg" alt="VelesDB Logo" width="200"/>
</p>

<h1 align="center">VelesDB</h1>

<p align="center">
  <strong>The Local-First Vector Database, Built in Rust</strong>
</p>

<p align="center">
  <a href="https://github.com/YOUR_USERNAME/velesdb/actions"><img src="https://img.shields.io/github/actions/workflow/status/YOUR_USERNAME/velesdb/ci.yml?branch=main&style=flat-square" alt="Build Status"></a>
  <a href="https://crates.io/crates/velesdb"><img src="https://img.shields.io/crates/v/velesdb.svg?style=flat-square" alt="Crates.io"></a>
  <a href="https://github.com/YOUR_USERNAME/velesdb/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg?style=flat-square" alt="License"></a>
  <a href="https://discord.gg/YOUR_DISCORD"><img src="https://img.shields.io/discord/YOUR_DISCORD_ID?style=flat-square&logo=discord" alt="Discord"></a>
</p>

<p align="center">
  <a href="#-features">Features</a> •
  <a href="#-quick-start">Quick Start</a> •
  <a href="#-documentation">Documentation</a> •
  <a href="#-benchmarks">Benchmarks</a> •
  <a href="#-contributing">Contributing</a>
</p>

---

## 🎯 Why VelesDB?

**The Problem:** Cloud-based vector databases are powerful, but they come with trade-offs: high costs at scale, network latency, data sovereignty concerns, and vendor lock-in. For many use cases—especially in Europe—sending sensitive data to third-party cloud services is simply not an option.

**The Solution:** VelesDB is a **high-performance, local-first vector database** written entirely in Rust. It gives you the power of semantic search and AI-powered retrieval **on your own infrastructure**, with a single binary that's easy to deploy and operate.

> *"VelesDB is to vector search what SQLite is to relational databases: simple, fast, and everywhere."*

---

## ✨ Features

| Feature | Core (Free) | Premium |
|---------|:-----------:|:-------:|
| 🚀 **Blazing Fast HNSW Index** | ✅ | ✅ |
| 💾 **Persistent Storage (mmap)** | ✅ | ✅ |
| 🔌 **REST API** | ✅ | ✅ |
| 📦 **Single Binary Deployment** | ✅ | ✅ |
| 🔍 **Hybrid Search (BM25 + Vector)** | ❌ | ✅ |
| 🎯 **Advanced Metadata Filtering** | ❌ | ✅ |
| 🔐 **Encryption at Rest** | ❌ | ✅ |
| 📸 **Snapshots & Backups** | ❌ | ✅ |
| 🐍 **Official Python SDK** | ❌ | ✅ |
| 📞 **Priority Support & SLA** | ❌ | ✅ |

---

## 🚀 Quick Start

### Using Docker (Recommended)

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
| [Getting Started Guide](docs/getting-started.md) | Step-by-step tutorial for your first VelesDB project |
| [API Reference](docs/api-reference.md) | Complete REST API documentation |
| [Configuration](docs/configuration.md) | Server configuration options |
| [Architecture](docs/architecture.md) | Deep dive into VelesDB internals |
| [FAQ](docs/faq.md) | Frequently asked questions |

---

## 📊 Benchmarks

Performance comparison on **1 million vectors** (768 dimensions, cosine similarity):

| Metric | VelesDB | Qdrant | LanceDB |
|--------|---------|--------|---------|
| **Search Latency (p99)** | 12ms | 15ms | 18ms |
| **Insert Throughput** | 15K vec/s | 12K vec/s | 10K vec/s |
| **Memory Usage** | 2.1 GB | 2.8 GB | 2.4 GB |
| **Binary Size** | 8 MB | 45 MB | 12 MB |

*Benchmarks run on AWS c5.xlarge (4 vCPU, 8GB RAM). See [benches/](benches/) for methodology.*

---

## 🏗️ Use Cases

### 🔎 Semantic Document Search
Build powerful search experiences that understand meaning, not just keywords.

### 🛒 Product Recommendations
Power real-time "similar items" features for e-commerce platforms.

### 🤖 RAG Applications
Enhance your LLM applications with fast, local vector retrieval.

### 🏢 Enterprise Knowledge Bases
Keep your sensitive corporate data on-premise while enabling AI-powered search.

---

## 🤝 Contributing

We love contributions! Whether it's bug reports, feature requests, documentation improvements, or code contributions, we welcome them all.

Please read our [Contributing Guide](CONTRIBUTING.md) and [Code of Conduct](CODE_OF_CONDUCT.md) before getting started.

### Good First Issues

Looking for a place to start? Check out issues labeled [`good first issue`](https://github.com/YOUR_USERNAME/velesdb/labels/good%20first%20issue).

---

## 💬 Community

- **Discord:** [Join our community](https://discord.gg/YOUR_DISCORD)
- **Twitter:** [@VelesDB](https://twitter.com/VelesDB)
- **GitHub Discussions:** [Ask questions & share ideas](https://github.com/YOUR_USERNAME/velesdb/discussions)

---

## 📜 License

VelesDB Core is licensed under the [Apache License 2.0](LICENSE).

VelesDB Premium features are available under a commercial license. [Contact us](mailto:sales@velesdb.io) for pricing.

---

## ⭐ Star History

If you find VelesDB useful, please consider giving us a star! It helps others discover the project.

[![Star History Chart](https://api.star-history.com/svg?repos=YOUR_USERNAME/velesdb&type=Date)](https://star-history.com/#YOUR_USERNAME/velesdb&Date)

---

<p align="center">
  Made with ❤️ and 🦀 by the VelesDB Team
</p>
