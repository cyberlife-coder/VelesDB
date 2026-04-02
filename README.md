<p align="center">
  <img src="velesdb_icon_pack/favicon/android-chrome-512x512.png" alt="VelesDB Logo" width="200"/>
</p>
<h1 align="center">
  <img src="velesdb_icon_pack/favicon/favicon-32x32.png" alt="VelesDB" width="32" height="32" style="vertical-align: middle;"/>
</h1>
<h3 align="center">
  Your AI agents forget everything. VelesDB fixes that.
</h3>
<p align="center">
  <strong>One 6 MB binary. Three engines. One query language. Zero cloud dependency.</strong><br/>
  <em>Vector + Graph + ColumnStore — unified under <a href="docs/VELESQL_SPEC.md">VelesQL</a></em>
</p>
<p align="center">
  <a href="https://github.com/cyberlife-coder/VelesDB/actions/workflows/ci.yml"><img src="https://github.com/cyberlife-coder/VelesDB/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://app.codacy.com/gh/cyberlife-coder/VelesDB/dashboard?utm_source=gh&utm_medium=referral&utm_content=&utm_campaign=Badge_grade"><img src="https://app.codacy.com/project/badge/Grade/58c73832dd294ba38144856ae69e9cf2" alt="Codacy Badge"></a>
  <a href="https://crates.io/crates/velesdb-core"><img src="https://img.shields.io/crates/v/velesdb-core.svg" alt="Crates.io"></a>
  <a href="https://crates.io/crates/velesdb-core"><img src="https://img.shields.io/crates/d/velesdb-core.svg" alt="Crates.io Downloads"></a>
  <a href="https://pypi.org/project/velesdb/"><img src="https://img.shields.io/pypi/v/velesdb.svg" alt="PyPI"></a>
  <a href="https://www.npmjs.com/package/@wiscale/velesdb-sdk"><img src="https://img.shields.io/npm/v/@wiscale/velesdb-sdk.svg" alt="npm"></a>
  <a href="https://app.codacy.com/gh/cyberlife-coder/VelesDB/dashboard"><img src="https://app.codacy.com/project/badge/Coverage/58c73832dd294ba38144856ae69e9cf2" alt="Coverage"></a>
  <img src="https://img.shields.io/badge/tests-5495_(incl._203_BDD)-brightgreen" alt="Tests">
  <a href="https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-VelesDB_Core_1.0-blue" alt="License"></a>
  <a href="https://github.com/cyberlife-coder/VelesDB"><img src="https://img.shields.io/github/stars/cyberlife-coder/VelesDB?style=flat-square" alt="Stars"></a>
  <a href="https://img.shields.io/badge/contributors-welcome-brightgreen"><img src="https://img.shields.io/badge/contributors-welcome-brightgreen" alt="Contributors Welcome"></a>
</p>
<p align="center">
  <a href="https://github.com/cyberlife-coder/VelesDB/releases/tag/v1.11.0">Download v1.11.0</a> &bull;
  <a href="#getting-started-in-60-seconds">Quick Start</a> &bull;
  <a href="https://velesdb.com/en/">Documentation</a> &bull;
  <a href="https://deepwiki.com/cyberlife-coder/VelesDB">DeepWiki</a>
</p>

<!-- TODO: Uncomment when GIF demo is ready
<p align="center">
  <img src="docs/assets/velesdb-demo.gif" alt="VelesDB Demo" width="700"/>
</p>
-->

---

> **Every AI agent today stitches together 3 databases for memory — vectors for "what feels similar", a graph for "what is connected", and SQL for "what I know for sure". That's 3 deployments, 3 configs, 3 query languages, and a pile of glue code.**
>
> **VelesDB replaces all of that with a single Rust binary that fits on a floppy disk.**

---

## Why VelesDB?

| Today (3 systems to maintain) | With VelesDB (1 binary) |
|-------------------------------|------------------------|
| pgvector for embeddings | **Vector Engine** — 47us HNSW search (768D) |
| Neo4j for knowledge graphs | **Graph Engine** — MATCH clause, BFS/DFS |
| PostgreSQL/DuckDB for metadata | **ColumnStore** — 130x faster than JSON at 100K rows |
| Custom glue code + 3 query languages | **VelesQL** — one language for everything |
| 3 deployments, 3 configs, 3 backups | **6 MB binary** — works offline, air-gapped |

---
## What is VelesDB?

VelesDB is a **local-first database for AI agents** that fuses three engines into a single 6 MB binary:

| Engine | What it does | Performance |
|--------|-------------|-------------|
| **Vector** | Semantic similarity search (HNSW + AVX2/NEON SIMD) | **450us** p50 end-to-end (384D, WAL ON, recall>=96%) |
| **Graph** | Knowledge relationships (BFS/DFS, edge properties) | Native **MATCH** clause |
| **ColumnStore** | Structured metadata filtering (typed columns) | **130x** faster than JSON scanning |

All three are queried through **VelesQL** — a single SQL-like language with vector, graph, and columnar extensions:

```sql
MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)
WHERE similarity(doc.embedding, $question) > 0.8
  AND author.department = 'Engineering'
RETURN author.name, doc.title
ORDER BY similarity() DESC LIMIT 5
```

**Built-in Agent Memory SDK** provides semantic, episodic, and procedural memory for AI agents — no external services needed.

> **One binary. No cloud. No glue code. Runs on server, browser, mobile, and desktop.**

---

## Three Engines, One Query

<table align="center">
<tr>
<td align="center" width="33%">
<h3>Vector Engine</h3>
<p>Native HNSW + AVX-512/AVX2/NEON SIMD<br/><strong>47us search (768D), 19.8ns dot product</strong></p>
<p><em>Semantic similarity, embeddings, RAG retrieval</em></p>
</td>
<td align="center" width="33%">
<h3>Graph Engine</h3>
<p>Property graph with BFS/DFS traversal<br/><strong>MATCH clause, edge properties</strong></p>
<p><em>Knowledge graphs, citations, co-purchase</em></p>
</td>
<td align="center" width="33%">
<h3>ColumnStore Engine</h3>
<p>Typed columnar storage with bitmap filters<br/><strong>130x faster than JSON at 100K rows</strong></p>
<p><em>Metadata filters, reference tables, catalogs</em></p>
</td>
</tr>
</table>

**The power is in the fusion.** VelesQL combines all three in a single statement:

```sql
-- Vector similarity + Graph traversal + ColumnStore filter — ONE query
MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)
WHERE similarity(doc.embedding, $question) > 0.8
  AND author.department = 'Engineering'
RETURN author.name, doc.title
ORDER BY similarity() DESC
LIMIT 5
```

---

## Agent Memory SDK

Built-in memory subsystems for AI agents — no external vector DB, no graph DB, no extra dependencies. **99 tests** cover the SDK end-to-end.

```python
from velesdb import Database, AgentMemory

db = Database("./agent_data")
memory = AgentMemory(db, dimension=384)
```

### Three Memory Types

| Memory | Purpose | Key methods |
|--------|---------|-------------|
| **Semantic** | Long-term knowledge facts | `store`, `query`, `delete`, `store_with_ttl` |
| **Episodic** | Event timeline with context | `record`, `recent`, `older_than`, `recall_similar`, `delete` |
| **Procedural** | Learned patterns & actions | `learn`, `recall`, `reinforce`, `list_all`, `delete` |

### Semantic Memory — What the agent knows

```python
memory.semantic.store(1, "Paris is the capital of France", embedding)
results = memory.semantic.query(query_embedding, top_k=5)
memory.semantic.delete(1)  # Remove outdated knowledge
```

### Episodic Memory — What happened and when

```python
memory.episodic.record(1, "User asked about geography", int(time.time()), embedding)
events = memory.episodic.recent(limit=10)
old_events = memory.episodic.older_than(cutoff_timestamp, limit=50)
similar = memory.episodic.recall_similar(query_embedding, top_k=5)
memory.episodic.delete(1)
```

### Procedural Memory — What the agent learned to do

```python
memory.procedural.learn(
    procedure_id=1, name="answer_geography",
    steps=["search memory", "retrieve facts", "compose answer"],
    embedding=task_embedding, confidence=0.8
)
matches = memory.procedural.recall(task_embedding, top_k=3, min_confidence=0.5)
all_procedures = memory.procedural.list_all()
memory.procedural.reinforce(procedure_id=1, success=True)   # confidence +0.1
memory.procedural.delete(1)
```

### Advanced features

| Feature | API |
|---------|-----|
| **TTL / Auto-expiration** | `store_with_ttl()`, `record_with_ttl()`, `learn_with_ttl()`, `auto_expire()` |
| **Snapshots / Rollback** | `snapshot()`, `load_latest_snapshot()`, `list_snapshot_versions()` |
| **Confidence eviction** | `evict_low_confidence_procedures(min_confidence)` |
| **Reinforcement strategies** | `FixedRate`, `AdaptiveLearningRate`, `TemporalDecay`, `ContextualReinforcement` |
| **Serialization** | `serialize()` / `deserialize()` on all memory types |

<details>
<summary>Why not SQLite + Vector DB + Graph DB?</summary>

| | **VelesDB Agent Memory** | SQLite + pgvector + Neo4j |
|---|---|---|
| **Dependencies** | 0 (single binary) | 3 separate engines |
| **Setup** | `pip install velesdb` | Install, configure, connect each |
| **Semantic search** | Native HNSW (sub-ms) | Requires separate vector DB |
| **Temporal queries** | Built-in B-tree index | Manual SQL schema |
| **Confidence scoring** | 4 reinforcement strategies | Build from scratch |
| **TTL / Auto-expiration** | Built-in | Manual cleanup jobs |
| **Snapshots / Rollback** | Versioned with CRC32 | Custom backup logic |

</details>

> **[Full guide: embedding setup, retrieval patterns, TTL, snapshots](docs/guides/AGENT_MEMORY.md)** | [Source code](crates/velesdb-core/src/agent/)

---

## Quick Comparison

| | **VelesDB** | Chroma | Qdrant | pgvector |
|---|---|---|---|---|
| **Architecture** | Unified vector + graph + columnar | Vector only | Vector + payload | Vector extension for PostgreSQL |
| **Metadata filtering** | **ColumnStore (130x vs JSON)** | JSON scan | JSON payload | SQL (PostgreSQL) |
| **Deployment** | Embedded / Server / WASM / Mobile | Server (Python) | Server (Rust) | Requires PostgreSQL |
| **Binary size** | 6 MB | ~500 MB (with deps) | ~50 MB | N/A (PG extension) |
| **Search latency** | **450us** p50 (10K/384D, WAL ON, recall>=96%) | ~1-5ms | ~1-5ms (in-memory) | ~5-20ms |
| **Graph support** | Native (MATCH clause) | No | No | No |
| **Query language** | VelesQL (SQL + NEAR + MATCH) | Python API | JSON API / gRPC | SQL + operators |
| **Browser (WASM)** | Yes | No | No | No |
| **Mobile (iOS/Android)** | Yes | No | No | No |
| **Offline / Local-first** | Yes | Partial | No | No |

> *Competitor latencies are typical ranges from public benchmarks and vendor documentation. Direct comparison is approximate — architectures differ (embedded vs client-server, durable vs in-memory, recall levels). Run your own benchmarks for accurate comparison.*

> **VelesDB's sweet spot:** When you need vector + graph + structured filtering in a single engine, local-first deployment, or a lightweight binary that runs anywhere.
>
> **Not the best fit (yet):** If you need a managed cloud service with a multi-node distributed cluster.

---

## Getting Started in 60 Seconds

### Install

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
# Build the image locally
git clone https://github.com/cyberlife-coder/VelesDB.git && cd VelesDB
docker build -t velesdb .

# Run with persistent data (named volume)
docker run -d -p 8080:8080 -v velesdb_data:/data --name velesdb velesdb

# Verify it's running
curl http://localhost:8080/health
```
Data is stored in the `/data` directory inside the container. The named volume `velesdb_data` persists data across container restarts. The built-in health check polls `GET /health` every 30 seconds.

<details>
<summary>More install options (Docker Compose, WASM, install scripts)</summary>

**Docker Compose:**
```bash
git clone https://github.com/cyberlife-coder/VelesDB.git && cd VelesDB
docker-compose up -d
```

| Environment variable | Default | Description |
|---|---|---|
| `VELESDB_DATA_DIR` | `/data` | Data storage directory |
| `VELESDB_HOST` | `0.0.0.0` | Bind address |
| `VELESDB_PORT` | `8080` | HTTP port |
| `RUST_LOG` | `info` | Log level (`debug`, `info`, `warn`, `error`) |

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

</details>

### First search in 30 seconds

```bash
velesdb-server --data-dir ./my_data &

# Create collection + insert + search
curl -X POST http://localhost:8080/collections \
  -d '{"name": "docs", "dimension": 4, "metric": "cosine"}' -H "Content-Type: application/json"

curl -X POST http://localhost:8080/collections/docs/points \
  -d '{"points": [
    {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "AI Intro", "category": "tech"}},
    {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"title": "ML Basics", "category": "tech"}},
    {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"title": "History of Computing", "category": "history"}}
  ]}' -H "Content-Type: application/json"

curl -X POST http://localhost:8080/collections/docs/search \
  -d '{"vector": [0.9, 0.1, 0.0, 0.0], "top_k": 2}' -H "Content-Type: application/json"
# [{"id":1,"score":0.995,"payload":{"title":"AI Intro","category":"tech"}}, ...]
```

> Full installation guide: [docs/guides/INSTALLATION.md](docs/guides/INSTALLATION.md)

---

## Vector Engine

Native HNSW index with SIMD-accelerated distance kernels. Sub-millisecond search on commodity hardware.

### Performance (v1.9.1)

End-to-end numbers on the **complete production path**: WAL durability, payload storage, HNSW search, result resolution. No shortcuts.

#### Full production path (Python SDK, WAL ON, recall@10 >= 96%)

| Dataset | Bulk insert | Search p50 | Search p99 | Recall@10 | DB size |
|---------|-------------|-----------|------------|-----------|---------|
| 10K x 384D | **18.5K vec/s** | **450 us** | **670 us** | >= 96% | 31 MB |
| 50K x 384D | **5.9K vec/s** | **1.1 ms** | **1.4 ms** | >= 96% | 162 MB |

#### Core engine (Rust, index-only, no WAL/payload overhead)

| Benchmark | Result |
|-----------|--------|
| HNSW Search (5K/768D, k=10) | **55 us** |
| SIMD Dot Product (768D, AVX2) | **21.7 ns** |
| SIMD Euclidean (768D, AVX2) | **20.1 ns** |
| Parallel insert (1K/768D) | **48.2K vec/s** |
| Recall@10 (Balanced) | **98.8%** |
| Recall@10 (Accurate) | **100%** |

<details>
<summary>What these numbers mean</summary>

- **Full production path**: measures the real user experience — Python SDK call, WAL write, HNSW search, payload + vector retrieval. This is what your application actually sees. Measured with `benchmarks/velesdb_benchmark.py --recall`.
- **Core engine**: measures the Rust index layer in isolation (Criterion.rs, sequential runs). Useful for architecture comparisons but not representative of end-to-end latency.
- **Recall@10 >= 96%**: measured on synthetic clustered datasets (50 Gaussian clusters, 384D). Real-world recall depends on your data distribution — run the benchmark on your own dataset to verify. We use `Balanced` mode (ef_search=128) which prioritizes recall over raw speed.
- **WAL ON**: every insert is durable. A crash at any point recovers all committed data.
- **Reproduce these numbers**: `pip install velesdb numpy && python benchmarks/velesdb_benchmark.py --recall`

</details>

*Measured 2026-03-27 on i9-14900KF, 64 GB DDR5, Windows 11, Rust 1.92.0, AVX2. [Benchmark script](benchmarks/velesdb_benchmark.py).*

### Search quality modes

| Mode | ef_search | Recall@10 | Use case |
|------|-----------|-----------|----------|
| Fast | 64 | 92.2% | Real-time suggestions, typeahead |
| Balanced (default) | 128 | 98.8% | Production search, RAG pipelines |
| Accurate | 512 | 100% | Evaluation, ground truth comparison |
| **AutoTune** | auto | 95%+ | Adapts to collection size and dimension |
| Adaptive | 32-512 | 95%+ | Hard-query escalation (two-phase) |

### Vector search with metadata filter

```bash
curl -X POST http://localhost:8080/collections/docs/search \
  -d '{"vector": [0.9, 0.1, 0.0, 0.0], "top_k": 5,
       "filter": {"type": "eq", "field": "category", "value": "tech"}}' \
  -H "Content-Type: application/json"
```

```sql
-- Same query in VelesQL
SELECT * FROM docs WHERE vector NEAR $v AND category = 'tech' LIMIT 5
```

### Quantization

4-32x memory reduction with minimal recall loss:

| Method | Compression | Recall impact |
|--------|-------------|---------------|
| **SQ8** | 4x | < 1% loss |
| **PQ** (m=8, k=256) | 32x | ~5% loss (with rescore) |
| **Binary** | 32x | For Hamming-compatible workloads |
| **RaBitQ** | 32x | Random rotation + binary codes ([arXiv:2405.12497](https://arxiv.org/abs/2405.12497)). Dual-precision HNSW: binary traversal + f32 rerank |

> **Full benchmarks:** [docs/BENCHMARKS.md](docs/BENCHMARKS.md) | **Quantization guide:** [docs/guides/QUANTIZATION.md](docs/guides/QUANTIZATION.md)

*Quantization benchmarks: Criterion.rs, sequential runs on idle machine. Same hardware as above.*

---

## Graph Engine

Property graph with BFS/DFS traversal, edge labels, and Cypher-inspired MATCH queries — all integrated with vector search.

### Graph operations

```bash
# Add edges between documents
curl -X POST http://localhost:8080/collections/docs/graph/edges \
  -d '{"source": 1, "target": 2, "label": "CITES", "properties": {"weight": 0.9}}' \
  -H "Content-Type: application/json"

# Traverse the graph
curl -X POST http://localhost:8080/collections/docs/graph/traverse \
  -d '{"start_node": 1, "direction": "outgoing", "max_depth": 3}' \
  -H "Content-Type: application/json"
```

### Vector + Graph fusion (VelesQL)

The query that defines VelesDB — semantic similarity AND relationship traversal in ONE statement:

```sql
-- Find authors of documents similar to my question
MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)
WHERE similarity(doc.embedding, $question) > 0.8
RETURN author.name, doc.title
ORDER BY similarity() DESC
LIMIT 5

-- Co-purchase recommendations
MATCH (product)-[:BOUGHT_TOGETHER]->(related)
WHERE similarity(product.embedding, $query) > 0.7
RETURN related.name, related.price
```

> **Note**: MATCH operates within a single collection. Labels like `Document` and `Person` are tags stored in each point's `_labels` payload array -- all points and edges live in the same collection. See the [Graph Patterns Guide](docs/guides/GRAPH_PATTERNS.md) for setup details and requirements.

### Why VelesQL?

| Task | REST API | VelesQL |
|------|----------|---------|
| Vector search | `POST .../search` | `SELECT * FROM docs WHERE vector NEAR $v LIMIT 10` |
| Vector + filter | `POST .../search` with `filter` | `SELECT ... WHERE vector NEAR $v AND category = 'tech'` |
| Vector + graph | 2 calls + app-side merge | `MATCH ... WHERE similarity() > 0.8 RETURN ...` |
| Aggregation | Not available | `SELECT category, COUNT(*) FROM docs GROUP BY category` |

---

## ColumnStore Engine

Most vector databases store metadata as JSON blobs and scan them linearly. VelesDB's ColumnStore uses typed columns — the same approach analytical databases (DuckDB, ClickHouse) use for fast filtering.

```
Traditional (JSON):     VelesDB (ColumnStore):
┌──────────────────┐    ┌──────────┬───────┬────────┬──────────┐
│ {"category":"tech"│    │ category │ price │ rating │ in_stock │
│  "price": 29.99, │    ├──────────┼───────┼────────┼──────────┤
│  "rating": 4.5,  │    │ "tech"   │ 29.99 │ 4.5    │ true     │
│  "in_stock": true│    │ "science"│ 49.99 │ 4.2    │ false    │
│ }                │    │ "tech"   │ 19.99 │ 4.8    │ true     │
│ Parse every row  │    │ Scan one column   │        │          │
└──────────────────┘    └──────────┴───────┴────────┴──────────┘
    3.84 ms @ 100K           29.5 us @ 100K (130x faster)
```

### Benchmarks vs JSON scanning

| Scale | ColumnStore (int eq) | JSON Scan | Speedup |
|-------|---------------------|-----------|---------|
| 1K rows | 0.34 us | 16.2 us | **48x** |
| 10K rows | 2.95 us | 162.7 us | **55x** |
| 100K rows | 29.5 us | 3.84 ms | **130x** |

### Supported types and filter operations

| Type | Filter operations |
|------|-------------------|
| **Int** (`i64`) | Eq, Gt, Lt, Range, In |
| **Float** (`f64`) | Eq, Gt, Lt, Range |
| **String** (interned) | Eq, In (O(1) via string IDs) |
| **Bool** | Eq |

### MetadataCollection: ColumnStore without vectors

For reference tables, catalogs, and lookup data that don't need vector search:

```rust
let pricing = db.create_metadata_collection("pricing")?;
pricing.upsert(vec![
    Point::metadata_only(1, json!({"product": "Widget A", "price": 29.99})),
    Point::metadata_only(2, json!({"product": "Widget B", "price": 49.99})),
])?;
let results = pricing.execute_query_str("SELECT * FROM pricing WHERE price < 40 LIMIT 10", &params)?;
```

### Vector + ColumnStore: filtered search

```sql
-- Find similar products that are in stock and under $50
SELECT * FROM products
WHERE vector NEAR $query_embedding
  AND in_stock = true
  AND price < 50.0
  AND category IN ('electronics', 'gadgets')
LIMIT 10
```

ColumnStore filters are applied as pre-filters or post-filters depending on selectivity, automatically optimized by the query planner.

---

## Use Cases

| Use Case | VelesDB Feature |
|----------|-----------------|
| **RAG Pipelines** | Vector search + ColumnStore metadata filters |
| **AI Agents** | [Agent Memory SDK](#agent-memory-sdk) — semantic, episodic, procedural memory |
| **E-commerce** | Vector similarity + price/stock ColumnStore filters + co-purchase graph |
| **Desktop Apps (Tauri/Electron)** | Single binary, no server needed |
| **Mobile AI (iOS/Android)** | Native SDKs with 32x memory compression |
| **Browser-side Search** | WASM module, zero backend |
| **Edge/IoT Devices** | 6 MB footprint, ARM NEON optimized |
| **On-Prem / Air-Gapped** | No cloud dependency, full data sovereignty |

---

## Full Ecosystem

| Domain | Component | Install |
|--------|-----------|---------|
| **Core** | [velesdb-core](crates/velesdb-core) — Vector + Graph + ColumnStore + VelesQL | `cargo add velesdb-core` |
| **Server** | [velesdb-server](crates/velesdb-server) — REST API (37 endpoints, OpenAPI) | `cargo install velesdb-server` |
| **CLI** | [velesdb-cli](crates/velesdb-cli) — Interactive VelesQL REPL | `cargo install velesdb-cli` |
| **Python** | [velesdb-python](crates/velesdb-python) — PyO3 bindings + NumPy | `pip install velesdb` |
| **TypeScript** | [typescript-sdk](sdks/typescript) — Node.js & Browser SDK | `npm install @wiscale/velesdb-sdk` |
| **WASM** | [velesdb-wasm](crates/velesdb-wasm) — Browser-side vector search | `npm install @wiscale/velesdb-wasm` |
| **Mobile** | [velesdb-mobile](crates/velesdb-mobile) — iOS (Swift) & Android (Kotlin) | [Build instructions](docs/guides/INSTALLATION.md#-mobile-iosandroid) |
| **Desktop** | [tauri-plugin](crates/tauri-plugin-velesdb) — Tauri v2 AI-powered apps | `cargo add tauri-plugin-velesdb` |
| **LangChain** | [langchain-velesdb](integrations/langchain) — Official VectorStore | [From source](integrations/langchain/README.md) |
| **LlamaIndex** | [llamaindex-velesdb](integrations/llamaindex) — Document indexing | [From source](integrations/llamaindex/README.md) |
| **Migration** | [velesdb-migrate](crates/velesdb-migrate) — From Qdrant, Pinecone, Supabase | `cargo install velesdb-migrate` |

---

## How VelesDB Works

```
INSERT                      INDEX                       SEARCH
┌──────────┐  upsert   ┌──────────────┐  build   ┌──────────────┐
│ Your App │──────────> │ WAL (append) │────────> │  HNSW Graph  │
│          │           │ + mmap store │         │  (in-memory) │
└──────────┘           └──────┬───────┘         └──────┬───────┘
                              │                        │
                       ┌──────▼───────┐                │ search
                       │  ColumnStore  │  filter   ┌────▼─────────┐
                       │ (typed cols)  │────────> │ SIMD Distance│
                       └──────────────┘          │(AVX-512/NEON)│
                        RESULT                    └──────┬───────┘
┌──────────┐  top-k    ┌──────────────┐  rank           │
│ Your App │<──────────│   Payload    │<────────────────┘
│          │           │  Hydration   │
└──────────┘           └──────────────┘
```

**Key design choices:**
- **Local-first**: In-process or single binary — no network hops, no cloud dependency
- **Memory-mapped storage**: OS manages paging between RAM and disk
- **WAL durability**: Every write is journaled. Crash-safe by default (`fsync` mode). Deferred sync during bulk insert for throughput
- **ColumnStore**: Typed columns with string interning, RoaringBitmap tombstones, PostgreSQL-inspired auto-vacuum

<details>
<summary>Docker deployment</summary>

```bash
# Build and run locally
docker build -t velesdb .
docker run -d -p 8080:8080 -v velesdb_data:/data --name velesdb velesdb
curl http://localhost:8080/health

# Or with docker-compose (builds + auto-restart)
docker-compose up -d
```

| Variable | Default | Description |
|---|---|---|
| `VELESDB_DATA_DIR` | `/data` | Data storage directory |
| `VELESDB_HOST` | `0.0.0.0` | Bind address |
| `VELESDB_PORT` | `8080` | HTTP port |
| `RUST_LOG` | `info` | Log level |

The container runs as a non-root `velesdb` user. Data persists via the named volume `velesdb_data`. A built-in health check (`GET /health`) is configured with a 30-second interval.

</details>

<details>
<summary>API Reference (37 REST endpoints)</summary>

| Category | Key Endpoints |
|----------|--------------|
| **Collections** | `POST /collections`, `GET /collections`, `GET/DELETE /collections/{name}` |
| **Points** | `/collections/{name}/points`, `/collections/{name}/stream/insert` |
| **Search** | `/collections/{name}/search`, `/collections/{name}/search/batch`, `/collections/{name}/search/hybrid`, `/collections/{name}/search/text`, `/collections/{name}/search/multi`, `/collections/{name}/search/ids`, `/collections/{name}/match` |
| **Graph** | `/collections/{name}/graph/edges`, `/collections/{name}/graph/traverse`, `/collections/{name}/graph/traverse/stream`, `/collections/{name}/graph/nodes/{id}/degree` |
| **Indexes** | `GET/POST /collections/{name}/indexes`, `DELETE /collections/{name}/indexes/{label}/{property}` |
| **VelesQL** | `/query`, `/aggregate`, `/query/explain` |
| **Admin** | `/health`, `/ready`, `/metrics`, `/guardrails`, `/collections/{name}/stats`, `/collections/{name}/config`, `/collections/{name}/flush`, `/collections/{name}/analyze`, `/collections/{name}/empty`, `/collections/{name}/sanity` |

> **Full API reference:** [docs/reference/api-reference.md](docs/reference/api-reference.md) | **OpenAPI spec:** [docs/openapi.yaml](docs/openapi.yaml)

</details>

<details>
<summary>Security</summary>

- **API Key Authentication** — Bearer token auth via `VELESDB_API_KEYS` env var
- **TLS (HTTPS)** — Built-in via rustls (`VELESDB_TLS_CERT` / `VELESDB_TLS_KEY`)
- **Graceful Shutdown** — SIGTERM triggers connection drain + WAL flush. Zero data loss
- **Health Endpoints** — `GET /health` and `GET /ready` always public

> [docs/guides/SERVER_SECURITY.md](docs/guides/SERVER_SECURITY.md)

</details>

---

## Demos & Examples

### Flagship: E-commerce Recommendation Engine

The ultimate showcase of **Vector + Graph + ColumnStore** combined:

```bash
cd examples/ecommerce_recommendation && cargo run --release
```

| Query Type | Latency | Description |
|------------|---------|-------------|
| **Vector Similarity** | 187 us | Semantically similar products |
| **Vector + ColumnStore** | 55 us | In-stock, price < $500, rating >= 4.0 |
| **Graph Lookup** | 88 us | Co-purchased products (BOUGHT_TOGETHER) |
| **Combined** | 202 us | Vector + Graph + ColumnStore Filters |

<details>
<summary>Other demos</summary>

| Demo | Description | Tech |
|------|-------------|------|
| [**rag-pdf-demo**](demos/rag-pdf-demo/) | PDF document Q&A with RAG | Python, FastAPI |
| [**tauri-rag-app**](demos/tauri-rag-app/) | Desktop RAG application | Tauri v2, React |
| [**wasm-browser-demo**](examples/wasm-browser-demo/) | In-browser vector search | WASM, vanilla JS |
| [**mini_recommender**](examples/mini_recommender/) | Simple product recommendations | Rust |

</details>

---

## Research Foundations

VelesDB's performance is built on peer-reviewed research. Every technique listed below is **implemented and production-active** in the codebase.

### Vector Search Core

| Technique | Paper | What it does in VelesDB |
|-----------|-------|------------------------|
| **HNSW** | [Malkov & Yashunin, 2016](https://arxiv.org/abs/1603.09320) | Hierarchical navigable small-world graph for approximate nearest neighbor search |
| **VAMANA Diversification** | [Subramanya et al. (DiskANN), 2019](https://arxiv.org/abs/1907.05024) | Alpha-based neighbor selection for graph quality and recall. Graduated ef_construction (3-phase schedule) during batch insert |
| **Graph Reordering** | [Chen et al., NeurIPS 2022](https://arxiv.org/abs/2104.03221) | BFS-based node reordering for 15-30% cache miss reduction |

### Quantization & Compression

| Technique | Paper | What it does in VelesDB |
|-----------|-------|------------------------|
| **RaBitQ** | [Gao & Long, 2024](https://arxiv.org/abs/2405.12497) | 1-bit quantization with random rotation + affine correction (32x compression). Dual-precision HNSW: binary traversal + float32 re-ranking |
| **Dual-Precision (VSAG)** | [Xu et al., 2025](https://arxiv.org/abs/2503.17911) | int8 quantized graph traversal with exact float32 re-ranking (4x bandwidth reduction) |

### SIMD & Memory Optimization

| Technique | Paper | What it does in VelesDB |
|-----------|-------|------------------------|
| **Software Pipelining** | [Jiang et al. (Bang for the Buck), 2025](https://arxiv.org/abs/2505.07621) | Speculative prefetch of next candidate's vectors during distance computation. Peek-based pipeline preserving recall |
| **PDX Layout** | [Pirk et al., 2025](https://arxiv.org/abs/2503.04422) | Block-columnar vector storage (64 vectors/block) for SIMD-parallel batch distance computation |
| **SIMD Distance Kernels** | Lemire et al. | Multi-accumulator FMA loops, masked-tail AVX-512, runtime dispatch (AVX-512/AVX2/NEON/scalar) |

### Text Search

| Technique | Reference | What it does in VelesDB |
|-----------|-----------|------------------------|
| **Trigram Fingerprint** | [Broder, 1997](https://doi.org/10.1109/SEQUEN.1997.666900) | 256-bit bloom filter for SIMD-accelerated Jaccard similarity pre-filtering in trigram search |

### Data Structures

| Technique | Reference | What it does in VelesDB |
|-----------|-----------|------------------------|
| **BitVec Visited Set** | ANN best practice | 1-bit-per-node tracking (1.25 KB for 10K nodes vs 80 KB HashSet). Thread-local pooling |
| **FxHashSet** | [rustc-hash](https://crates.io/crates/rustc-hash) | Non-cryptographic hash for integer node IDs in graph traversal visited sets |
| **CSR Snapshot** | Sparse matrix literature | Compressed Sparse Row snapshot of edge store for zero-copy BFS/DFS without lock contention |
| **Parent-Pointer Path** | Graph algorithms | Parent-pointer backtracking replaces per-path cloning in BFS/DFS traversal |
| **Lock-Free CAS Entry** | ANN best practice | AtomicUsize CAS for HNSW entry-point promotion, eliminating mutex contention during parallel inserts |
| **Partial Sort** | `select_nth_unstable_by` | O(n + k log k) top-k extraction instead of O(n log n) full sort |
| **Two-Tier Cache** | [LRU + DashMap] | Lock-free L1 (DashMap, ~50ns) + LRU L2 for distance and query caching |
| **AutoTune ef** | Adaptive search (VelesDB) | Auto-computed ef_search from collection size + dimension. Two-phase adaptive: low-ef first, escalate on hard queries |

---

## Contributing

```bash
git clone https://github.com/cyberlife-coder/VelesDB.git && cd VelesDB
cargo test --workspace --features persistence,gpu,update-check --exclude velesdb-python -- --test-threads=1
cargo fmt --all && cargo clippy --workspace --all-targets --features persistence,gpu,update-check --exclude velesdb-python -- -D warnings -D clippy::pedantic
```

Looking for a place to start? Check out issues labeled [`good first issue`](https://github.com/cyberlife-coder/VelesDB/labels/good%20first%20issue).

---

## Using VelesDB?

If you use VelesDB in your project, add this badge to your README:

```markdown
[![Built with VelesDB](https://img.shields.io/badge/Built_with-VelesDB-blue?style=flat-square)](https://github.com/cyberlife-coder/VelesDB)
```

[![Built with VelesDB](https://img.shields.io/badge/Built_with-VelesDB-blue?style=flat-square)](https://github.com/cyberlife-coder/VelesDB)

We'd love to hear how you're using VelesDB! Share your experience:

- **[GitHub Discussions](https://github.com/cyberlife-coder/VelesDB/discussions)** — tell us about your use case, what works well, and what could be improved
- **Social media** — mention [@VelesDB](https://github.com/cyberlife-coder/VelesDB) and let the community know how VelesDB compares to your previous stack
- **[Star the repo](https://github.com/cyberlife-coder/VelesDB)** — it helps others discover VelesDB

Your feedback shapes the roadmap. Whether it's a RAG pipeline, a knowledge graph, an AI agent, or something we haven't imagined yet — we want to know.

---

## Roadmap

| Version | Status | Highlights |
|---------|--------|------------|
| **v1.11.0** | Released | VelesQL v3.6 — 15 new SQL statements (SHOW, DESCRIBE, EXPLAIN, CREATE/DROP INDEX, ANALYZE, TRUNCATE, ALTER, FLUSH, multi-row INSERT, UPSERT, SELECT EDGES, INSERT NODE), 203 BDD tests, full ecosystem propagation. |
| **v1.10.0** | Released | VelesQL v3.2 — WITH options wiring, component scores, LET clause, Agent Memory VelesQL bridge. All 5 pillars accessible via SQL. |
| **v1.9.3** | Released | VelesQL ecosystem completion — OFFSET fix, MATCH propagation to Python/CLI/tauri/mobile, aggregation routing, DRY refactoring |
| **v1.9.0** | Released | VelesQL ORDER BY arithmetic expressions, MATCH graph documentation, conformance cases P046-P052 |
| **v1.8.0** | Released | 6 perf optimization phases (software pipelining, RaBitQ, PDX layout, SmallVec, AutoTune, Trigram SIMD) + production wiring across 8 crates. **x55 insert, x4 search vs v0.8.10** |
| **v1.7.2** | Released | Partial sort search, batch insert fast-path, upsert lock contention fix, Agent Memory SDK |
| **v1.7.0** | Released | HNSW Upsert, GPU Acceleration, Batch SIMD, Chunked Insertion |
| **v1.6.0** | Released | Product Quantization, Sparse Vectors, Hybrid Search, Streaming Inserts, Query Plan Cache |
| **v1.4.0** | Released | VelesQL v2.2.0, Multi-Score Fusion, Parallel Graph, 2,765 tests |
| **v1.2.0** | Released | Knowledge Graph, Vector-Graph Fusion, ColumnStore, 15 EPICs |

<details>
<summary>Detailed release history</summary>

**v1.11.0** — VelesQL v3.6: 15 new SQL statements — SHOW COLLECTIONS, DESCRIBE, EXPLAIN, CREATE/DROP INDEX, ANALYZE, TRUNCATE (incl. graph collections), ALTER COLLECTION, FLUSH, multi-row INSERT, UPSERT, SELECT EDGES, INSERT NODE. 203 BDD E2E tests. Grammar word-boundary fix for COLLECTION keyword. Tauri query routing aligned with server architecture. Cyclomatic complexity reduced to CC≤8 across 6 hotspots. Full ecosystem propagation across all 8 crates. Python `execute_query()` method. VelesQL spec v3.6.

**v1.10.0** — VelesQL v3.2: WITH options wired to execution (mode, timeout_ms, rerank were parsed but silently ignored — now functional). Independent component scores (`vector_score`, `bm25_score` resolve independently in ORDER BY arithmetic). LET clause for named score bindings (`LET hybrid = 0.7 * vector_score + 0.3 * bm25_score`). Agent Memory VelesQL bridge (`query_semantic/episodic/procedural()` convenience API). USING FUSION configurable for vector+text hybrid queries. 100+ new tests, grammar v3.2.0 with 7 conformance cases.

**v1.9.3** — VelesQL ecosystem completion: OFFSET clause now executed (was parsed-only), MATCH start-node discovery includes graph-only nodes, CLI routes MATCH via active collection, tauri-plugin aggregation results preserved, mobile SDK returns payloads. Python GraphCollection gains 4 VelesQL methods (query, match_query, explain, query_ids). DRY refactoring: shared query helpers. 11 new integration tests.

**v1.9.0** — VelesQL ORDER BY arithmetic expressions (#442): weighted score combinations with operator precedence and parenthesized expressions. New ArithmeticExpr/ArithmeticOp AST types. Conformance cases P046-P052. MATCH documentation: clarified hybrid RRF semantics (#444), documented single-collection graph scope (#445). Graph patterns guide. Closes #442, #443, #444, #445.

**v1.8.0** — 6 performance optimization phases from peer-reviewed research: software pipelining (arXiv:2505.07621), RaBitQ 32x compression (arXiv:2405.12497), PDX block-columnar layout (arXiv:2503.04422), SmallVec batch distances, AutoTune adaptive ef, Trigram SIMD fingerprints. Full production wiring: AutoTune via REST/Python, RaBitQ backend (`HnswBackend` enum), PDX auto-build after reordering. Ecosystem propagation to all 8 crates + TypeScript SDK. Bug fixes: bool→int conversion (#412), silent payload data loss (#413). Concurrency fixes: training buffer race, enum cache regression. Closes #404, #408, #410, #412, #413, #416, #417, #421, #422, #425, #430.

**v1.7.2** — Partial sort in HNSW search_layer (#373), batch insert fast-path (#375), upsert lock contention elimination (3-phase pipeline, write-to-read lock). Agent Memory SDK with complete Python API.

**v1.7.0** — HNSW upsert semantics, complete GPU multi-metric pipelines (wgpu), chunked batch insertion, search_layer batch SIMD + deferred indexing.

**v1.6.0** — Server security (API keys, TLS, graceful shutdown), ~150 Codacy complexity violations resolved, WAL replay, atomic index swap, Windows crash recovery, SDK feature parity, migration tooling (Qdrant, Pinecone), VelesDB Core License 1.0.

**v1.4.0** — VelesQL MATCH queries, EXPLAIN plans, Multi-Score Fusion (RRF, Average, Weighted), parallel graph traversal, VelesQL DISTINCT + Self-JOIN, LangChain/LlamaIndex integrations.

**v1.2.0** — 15 EPICs: Knowledge Graph, VelesQL MATCH, Agent Toolkit, Vector-Graph Fusion, ColumnStore CRUD, Cross-Store JOIN, Python SDK, GPU acceleration.

</details>

---

## License

VelesDB Core License 1.0. Free for development, research, and internal tools. Commercial use requires a license for revenue-generating applications. [Read the full license](LICENSE).

---

<p align="center">
  <strong>VelesDB</strong> &mdash; The Local Knowledge Engine for AI Agents<br/>
  <a href="https://velesdb.com">velesdb.com</a> &bull; <a href="https://github.com/cyberlife-coder/VelesDB">GitHub</a>
</p>
