<p align="center">
  <img src="velesdb_icon_pack/favicon/android-chrome-512x512.png" alt="VelesDB Logo" width="200"/>
</p>

<h1 align="center">
  <img src="velesdb_icon_pack/favicon/favicon-32x32.png" alt="VelesDB" width="32" height="32" style="vertical-align: middle;"/>
</h1>

<h3 align="center">
  The Local Knowledge Engine for AI Agents<br/>
  <em>Vector + Graph + ColumnStore Fusion &bull; 42.8&micro;s HNSW Search &bull; 23.6ns SIMD &bull; 4,300+ Tests &bull; 82% Coverage</em>
</h3>

<p align="center">
  <a href="https://github.com/cyberlife-coder/VelesDB/actions/workflows/ci.yml"><img src="https://github.com/cyberlife-coder/VelesDB/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://app.codacy.com/gh/cyberlife-coder/VelesDB/dashboard?utm_source=gh&utm_medium=referral&utm_content=&utm_campaign=Badge_grade"><img src="https://app.codacy.com/project/badge/Grade/58c73832dd294ba38144856ae69e9cf2" alt="Codacy Badge"></a>
  <a href="https://crates.io/crates/velesdb-core"><img src="https://img.shields.io/crates/v/velesdb-core.svg" alt="Crates.io"></a>
  <a href="https://crates.io/crates/velesdb-core"><img src="https://img.shields.io/crates/d/velesdb-core.svg" alt="Crates.io Downloads"></a>
  <a href="https://pypi.org/project/velesdb/"><img src="https://img.shields.io/pypi/v/velesdb.svg" alt="PyPI"></a>
  <a href="https://www.npmjs.com/package/@wiscale/velesdb-sdk"><img src="https://img.shields.io/npm/v/@wiscale/velesdb-sdk.svg" alt="npm"></a>
  <img src="https://img.shields.io/badge/coverage-82.3%25-brightgreen" alt="Coverage">
  <a href="https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-VelesDB_Core_1.0-blue" alt="License"></a>
  <a href="https://github.com/cyberlife-coder/VelesDB"><img src="https://img.shields.io/github/stars/cyberlife-coder/VelesDB?style=flat-square" alt="Stars"></a>
</p>

<p align="center">
  <a href="https://github.com/cyberlife-coder/VelesDB/releases/tag/v1.6.0">Download v1.6.0</a> &bull;
  <a href="#getting-started-in-60-seconds">Quick Start</a> &bull;
  <a href="https://velesdb.com/en/">Documentation</a> &bull;
  <a href="https://deepwiki.com/cyberlife-coder/VelesDB">DeepWiki</a>
</p>

---

## What's New in v1.6

- **Server Security** — API key authentication + TLS (HTTPS) + graceful shutdown
- **6 New REST Endpoints** — `/ready`, `/stats`, `/config`, `/guardrails`, `/analyze`, `/search/ids`
- **SIMD Performance Gains** — 4-accumulator ILP kernels for Hamming & Jaccard, cosine finish optimization
- **PQ/Sparse/Graph Enhancements** — OPQ rotation fix, batch sparse insert, composite property index

> Full changelog: [What's New in v1.6](https://github.com/cyberlife-coder/VelesDB/releases/tag/v1.6.0)

---

## The Problem We Solve

> **"My RAG agent needs both semantic search AND knowledge relationships. Existing tools force me to choose or glue multiple systems together."**

| Pain Point | Business Impact | VelesDB Solution |
|------------|-----------------|------------------|
| **Latency kills UX** | Cloud vector DBs add 50-100ms/query | **42.8µs local** (k=10, 10K vectors) — network-free retrieval |
| **Vectors alone aren't enough** | Semantic similarity misses relationships | **Vector + Graph unified** in one query |
| **Privacy & deployment friction** | Cloud dependencies, GDPR concerns | **6 MB self-contained binary** — works offline, air-gapped |

---

## Why Developers Choose VelesDB

<table align="center">
<tr>
<td align="center" width="25%">
<h3>Vector + Graph + Columns</h3>
<p>Unified semantic search, relationships, AND structured data.<br/><strong>No glue code needed.</strong></p>
</td>
<td align="center" width="25%">
<h3>23.6ns SIMD</h3>
<p>Native HNSW + AVX-512/AVX2/NEON SIMD.<br/><strong>32.5 Gelem/s throughput.</strong></p>
</td>
<td align="center" width="25%">
<h3>6 MB Self-Contained</h3>
<p>No external services required.<br/><strong>Works offline, air-gapped.</strong></p>
</td>
<td align="center" width="25%">
<h3>Run Anywhere</h3>
<p>Server, Browser, Mobile, Desktop.<br/><strong>Same Rust codebase.</strong></p>
</td>
</tr>
</table>

---

## Quick Comparison

| | **VelesDB** | Chroma | Qdrant | pgvector |
|---|---|---|---|---|
| **Architecture** | Unified vector + graph + columnar | Vector only | Vector + payload | Vector extension for PostgreSQL |
| **Deployment** | Embedded / Server / WASM / Mobile | Server (Python) | Server (Rust) | Requires PostgreSQL |
| **Binary size** | 6 MB | ~500 MB (with deps) | ~50 MB | N/A (PG extension) |
| **Search latency** | 42.8µs (embedded) | ~1-5ms | ~1-5ms | ~5-20ms |
| **Graph support** | Native (MATCH clause) | No | No | No |
| **Query language** | VelesQL (SQL + NEAR + MATCH) | Python API | JSON API / gRPC | SQL + operators |
| **Browser (WASM)** | Yes | No | No | No |
| **Mobile (iOS/Android)** | Yes | No | No | No |
| **Offline / Local-first** | Yes | Partial | No | No |
| **Quantization** | SQ8, PQ, Binary | No | SQ, PQ, Binary | No (halfvec only) |
| **LangChain integration** | Yes | Yes | Yes | Yes |
| **LlamaIndex integration** | Yes | Yes | Yes | Yes |

> **VelesDB's sweet spot:** When you need vector + graph in a single engine, local-first deployment, or a lightweight binary that runs anywhere (server, browser, mobile).
>
> **Not the best fit (yet):** If you need a managed cloud service with a multi-node distributed cluster, or if you're already invested in a PostgreSQL ecosystem and only need basic vector search.

---

## Getting Started in 60 Seconds

### Install

Choose your preferred method:

**Cargo (Rust):**
```bash
cargo install velesdb-server velesdb-cli
```

**Python:**
```bash
pip install velesdb
```

**Docker:**
```bash
docker run -d -p 8080:8080 -v velesdb_data:/data --name velesdb velesdb/velesdb:latest
```

**Docker Compose:**
```bash
curl -O https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/docker-compose.yml
docker-compose up -d
```

**WASM (Browser):**
```bash
npm install @wiscale/velesdb-wasm
```

**Install script (Linux/macOS):**
```bash
curl -fsSL https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/scripts/install.sh | bash
```

**Install script (Windows PowerShell):**
```powershell
irm https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/scripts/install.ps1 | iex
```

### Start the server

```bash
velesdb-server --data-dir ./my_data
# Server running at http://localhost:8080

curl http://localhost:8080/health
# {"status":"ok","version":"1.6.0"}
```

### Store your first vectors

```bash
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "docs", "dimension": 4, "metric": "cosine"}'

curl -X POST http://localhost:8080/collections/docs/points \
  -H "Content-Type: application/json" \
  -d '{
    "points": [
      {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "My first doc"}}
    ]
  }'
```

### Search (42µs latency)

```bash
curl -X POST http://localhost:8080/collections/docs/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [0.9, 0.1, 0.0, 0.0], "top_k": 5}'
```

### Or use VelesQL

```sql
SELECT * FROM docs
WHERE category = 'tech'
  AND vector NEAR $v
LIMIT 5
```

```sql
-- The query that defines VelesDB: Vector + Graph in ONE statement
MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)
WHERE similarity(doc.embedding, $question) > 0.8
RETURN author.name, doc.title
ORDER BY similarity() DESC
LIMIT 5
```

> Full installation guide: [docs/guides/INSTALLATION.md](docs/guides/INSTALLATION.md)

---

## Your First Vector Search

**1. Create a collection**

```bash
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "my_vectors", "dimension": 4, "metric": "cosine"}'
```

**2. Insert vectors with metadata**

```bash
curl -X POST http://localhost:8080/collections/my_vectors/points \
  -H "Content-Type: application/json" \
  -d '{
    "points": [
      {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "AI Introduction", "category": "tech"}},
      {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"title": "ML Basics", "category": "tech"}},
      {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"title": "History of Computing", "category": "history"}}
    ]
  }'
```

**3. Search for similar vectors**

```bash
curl -X POST http://localhost:8080/collections/my_vectors/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [0.9, 0.1, 0.0, 0.0], "top_k": 2}'
```

Response:
```json
{
  "results": [
    {"id": 1, "score": 0.9950, "payload": {"title": "AI Introduction", "category": "tech"}},
    {"id": 2, "score": 0.1104, "payload": {"title": "ML Basics", "category": "tech"}}
  ]
}
```

**4. Search with metadata filter**

```bash
curl -X POST http://localhost:8080/collections/my_vectors/search \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.9, 0.1, 0.0, 0.0],
    "top_k": 5,
    "filter": {"type": "eq", "field": "category", "value": "tech"}
  }'
```

**5. Use VelesQL (SQL-like syntax)**

```bash
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT * FROM my_vectors WHERE vector NEAR $v AND category = '\''tech'\'' LIMIT 5",
    "params": {"v": [0.9, 0.1, 0.0, 0.0]}
  }'
```

**Why VelesQL?** REST endpoints handle one operation at a time. VelesQL combines vector search + graph traversal + metadata filters + aggregation in a single statement:

| Task | REST API | VelesQL |
|------|----------|---------|
| Vector search | `POST .../search` | `SELECT * FROM docs WHERE vector NEAR $v LIMIT 10` |
| Vector + filter | `POST .../search` with `filter` field | `SELECT ... WHERE vector NEAR $v AND category = 'tech' LIMIT 10` |
| Vector + graph | 2 calls (search + traverse), merge in app | `MATCH (doc)-[:CITES]->(ref) WHERE similarity(doc.embedding, $v) > 0.8 RETURN ref` |
| Aggregation | Not available via search endpoints | `SELECT category, COUNT(*) FROM docs GROUP BY category` |

---

## Use Cases

| Use Case                      | VelesDB Feature                     |
|-------------------------------|-------------------------------------|
| **RAG Pipelines**             | Sub-ms retrieval                    |
| **AI Agents**                 | Embedded memory, local context      |
| **Desktop Apps (Tauri/Electron)** | Single binary, no server needed     |
| **Mobile AI (iOS/Android)**   | Native SDKs with 32x memory compression |
| **Browser-side Search**       | WASM module, zero backend           |
| **Edge/IoT Devices**          | 6 MB footprint, ARM NEON optimized  |
| **On-Prem / Air-Gapped**      | No cloud dependency, full data sovereignty |

> **Business scenarios & demos:** [examples/](examples/) — E-commerce recommendation engine, GraphRAG, hybrid search, LangChain/LlamaIndex integration.

---

## Full Ecosystem

VelesDB is designed to run **where your agents live** — from cloud servers to mobile devices to browsers.

| Domain      | Component                          | Description                              | Install                     |
|-------------|------------------------------------|------------------------------------------|----------------------------|
| **Core** | [velesdb-core](crates/velesdb-core) | Core engine (HNSW, SIMD, VelesQL)        | `cargo add velesdb-core`   |
| **Server**| [velesdb-server](crates/velesdb-server) | REST API (37 endpoints, OpenAPI)         | `cargo install velesdb-server` |
| **CLI**  | [velesdb-cli](crates/velesdb-cli)   | Interactive REPL for VelesQL             | `cargo install velesdb-cli` |
| **Python** | [velesdb-python](crates/velesdb-python) | PyO3 bindings + NumPy                    | `pip install velesdb` |
| **TypeScript** | [typescript-sdk](sdks/typescript) | Node.js & Browser SDK                    | `npm install @wiscale/velesdb-sdk` |
| **WASM** | [velesdb-wasm](crates/velesdb-wasm) | Browser-side vector search               | `npm install @wiscale/velesdb-wasm` |
| **Mobile** | [velesdb-mobile](crates/velesdb-mobile) | iOS (Swift) & Android (Kotlin)           | [Build instructions](docs/guides/INSTALLATION.md#-mobile-iosandroid) |
| **Desktop** | [tauri-plugin](crates/tauri-plugin-velesdb) | Tauri v2 AI-powered apps               | `cargo add tauri-plugin-velesdb` |
| **LangChain** | [langchain-velesdb](integrations/langchain) | Official VectorStore                   | [From source](integrations/langchain/README.md) |
| **LlamaIndex** | [llamaindex-velesdb](integrations/llamaindex) | Document indexing                     | [From source](integrations/llamaindex/README.md) |
| **Migration** | [velesdb-migrate](crates/velesdb-migrate) | From Qdrant, Pinecone, Supabase        | `cargo install velesdb-migrate` |

---

## Docker

VelesDB ships with a production-ready Docker image (multi-stage build, non-root user, health check):

```bash
# Quick start
docker run -d -p 8080:8080 -v velesdb_data:/data --name velesdb velesdb/velesdb:latest

# Check health
curl http://localhost:8080/health

# With docker-compose (includes persistence and auto-restart)
docker-compose up -d
```

**Environment variables:**

| Variable | Default | Description |
|---|---|---|
| `VELESDB_DATA_DIR` | `/data` | Data storage directory |
| `VELESDB_HOST` | `0.0.0.0` | Bind address |
| `VELESDB_PORT` | `8080` | HTTP port |
| `RUST_LOG` | `info` | Log level |

**Optimized build** (10-20% faster, uses Rust nightly + LTO):
```bash
docker build -f benchmarks/Dockerfile.optimized -t velesdb:optimized .
```

> Data is persisted in a named volume (`velesdb_data`). Your vectors survive container restarts.

---

## How VelesDB Works

```
INSERT                      INDEX                       SEARCH
┌──────────┐  upsert   ┌──────────────┐  build   ┌──────────────┐
│ Your App │──────────▶│ WAL (append) │────────▶│  HNSW Graph  │
│          │           │ + mmap store │         │  (in-memory) │
└──────────┘           └──────────────┘         └──────┬───────┘
                                                       │
                        RESULT                         │ search
┌──────────┐  top-k    ┌──────────────┐  rank    ┌────▼─────────┐
│ Your App │◀──────────│   Payload    │◀────────│ SIMD Distance│
│          │           │  Hydration   │         │(AVX-512/NEON)│
└──────────┘           └──────────────┘         └──────────────┘
```

1. **Insert**: Vectors + metadata are appended to a Write-Ahead Log (WAL) for crash safety, then stored in memory-mapped files.
2. **Index**: Each vector is inserted into an HNSW graph — a multi-layer structure where similar vectors are connected by edges.
3. **Search**: A query vector enters the HNSW graph. SIMD-accelerated distance kernels (AVX-512, AVX2, or NEON) compare candidates at hardware speed. The `ef_search` parameter controls how many candidates to explore.
4. **Result**: Top-k nearest neighbors are returned with similarity scores and their JSON payloads.

**Key design choices:**
- **Local-first**: Runs in-process or as a single binary — no network hops, no cloud dependency.
- **Memory-mapped storage**: The OS manages paging between RAM and disk. Practical limit = available RAM + swap.
- **WAL durability**: Every write is journaled. Crash-safe by default (`fsync` mode).

---

## Performance

### Core SIMD Operations (768D)

| Operation | Latency | Throughput |
|-----------|---------|------------|
| **SIMD Dot Product (768D)** | **23.6 ns** | **32.5 Gelem/s** |
| **Euclidean** | **22.7 ns** | **33.8 Gelem/s** |
| **Cosine** | **33.6 ns** | **22.9 Gelem/s** |
| **Hamming** | **34.3 ns** | — |
| **Jaccard** | **29.3 ns** | — |

### System Benchmarks (10K Vectors, 768D)

| Benchmark | Result |
|-----------|--------|
| **HNSW Search (10K vectors)** | **42.8 µs** (k=10) |
| **VelesQL Cache Hit** | **1.06 µs** (~943K QPS) |
| **Sparse Search** | **958 µs** (MaxScore DAAT) |
| **Recall@10 (Accurate)** | **100%** |

### Search Quality (Recall)

| Mode | ef_search | Recall@10 | Latency (10K/128D) |
|------|-----------|-----------|---------------------|
| Fast | 64 | 92.2% | 36µs |
| Balanced (default) | 128 | 98.8% | ~102µs |
| Accurate | 512 | 100% | 130µs |
| Perfect | 4096 | 100% | 163µs |
| Adaptive | 32–512 | 95%+ | ~15-40µs (easy queries) |

**Optimizations:** AVX-512/AVX2/NEON auto-detection, 4-accumulator ILP, zero-dispatch DistanceEngine, batch prefetch L1/L2, 64-byte aligned memory, batch WAL, memory-mapped files.

> **Full benchmarks & methodology:** [docs/BENCHMARKS.md](docs/BENCHMARKS.md) | **Quantization guide:** [docs/guides/QUANTIZATION.md](docs/guides/QUANTIZATION.md)

*Measured March 20, 2026 on i9-14900KF, 64GB DDR5, Windows 11, Rust 1.92.0. Criterion.rs, sequential runs on idle machine. Results depend on CPU, SIMD support, and dataset.*

---

## Core Features

| Feature | Technical Capability | Real-World Impact |
|---------|----------------------|-------------------|
| **Vector + Graph Fusion** | Unified query language for semantic + relationship queries | **Build smarter AI agents** with contextual understanding |
| **42.8µs Search** | Native HNSW + AVX-512/AVX2/NEON SIMD | **Create real-time experiences** previously impossible |
| **6 MB Self-Contained** | No external services, single executable | **Deploy anywhere** — from servers to edge devices |
| **Air-Gapped Deployment** | Full functionality without internet | **Meet strict compliance** in healthcare/finance |
| **Everywhere Runtime** | Consistent API across server/mobile/browser | **Massive code reuse** across platforms |
| **PQ + SQ8 Quantization** | 4-32x memory reduction (PQ, SQ8, Binary, RaBitQ) | **Run complex AI** on resource-constrained devices |
| **Hybrid Dense+Sparse** | SPLADE/BM42 sparse index + RRF/RSF fusion | **Best lexical + semantic** retrieval in one query |
| **Adaptive Search** | Two-phase ef_search that starts low and escalates only for hard queries | **2-4x faster median latency** without sacrificing recall |
| **VelesQL** | SQL-like unified query language with SPARSE_NEAR, TRAIN QUANTIZER | **Simplify complex queries** — no DSL learning curve |

---

## API Reference

VelesDB exposes **37 REST endpoints** organized by domain: Collections, Points, Search, Graph, Indexes, VelesQL, and Administration.

| Category | Key Endpoints | Description |
|----------|--------------|-------------|
| **Collections** | `POST /collections`, `GET /collections`, `GET/DELETE /collections/{name}` | Create, list, inspect, delete collections |
| **Points** | `/collections/{name}/points`, `/collections/{name}/stream/insert` | CRUD & streaming insert |
| **Search** | `/collections/{name}/search`, `/collections/{name}/search/batch`, `/collections/{name}/search/hybrid`, `/collections/{name}/search/text`, `/collections/{name}/search/multi`, `/collections/{name}/search/ids`, `/collections/{name}/match` | Vector, sparse, hybrid, text, multi, ID-only search |
| **Graph** | `/collections/{name}/graph/edges`, `/collections/{name}/graph/traverse`, `.../graph/traverse/stream`, `.../graph/nodes/{id}/degree` | Edge CRUD, BFS/DFS traversal, streaming, degree query |
| **Indexes** | `GET/POST .../indexes`, `DELETE .../indexes/{label}/{property}` | Secondary index management |
| **VelesQL** | `/query`, `/aggregate`, `/query/explain` | Unified query language with EXPLAIN |
| **Admin** | `/health`, `/ready`, `/metrics`, `/guardrails`, `/collections/{name}/stats`, `/collections/{name}/config`, `/collections/{name}/flush`, `/collections/{name}/empty`, `/collections/{name}/sanity`, `/collections/{name}/analyze` | Liveness, readiness, Prometheus, collection ops |

<details>
<summary>All 37 REST endpoints</summary>

**Admin:** `GET /health` · `GET /ready` · `GET /metrics`¹ · `GET /guardrails` · `PUT /guardrails`

**Collections:** `GET /collections` · `POST /collections` · `GET /collections/{name}` · `DELETE /collections/{name}` · `GET /collections/{name}/stats` · `GET /collections/{name}/config` · `POST /collections/{name}/analyze` · `POST /collections/{name}/flush` · `GET /collections/{name}/empty` · `GET /collections/{name}/sanity`

**Points:** `POST /collections/{name}/points` · `GET /collections/{name}/points/{id}` · `DELETE /collections/{name}/points/{id}` · `POST /collections/{name}/stream/insert` · `POST /collections/{name}/points/stream`

**Search:** `POST /collections/{name}/search` · `POST /collections/{name}/search/batch` · `POST /collections/{name}/search/text` · `POST /collections/{name}/search/hybrid` · `POST /collections/{name}/search/multi` · `POST /collections/{name}/search/ids` · `POST /collections/{name}/match`

**Graph:** `GET /collections/{name}/graph/edges` · `POST /collections/{name}/graph/edges` · `POST /collections/{name}/graph/traverse` · `GET /collections/{name}/graph/traverse/stream` · `GET /collections/{name}/graph/nodes/{node_id}/degree`

**Indexes:** `GET /collections/{name}/indexes` · `POST /collections/{name}/indexes` · `DELETE /collections/{name}/indexes/{label}/{property}`

**VelesQL:** `POST /query` · `POST /query/explain` · `POST /aggregate`

</details>

¹ `/metrics` requires the `prometheus` feature flag.

> **Full API reference:** [docs/reference/API_REFERENCE.md](docs/reference/API_REFERENCE.md) — all endpoints with request/response examples.
> **OpenAPI spec:** [docs/openapi.yaml](docs/openapi.yaml)

---

## Demos & Examples

### Flagship: E-commerce Recommendation Engine

The ultimate showcase of VelesDB's **Vector + Graph + MultiColumn** combined power:

```bash
cd examples/ecommerce_recommendation
cargo run --release
```

| Query Type | Typical Latency | Description |
|------------|-----------------|-------------|
| **Vector Similarity** | **187 µs** | Find semantically similar products |
| **Vector + Filter** | **55 µs** | In-stock, price < $500, rating >= 4.0 |
| **Graph Lookup** | **88 µs** | Co-purchased products (BOUGHT_TOGETHER) |
| **Combined (Full Power)** | **202 µs** | Union of Vector + Graph + Filters |

*Latencies measured with `Instant::now()` instrumentation in the demo. Results vary by hardware.*

### Other Demos

| Demo | Description | Tech |
|------|-------------|------|
| [**rag-pdf-demo**](demos/rag-pdf-demo/) | PDF document Q&A with RAG | Python, FastAPI |
| [**tauri-rag-app**](demos/tauri-rag-app/) | Desktop RAG application | Tauri v2, React |
| [**wasm-browser-demo**](examples/wasm-browser-demo/) | In-browser vector search | WASM, vanilla JS |
| [**mini_recommender**](examples/mini_recommender/) | Simple product recommendations | Rust |

---

## Security

VelesDB's server component supports **opt-in security features** for deployments beyond localhost:

- **API Key Authentication** — Bearer token auth via `VELESDB_API_KEYS` env var or `[auth]` section in `velesdb.toml`
- **TLS (HTTPS)** — Built-in TLS via rustls. Configure with `VELESDB_TLS_CERT` / `VELESDB_TLS_KEY`
- **Graceful Shutdown** — SIGTERM/SIGINT triggers connection drain + WAL flush. Zero data loss
- **Health Endpoints** — `GET /health` and `GET /ready` always public

> **Full security guide:** [docs/guides/SERVER_SECURITY.md](docs/guides/SERVER_SECURITY.md)

---

## Contributing

```bash
git clone https://github.com/cyberlife-coder/VelesDB.git
cd VelesDB
cargo test --workspace --features persistence,gpu,update-check --exclude velesdb-python -- --test-threads=1
cargo fmt --all && cargo clippy --workspace --all-targets --features persistence,gpu,update-check --exclude velesdb-python -- -D warnings -D clippy::pedantic
```

### Project Structure

```
VelesDB/
├── crates/
│   ├── velesdb-core/     # Core engine library
│   ├── velesdb-server/   # REST API server
│   ├── velesdb-cli/      # Interactive CLI with VelesQL REPL
│   ├── velesdb-wasm/     # WebAssembly module
│   ├── velesdb-python/   # Python bindings (PyO3)
│   ├── velesdb-mobile/   # iOS/Android bindings (UniFFI)
│   ├── velesdb-migrate/  # Migration from Qdrant, Pinecone, Supabase
│   └── tauri-plugin-velesdb/ # Tauri v2 desktop plugin
├── benches/              # Benchmarks
└── docs/                 # Documentation
```

Looking for a place to start? Check out issues labeled [`good first issue`](https://github.com/cyberlife-coder/VelesDB/labels/good%20first%20issue).

---

## Roadmap

| Version | Status | Highlights |
|---------|--------|------------|
| **v1.2.0** | Released | Knowledge Graph, Vector-Graph Fusion, ColumnStore, 15 EPICs |
| **v1.4.0** | Released | VelesQL v2.2.0, Multi-Score Fusion, Parallel Graph, 2,765 tests |
| **v1.6.0** | Released | Product Quantization, Sparse Vectors, Hybrid Search, Streaming Inserts, Query Plan Cache |

<details>
<summary><b>v1.2.0 — 15 EPICs Completed (click to expand)</b></summary>

| EPIC | Feature | Impact |
|------|---------|--------|
| EPIC-001 | Code Quality Refactoring | Clean architecture |
| EPIC-002 | GPU Acceleration (wgpu) | Batch similarity offload |
| EPIC-003 | PyO3 Migration | Python 3.12+ support |
| EPIC-004 | Knowledge Graph Storage | GraphNode, GraphEdge, BFS |
| EPIC-005 | VelesQL MATCH Clause | Cypher-inspired queries |
| EPIC-006 | Agent Toolkit SDK | Python, WASM, Mobile |
| EPIC-007 | Python Bindings Refactoring | Clean API |
| EPIC-008 | Vector-Graph Fusion | `similarity()` in MATCH |
| EPIC-009 | Graph Property Index | 10x faster MATCH |
| EPIC-019 | Scalability 10M entries | Enterprise datasets |
| EPIC-020 | ColumnStore CRUD | Real-time updates |
| EPIC-021 | VelesQL JOIN Cross-Store | Graph <-> Table queries |
| EPIC-028 | ORDER BY Multi-Columns | Complex sorting |
| EPIC-029 | Python SDK Core Delegation | DRY bindings |
| EPIC-031 | Multimodel Query Engine | Unified execution |

</details>

<details>
<summary><b>v1.4.0 — 10 EPICs Completed (click to expand)</b></summary>

| EPIC | Feature | Impact |
|------|---------|--------|
| EPIC-045 | VelesQL MATCH Queries | Graph pattern matching |
| EPIC-046 | EXPLAIN Query Plans | Query optimization |
| EPIC-049 | Multi-Score Fusion | RRF, Average, Weighted |
| EPIC-051 | Parallel Graph Traversal | 2-4x speedup |
| EPIC-052 | VelesQL Enhancements | DISTINCT, Self-JOIN |
| EPIC-056 | VelesQL SDK Propagation | Python/WASM support |
| EPIC-057 | LangChain/LlamaIndex | All metrics & modes |
| EPIC-058 | Server API Completeness | EXPLAIN, SSE Stream |
| EPIC-059 | CLI & Examples | Multi-search, fusion |
| EPIC-060 | E2E Test Coverage | 2,765 tests |

</details>

<details>
<summary><b>v1.6.0 — 5 Major Features (click to expand)</b></summary>

| Feature | Description | Impact |
|---------|-------------|--------|
| **Product Quantization** | PQ (m/k configurable), OPQ, RaBitQ, GPU-accelerated training | 4-32x memory compression |
| **Sparse Vectors** | SPLADE/BM42 inverted index, MaxScore DAAT, WAL persistence | Sub-ms lexical search |
| **Hybrid Dense+Sparse** | RRF/RSF fusion, parallel branch execution, filtered sparse | Best of both worlds |
| **Streaming Inserts** | Bounded channel, backpressure, delta buffer, immediate search | Real-time ingestion |
| **Query Plan Cache** | Two-level LRU, write_generation invalidation, EXPLAIN metrics | Faster repeated queries |

</details>

### Future Vision

| Horizon | Features |
|---------|----------|
| **2026 H2** | Distributed mode, Multi-tenancy |
| **2027** | Cluster HA, Agent Hooks & Triggers |

---

<details>
<summary><b>Persistence & Durability</b></summary>

All data is persisted to disk by default. No configuration needed for basic durability.

| Question | Answer |
|----------|--------|
| **Is data persisted?** | Yes. Vectors, payloads, and HNSW index are stored in the `--data-dir` directory. |
| **What if the process crashes?** | The WAL replays uncommitted operations on restart. Default mode is `fsync` (power-loss safe). |
| **What if I kill the server?** | Graceful shutdown (SIGTERM / Ctrl+C) drains connections and flushes WALs before exit. |
| **Where is my data?** | Each collection is a directory: `vectors.bin`, `payloads.log`, `hnsw.bin`, WAL files. |
| **Can I trade durability for speed?** | Yes. Bulk imports can disable `fsync` for higher throughput, then call `flush()`. |

</details>

<details>
<summary><b>Project Quality</b></summary>

<table align="center">
<tr>
<td align="center" width="20%"><h3>4,300+</h3><p><strong>Tests</strong></p></td>
<td align="center" width="20%"><h3>82.30%</h3><p><strong>Coverage</strong></p></td>
<td align="center" width="20%"><h3>0</h3><p><strong>Security Issues</strong></p></td>
<td align="center" width="20%"><h3>60K</h3><p><strong>Rust LoC</strong></p></td>
<td align="center" width="20%"><h3>8</h3><p><strong>Crates</strong></p></td>
</tr>
</table>

```
cargo check --workspace          cargo clippy -- -D warnings
cargo test --workspace (4,300+)  cargo deny check (0 advisories)
cargo fmt --check                Coverage > 75% (82.30%)
```

</details>

<details>
<summary><b>Known Limitations</b></summary>

| Limitation | Details | Workaround |
|------------|---------|------------|
| **Single-node only** | No distributed mode, no sharding, no replication | Planned for 2026 H2 |
| **No multi-operation transactions** | Each upsert/delete is individually WAL-backed, but no BEGIN/COMMIT | Design idempotent operations |
| **Dataset size ≈ RAM** | Vector data is memory-mapped; practical limit is available RAM + swap | Use SQ8 (4x) or Binary (32x) quantization |
| **No real-time replication** | No built-in primary/replica or CDC | Backup the data directory; HA planned for 2027 |
| **VelesQL subset** | No subquery execution, no UPDATE/DELETE WHERE, no CTEs | Use the programmatic API for mutations |

</details>

<details>
<summary><b>New to VelesDB? Quick Diagnosis</b></summary>

| Symptom | Probable cause | Immediate fix |
|---|---|---|
| `MATCH` query fails or behaves unlike SQL | VelesQL is SQL-like, not PostgreSQL/MySQL-compatible SQL | Start from VelesQL examples, then adapt queries incrementally |
| Query returns 0 rows | Vector dimension mismatch, strict threshold, or empty collection | Verify dimension first, remove strict filters, retest |
| Can't change metric/dimension after create | Collection schema is intentionally immutable for HNSW/SIMD performance | Create a new collection and reindex data |
| Installed `velesdb-core` but no HTTP endpoint/REPL | `velesdb-core` is embedded engine only | Add `velesdb-server` (HTTP) and/or `velesdb-cli` (REPL) |

**Fast sanity sequence:** (1) Confirm embedding dimension == collection dimension. (2) Run a permissive nearest-neighbor query (no strict filters). (3) Validate at least one known vector is retrievable. (4) Add filters progressively. (5) Tune `ef_search` after functional correctness is validated.

> **Full troubleshooting guide:** [docs/NEW_USER_TROUBLESHOOTING.md](docs/NEW_USER_TROUBLESHOOTING.md)

</details>

<details>
<summary><b>Update Check</b></summary>

VelesDB checks for updates at startup (similar to VS Code, Firefox). Only version, OS/arch, and an anonymous instance hash are sent. No IP logging, no query content, no PII.

```bash
# Disable update check
export VELESDB_NO_UPDATE_CHECK=1
```

</details>

---

## License

**VelesDB Core License 1.0** (source-available) — `velesdb-core` and `velesdb-server`. Free use, modification, and distribution with restrictions only on managed services and competing products. See [LICENSE](LICENSE).

**MIT** — all other components: CLI, Python/WASM/mobile bindings, Tauri plugin, TypeScript SDK, LangChain/LlamaIndex integrations, migration tool, examples, and demos.

---

## Support VelesDB

<p align="center">
  <a href="https://github.com/cyberlife-coder/VelesDB">
    <img src="https://img.shields.io/badge/Star_on_GitHub-181717?style=for-the-badge&logo=github" alt="Star on GitHub"/>
  </a>
  <a href="https://twitter.com/intent/tweet?text=Check%20out%20VelesDB%20-%20The%20Local%20Knowledge%20Engine%20for%20AI%20Agents!%20Vector%20%2B%20Graph%20%2B%20ColumnStore%20in%20one%206MB%20binary.&url=https://github.com/cyberlife-coder/VelesDB&hashtags=VectorDatabase,AI,Rust,OpenSource">
    <img src="https://img.shields.io/badge/Share_on_Twitter-1DA1F2?style=for-the-badge&logo=twitter&logoColor=white" alt="Share on Twitter"/>
  </a>
</p>

[![Star History Chart](https://api.star-history.com/svg?repos=cyberlife-coder/velesdb&type=Date)](https://star-history.com/#cyberlife-coder/velesdb&Date)

**Enterprise, partnerships & investment** — contact@wiscale.fr

[![Powered by VelesDB](https://img.shields.io/badge/Powered_by-VelesDB-blue?style=flat-square)](https://github.com/cyberlife-coder/VelesDB)

```markdown
[![Powered by VelesDB](https://img.shields.io/badge/Powered_by-VelesDB-blue?style=flat-square)](https://github.com/cyberlife-coder/VelesDB)
```

---

<p align="center">
  <strong>Built with Rust</strong>
</p>

<p align="center">
  <strong>Original Author:</strong> <a href="https://github.com/cyberlife-coder">Julien Lange</a> — <a href="https://wiscale.fr"><strong>WiScale</strong></a>
</p>

<p align="center">
  <a href="https://github.com/cyberlife-coder/VelesDB">GitHub</a> &bull;
  <a href="https://velesdb.com/en/">Documentation</a> &bull;
  <a href="https://github.com/cyberlife-coder/VelesDB/issues">Issues</a> &bull;
  <a href="https://github.com/cyberlife-coder/VelesDB/releases">Releases</a>
</p>

<p align="center">
  <sub>Star the repo if you find VelesDB useful!</sub>
</p>
