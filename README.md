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
  <strong>One ~9 MB binary. Three engines. One query language. Zero cloud dependency.</strong><br/>
  <em>Vector + Graph + ColumnStore — unified under <a href="docs/VELESQL_SPEC.md">VelesQL</a></em>
</p>
<p align="center">
  <a href="https://github.com/cyberlife-coder/VelesDB/actions/workflows/ci.yml"><img src="https://github.com/cyberlife-coder/VelesDB/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://app.codacy.com/gh/cyberlife-coder/VelesDB/dashboard?utm_source=gh&utm_medium=referral&utm_content=&utm_campaign=Badge_grade"><img src="https://img.shields.io/codacy/grade/58c73832dd294ba38144856ae69e9cf2?branch=main&label=code%20quality" alt="Codacy code quality"></a>
  <a href="https://crates.io/crates/velesdb-core"><img src="https://img.shields.io/crates/v/velesdb-core.svg?cacheSeconds=3600" alt="Crates.io"></a>
  <a href="https://crates.io/crates/velesdb-core"><img src="https://img.shields.io/crates/d/velesdb-core.svg" alt="Crates.io Downloads"></a>
  <a href="https://pypi.org/project/velesdb/"><img src="https://img.shields.io/pypi/v/velesdb.svg?cacheSeconds=3600" alt="PyPI"></a>
  <a href="https://www.npmjs.com/package/@wiscale/velesdb-sdk"><img src="https://img.shields.io/npm/v/@wiscale/velesdb-sdk.svg?cacheSeconds=3600" alt="npm"></a>
  <a href="https://app.codacy.com/gh/cyberlife-coder/VelesDB/dashboard"><img src="https://img.shields.io/codacy/coverage/58c73832dd294ba38144856ae69e9cf2?branch=main" alt="Codacy coverage"></a>
  <img src="https://img.shields.io/badge/tests-9k%2B_(Rust%2BTS%2BPy)-brightgreen" alt="Tests">
  <a href="https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-VelesDB_Core_1.0-blue" alt="License"></a>
  <a href="https://github.com/cyberlife-coder/VelesDB"><img src="https://img.shields.io/github/stars/cyberlife-coder/VelesDB?style=flat-square" alt="Stars"></a>
  <a href="https://img.shields.io/badge/contributors-welcome-brightgreen"><img src="https://img.shields.io/badge/contributors-welcome-brightgreen" alt="Contributors Welcome"></a>
</p>
<p align="center">
  <a href="https://github.com/cyberlife-coder/VelesDB/releases/latest">Download latest release</a> &bull;
  <a href="#getting-started-in-60-seconds">Quick Start</a> &bull;
  <a href="ARCHITECTURE.md">Architecture</a> &bull;
  <a href="ROADMAP.md">Roadmap</a> &bull;
  <a href="QUALITY_BAR.md">Quality Bar</a> &bull;
  <a href="https://velesdb.com/en/">Documentation</a> &bull;
  <a href="https://deepwiki.com/cyberlife-coder/VelesDB">DeepWiki</a>
</p>

---

> **Every AI agent today stitches together 3 databases for memory — vectors for "what feels similar", a graph for "what is connected", and SQL for "what I know for sure". That's 3 deployments, 3 configs, 3 query languages, and a pile of glue code.**
>
> **VelesDB replaces all of that with a single Rust binary — smaller than a single smartphone photo.**

---

## The Story Behind VelesDB

VelesDB was born in France out of a simple observation: **EU data sovereignty is an architectural problem, not a legal one.**

The US Cloud Act, FISA 702, and PATRIOT Act give US authorities multiple legal paths to reach data held by any US company — regardless of where the servers are. Hosting on AWS `eu-west-1` is a latency decision, not a sovereignty decision. The EU's Data Privacy Framework has been invalidated twice (Schrems I, Schrems II), and a third challenge is pending.

For European developers building AI agents that handle health data, legal documents, or financial records, the typical 2026 stack sends embeddings to Pinecone (US), graphs to Neo4j Aura (US), and metadata to PostgreSQL on AWS (US provider). Every one of these is reachable by a FISA warrant.

VelesDB removes the US provider from the chain entirely. One Rust binary, local-first by design. No API key, no cloud account, no data processor. Your data stays in a directory you control — on your laptop, your server, your jurisdiction.

> [Read the full story: "I built a database in France because the Cloud Act makes EU data sovereignty impossible"](https://dev.to/wiscale-fr/i-built-a-database-in-france-because-the-cloud-act-makes-eu-data-sovereignty-impossible-5325)

---

## Why VelesDB?

| Today (3 systems to maintain) | With VelesDB (1 binary) |
|-------------------------------|------------------------|
| pgvector for embeddings | **Vector Engine** — 450us p50 end-to-end (10K/384D, WAL ON, recall>=96%) |
| Neo4j for knowledge graphs | **Graph Engine** — MATCH clause, BFS/DFS |
| PostgreSQL/DuckDB for metadata | **Typed ColumnStore + secondary indexes** — filtering API 130x faster than JSON scanning at 100K rows*¹ |
| Custom glue code + 3 query languages | **VelesQL** — one language for everything |
| 3 deployments, 3 configs, 3 backups | **~9 MB binary** — works offline, air-gapped |

> *¹ ColumnStore filtering API micro-benchmark, integer equality: 130x at 100K rows, 55x at 10K rows — see [docs/BENCHMARKS.md § 6](docs/BENCHMARKS.md). `SELECT ... WHERE` metadata filtering uses secondary indexes when available, and an adaptive ColumnStore payload mirror for scan-heavy filters (see [2] below).*

---
## What is VelesDB?

VelesDB is a **local-first database for AI agents** that fuses three engines into a single ~9 MB binary [3]:

| Engine | What it does | Performance |
|--------|-------------|-------------|
| **Vector** | Semantic similarity search (HNSW + AVX2/NEON SIMD) | **450us** p50 end-to-end (384D, WAL ON, recall>=96%) [1] |
| **Graph** | Knowledge relationships (BFS/DFS, edge properties) | Native **MATCH** clause |
| **ColumnStore** | Structured metadata filtering (typed columns) | **130x** faster than JSON scanning [2] |

> [1] Reproduce: `python benchmarks/velesdb_benchmark.py --recall` (Python SDK path, 10K/384D, WAL fsync on, i9-14900KF reference machine). See [docs/BENCHMARKS.md](docs/BENCHMARKS.md) and [CHANGELOG v1.13.0](CHANGELOG.md). Re-verified on v3.3.0 (2026-06-24): p50 ≈ 360 µs (356–366 µs across two clean isolated runs), recall@10 0.986–0.989 on Apple Silicon — [report](benchmarks/results/report_3.3.0_apple-silicon_2026-06-24.json) (latency is hardware-specific; the canonical 450 µs is the i9-14900KF figure).
> [2] Reproduce: `cargo bench -p velesdb-core --bench column_filter_benchmark`. See [docs/BENCHMARKS.md § 6](docs/BENCHMARKS.md) — at 100K rows: ColumnStore 29.5 us vs JSON scan 3.84 ms (integer equality filter). Micro-benchmark of the ColumnStore filtering API, which now serves `SELECT ... WHERE` metadata filtering through a per-collection payload mirror (built adaptively for scan-heavy workloads) and backs JOIN execution; secondary indexes are used first when they cover the filter.
> [3] Binary size: `velesdb-server`, stripped release build — 9.3 MB on Apple Silicon for v3.3.0 (the v1.18.0 release artifact was 9.4 MB). Across platforms and binaries (CLI / server / migrate), release artifacts span 6–13 MB. Enforced in CI: `scripts/check_binary_size.py` (workflow `binary-size.yml`) fails the build if a binary exceeds its ceiling.

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

## Agent Memory SDK

Built-in memory for AI agents — semantic, episodic, and procedural. No external services needed.

### The wedge: `why()` — connected memory that survives restarts

Most "agent memory" is vector recall: it finds text that *looks like* your query. VelesDB's high-level `MemoryService` adds the part that's missing — it **connects** memories with typed links, so it can answer *why* something happened by walking the graph to context that shares **no words** with your question. The store is on disk, so it works across sessions. Offline, deterministic, no API key, no model download:

Where Mem0 and Zep are cloud-coupled orchestrators (a service mesh over Qdrant + Postgres, with a cloud LLM in the loop), this is **one local binary** — same tier as Mem0 on LoCoMo QA (~57-58% vs ~55%, neutral basis), ahead of Zep, but fully offline with an auditable `why()` evidence path. Pick it when your data can't leave the box.

![recall() finds the booking but misses the reason; why() reaches it through typed links, across a session restart](examples/agent_memory/why_across_sessions.gif)

```python
from velesdb import MemoryService            # pip install velesdb

mem = MemoryService("./agent_memory")        # a real on-disk store; survives restarts
reason = mem.remember("Robert is recovering from knee surgery")
mem.remember("Booked the aisle seat on Robert's flight", links=[(reason, "because")])

# A *new* process, weeks later, reopens the same store and asks why:
mem.why("why the aisle seat on Robert's flight?")   # walks booking → reason — recall() can't
```

Memories are permanent by default; `forget(id)` deletes one, and `remember(…, ttl_seconds=…)` (or a server-wide `VELESDB_MEMORY_DEFAULT_TTL`) gives a fact a durable, restart-surviving expiry.

The same wedge ships in **Python** (`pip install velesdb`), **Node** (`npm i @wiscale/velesdb-memory-node`), and as a local **[MCP server](crates/velesdb-memory)**.

**Four runnable ways to see it** — each shows what plain vector recall misses and `why()` recovers:

| Demo | What it shows |
|---|---|
| [`why_across_sessions.py`](examples/agent_memory/why_across_sessions.py) | the reason survives a process restart — recall of the top 5 of 16 memories stays blind, `why()` reaches it |
| [`why_magic_constant.py`](examples/agent_memory/why_magic_constant.py) | *why* a magic constant has its value — a business reason that shares no words with the code |
| [`memory_builds_its_own_graph.py`](examples/agent_memory/memory_builds_its_own_graph.py) | paste raw prose → a local model auto-wires the graph (no `relate()`), `why()` walks it to the root cause |
| [`why_magic_constant.mjs`](crates/velesdb-node/examples/why_magic_constant.mjs) | the same engine and wedge in the **Node** binding |

> **Not a weak-embedder trick.** In each retrieval demo, recall stays blind to the reason **even under a real semantic embedder** (`ollama` / `all-minilm`), not just the offline `hash` default — the reason is connected by a decision, not by surface similarity, which is exactly what a vector store cannot follow.

---

For the lower-level building blocks (episodic, procedural, TTL, snapshots):

```python
from velesdb import Database, AgentMemory

db = Database("./agent_data")
memory = AgentMemory(db, dimension=384)

memory.semantic.store(1, "Paris is the capital of France", embedding)
memory.episodic.record(1, "User asked about geography", timestamp, embedding)
memory.procedural.learn(1, "answer_geography", steps, embedding, confidence=0.8)
```

| Feature | API |
|---------|-----|
| TTL / Auto-expiration | `store_with_ttl()`, `auto_expire()` |
| Snapshots / Rollback | `snapshot()`, `load_latest_snapshot()` |
| Reinforcement | `reinforce(success=True)` — 6 strategies (strategy selection via the Rust API; Python uses the `FixedRate` default) |

And because memories live in the same engine as the graph and the ColumnStore, one VelesQL statement recalls by similarity, graph context, and session — in a single query ([tested end-to-end](crates/velesdb-core/tests/bdd/graph_vector_hybrid.rs)):

```sql
SELECT memory.*, similarity() FROM agent_memory AS memory
WHERE vector NEAR $embedding
  AND MATCH (ctx)-[:RELATES_TO]->(fact)
  AND session_id = $current_session
ORDER BY similarity() DESC LIMIT 10
```

> **Full guide:** [docs/guides/AGENT_MEMORY.md](docs/guides/AGENT_MEMORY.md) | [Source code](crates/velesdb-core/src/agent/)

---

## Quick Comparison

| | **VelesDB** | Chroma | Qdrant | pgvector |
|---|---|---|---|---|
| **Architecture** | Unified vector + graph + columnar | Vector only | Vector + payload | Vector extension for PostgreSQL |
| **Metadata filtering** | **Typed ColumnStore [2] + secondary indexes** | JSON scan | JSON payload | SQL (PostgreSQL) |
| **Deployment** | Embedded / Server / WASM / Mobile | Server (Python) | Server (Rust) | Requires PostgreSQL |
| **Binary size** | ~9 MB | ~500 MB (with deps) | ~50 MB | N/A (PG extension) |
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

## Known Limitations

VelesDB is honest about its boundaries. The following are current scope limits of the source-available Community Edition — each is either a deliberate design trade-off or a feature tracked for a separate Enterprise edition. We list them here so you can make an informed technical choice.

| # | Limitation | Scope | Tracked |
|---|------------|-------|---------|
| 1 | **Single writer per collection** — WAL is serialized; concurrent writers contend on the same fsync lock. | Design trade-off (local-first, crash-safe by default). Read throughput is unaffected. | Concurrent WAL writer is planned for the Enterprise edition (separate product, not yet public). See [docs/CONCURRENCY_MODEL.md](docs/CONCURRENCY_MODEL.md). |
| 2 | **No distributed replication** — VelesDB is single-node. No Raft, no sharding, no automatic failover in Core. | Deliberate: the sweet spot is local-first / embedded. | Raft-based replication is tracked internally for the Enterprise edition. Contact us for timeline. |
| 3 | **No advanced RBAC / multi-tenant isolation** — The `DatabaseObserver` hook is shipped (Core) and can be wired to a homegrown RBAC layer, but a production-grade RBAC/audit implementation is not in Core. | Core ships the hook, not the policy engine. | Enterprise feature. |
| 4 | **WASM MATCH limited to 2 hops** — The browser build of `velesdb-wasm` supports 1- and 2-hop graph `MATCH` patterns today. 3+ hop `MATCH` works fully in native builds (server / Python / mobile / CLI) via `velesdb-core`. | Scope of Sprint 4 item S4-13. | Tracked, not a correctness issue — native path already supports full traversal. |
| 5 | **SIFT1M benchmark fingerprints — pinning workflow ships, sidecar not yet committed** — The loader reads its pinned SHA-256 hashes from `benches/datasets/sift1m_fingerprints.json` when present (strict mode, mismatch fails the bench). Until a maintainer runs `cargo bench -p velesdb-core --features bench-sift1m --bench capture_sift1m_fingerprints` on the reference machine and commits the generated sidecar, the loader falls back to TOFU mode (prints the observed SHA-256 and proceeds). | Not a correctness issue — `check_shape` still validates row count and dimension. The one-command bootstrap closes the integrity gap in a single run. | One-command bootstrap shipped; sidecar commit pending first reference-machine run. |
| 6 | **No head-to-head Docker Compose benchmark vs Qdrant / Chroma / FAISS yet** — The SIFT1M benchmark (new in v1.13.0) is the standardized cross-implementation comparable number and matches the dataset used by every major ANN paper. A one-shot Docker Compose harness that runs all four systems on the same machine is deferred until the benchmark infrastructure stabilizes. | Transparency: side-by-side numbers require infrastructure we have not frozen yet. | Tracked; SIFT1M already gives comparable recall@10 numbers against the literature. |

None of the above is a correctness gap — the Community Edition is production-ready for single-node, local-first deployments. The items above are feature-scope boundaries, not bugs.

For **internal technical limitations** (query-planner approximations, plan cache semantics around `ANALYZE`, CBO integration status), see [`docs/reference/KNOWN_LIMITATIONS.md`](docs/reference/KNOWN_LIMITATIONS.md) — each entry is tracked by a GitHub issue or documented as an explicit approximation with regression tests.

---

## Getting Started in 60 Seconds

**The fastest path is Python — under 5 seconds median, measured.** ([timing methodology](docs/quickstart/timing-results.md))

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

That's it — no server, no JSON, no embedding model. Read the [25-line script](examples/python/hello_velesdb.py) to see what happened. From here, the [Agent Memory guide](docs/guides/AGENT_MEMORY.md) and the [VelesQL spec](docs/VELESQL_SPEC.md) are the natural next stops.

<details>
<summary><strong>Other install paths — Rust, Docker, WASM, REST server</strong></summary>

**Cargo (Rust + REST server):**
```bash
cargo install velesdb-server velesdb-cli
```

**Docker (REST server):**
```bash
# Build the image locally
git clone https://github.com/cyberlife-coder/VelesDB.git && cd VelesDB
docker build -t velesdb .

# Run with persistent data (named volume)
docker run -d -p 8080:8080 -v velesdb_data:/data --name velesdb velesdb

# Verify it's running
curl http://localhost:8080/health
```

Data is stored in `/data` inside the container; the named volume `velesdb_data` persists across restarts.

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

**First search against the REST server (once `velesdb-server` is running on :8080):**

```bash
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
# {"results":[{"id":"1","score":0.994,"payload":{"title":"AI Intro","category":"tech"}}, ...]}
# Results are wrapped in {"results":[...]} and point ids serialize as strings.
# (The unified POST /query endpoint instead returns projected rows with integer ids.)
```

</details>

> Full installation guide: [docs/guides/INSTALLATION.md](docs/guides/INSTALLATION.md)

---

## Vector Engine

Native HNSW index with SIMD-accelerated distance kernels. Sub-millisecond search on modern x86_64 hardware.

### End-to-end search latency (canonical)

| Metric | Value |
|--------|-------|
| Search p50 (10K, 384D, WAL ON) | **450 us** |
| SIMD Dot Product (768D, AVX2) | **21.7 ns** |
| Recall@10 (Balanced) | **98.8%** |
| Quantization | PQ (8–32x, config-dependent), RaBitQ (32x), SQ8 (4x)*³, Binary (32x)*³ |

> *³ Query-path compression comes from **PQ** and **RaBitQ** — both are wired end-to-end into the collection search path, restarts included. The collection-level SQ8/Binary modes maintain caches that no search path reads yet (search stays full-precision f32 — SQ8 as a collection mode therefore *adds* memory); their quantization primitives remain available programmatically. See [docs/guides/QUANTIZATION.md](docs/guides/QUANTIZATION.md).

> **Provenance of the canonical figures above:** Intel Core **i9-14900KF** (x86_64, AVX2), `velesdb_benchmark.py`. "End-to-end / p50" = the full production path (VelesQL → HNSW → **WAL ON** → payload hydration), median over the query set. "Index-only" figures (in the details below) exclude WAL and payload and run on a hot cache — they are not comparable to the end-to-end number. Per-machine figures vary; fresh Apple-Silicon measurements are given below.

5 search quality modes (Fast → Perfect), adaptive two-phase ef, AutoTune.

<details>
<summary>Detailed benchmarks and search modes</summary>

### HNSW index-only micro-benchmark (lab-grade)

> The number below is the **index-only** micro-benchmark (no WAL, no metadata fetch, hot cache). For the production-path number, see "End-to-end search latency (canonical)" above — **450µs p50** at 10K/384D, recall ≥ 96%.

| Component micro-benchmark | Result | How to reproduce |
|-----------|--------|------------------|
| HNSW Search index-only (5K/768D, k=10) | **55 us** | `cargo bench -p velesdb-core --bench hnsw_benchmark -- hnsw_search_latency` |
| SIMD Dot Product kernel (768D, AVX2) | **21.7 ns** | `cargo bench -p velesdb-core --bench simd_benchmark` |
| Recall@10 (Accurate mode) | **100%** | `cargo bench -p velesdb-core --bench recall_benchmark` |
| BM25 Sparse Search index-only (10K docs, top-10) | **57.6 us** (16x from 956 us in v1.12) | `cargo bench -p velesdb-core --bench sparse_benchmark -- top10_10k_corpus` |

#### Cross-checked on Apple M5 Pro (ARM64 / NEON, 18-core) — measured 2026-05-31, v1.16.0

Fresh figures on Apple Silicon (single-thread, run in isolation). They confirm the engine profile and make the *scope* of each number explicit; they are not a substitute for the x86_64/AVX2 reference figures above.

All `cargo bench` commands below are run as `cargo bench -p velesdb-core --bench <NAME>`.

| What it actually measures | Result | Bench |
|---|---|---|
| HNSW search, **index-only** (10K/768D, k=10; no WAL/payload, hot cache) | 55 µs | `hnsw_benchmark -- hnsw_search_latency` |
| HNSW search **scaling** (top-10, index-only) | 116 µs @100K · 128 µs @500K · 129 µs @1M | `scalability_benchmark` |
| **VelesQL engine** (parse→plan→execute→project, 10K) | 41 µs | `velesql_execution_benchmark` |
| **End-to-end via PyO3/NumPy** (10K/384D, p50; the Python production path) | 55 µs (p99 99 µs) | `python benchmarks/velesdb_benchmark.py` |
| SIMD distance, **NEON** (768D): dot / euclidean / cosine | 31 / 35 / 47 ns | `simd_benchmark` |
| BM25 full-text search (10K, single / multi-term) | 23.5 / 71 µs | `bm25_benchmark` |
| Sparse search (top-10, 10K corpus) | 29.8 µs | `sparse_benchmark -- top10_10k_corpus` |
| **Recall@10** (n=10K/128D, exact brute-force GT; ef sweep) | ef=96 → 97.4% · ef=160 → 99.8% · ef=512 → 100% | `recall_benchmark` |

> The recall figures above are `recall_benchmark`'s internal ef sweep (96/160/512) — **distinct** from the product "Modes" table below (Fast/Balanced/Accurate use ef 64/128/512). On this machine the PyO3/NumPy binding overhead is negligible: end-to-end ≈ index-only ≈ 55 µs. The **450 µs** canonical figure is the i9-14900KF reference under WAL-on production conditions; per-machine results vary. Recall uses a real exact-kNN ground truth, not approximate self-comparison.

| Mode | ef_search | Recall@10 | Use case |
|------|-----------|-----------|----------|
| Fast | 64 | 92.2% | Real-time suggestions, typeahead |
| Balanced (default) | 128 | 98.8% | Production search, RAG pipelines |
| Accurate | 512 | 100% | Evaluation, ground truth comparison |

*Measurements sourced from `benchmarks/results/pr363_365_comparison.md` (i9-14900KF, 64 GB DDR5, Windows 11, `--release`, `target-cpu=native`). Windows micro-benchmarks carry 5-10% noise — expect a range, not a single point.*

</details>

### Distance Metrics

5 metrics with SIMD acceleration (AVX-512, AVX2, NEON; WASM currently uses the scalar fallback — SIMD128 kernels are planned):

| Metric | What it measures | Use case | SIMD perf (768D)*² |
|--------|-----------------|----------|------------------|
| **Cosine** | Angle between vectors (direction similarity) | Text embeddings (BERT, OpenAI, Cohere), normalized vectors | 33 ns |
| **Euclidean** | Straight-line distance (L2 norm) | Image features, spatial data, when magnitude matters | 20 ns |
| **Dot Product** | Inner product (projection) | Pre-normalized vectors, Maximum Inner Product Search (MIPS) | 22 ns |
| **Hamming** | Bit differences in binary vectors | Binary embeddings, locality-sensitive hashing (LSH), fingerprints | 36 ns |
| **Jaccard** | Set overlap (intersection / union) | Sparse vectors, tag similarity, set membership | 35 ns |

> *² 768D vectors, AVX2 hot cache (matches the table column header), see promise-contract.json for the policed claim*

```sql
-- Choose metric at collection creation
CREATE COLLECTION docs (dimension = 768, metric = 'cosine');
CREATE COLLECTION images (dimension = 512, metric = 'euclidean');
CREATE COLLECTION fingerprints (dimension = 256, metric = 'hamming');
```

```sql
SELECT * FROM docs WHERE vector NEAR $v AND category = 'tech' LIMIT 5
```

- **SIFT1M standardized ANN benchmark** — measured on the de-facto-standard INRIA TEXMEX dataset (1M × 128D vectors, L2 metric). See [docs/BENCHMARKS.md § 11](docs/BENCHMARKS.md#11-sift1m--standard-ann-benchmark) for methodology, dataset provenance, and how to reproduce.

> **Full benchmarks and methodology:** [docs/BENCHMARKS.md](docs/BENCHMARKS.md) | [velesdb-benchmarks repo](https://github.com/cyberlife-coder/velesdb-benchmarks) | **Quantization guide:** [docs/guides/QUANTIZATION.md](docs/guides/QUANTIZATION.md)

---

## Graph Engine

Property graph with BFS/DFS traversal, edge labels, and Cypher-inspired MATCH queries — integrated with vector search.

```sql
-- Vector + Graph fusion in ONE statement
MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)
WHERE similarity(doc.embedding, $question) > 0.8
RETURN author.name, doc.title
ORDER BY similarity() DESC LIMIT 5
```

Cross-collection MATCH with `@collection` annotation — traversal runs on the
primary collection's edge store; `@collection` enriches the matched node's
payload from another collection (it is not a distributed cross-graph traversal):

```sql
MATCH (p:Product@products)-[:STORED_IN]->(inv:Inventory@inventory)
RETURN p.name, inv.price, inv.stock
LIMIT 20
```

> **Graph patterns guide:** [docs/guides/GRAPH_PATTERNS.md](docs/guides/GRAPH_PATTERNS.md)

---

## ColumnStore Engine

Typed columnar storage — the same approach DuckDB and ClickHouse use. Its
filtering API is **130x faster** than JSON scanning at 100K rows
(micro-benchmark: `cargo bench -p velesdb-core --bench column_filter_benchmark`).

```
JSON scan: 3.84 ms @ 100K    →    ColumnStore: 29.5 us @ 100K (130x faster)
```

The ColumnStore engine backs `JOIN` execution and serves `SELECT ... WHERE`
metadata filtering through a per-collection payload mirror: top-level scalar
payload fields are mirrored into typed columns, and filters compile to
RoaringBitmap scans. The mirror is built adaptively — only after sequential
scans have cost more than one full pass — so point lookups keep their fast
path; secondary indexes are still consulted first when they cover the filter:

```sql
SELECT * FROM products
WHERE vector NEAR $query AND in_stock = true AND price < 50.0
LIMIT 10
```

---

## Use Cases

### AI Agent Memory

Your agent needs to remember conversations, learn from mistakes, and recall relevant knowledge. VelesDB provides all three memory types in a single embedded database — no Redis, no Pinecone, no Neo4j.

```python
memory = AgentMemory(db, dimension=384)
memory.semantic.store(1, "User prefers dark mode", embedding)
memory.episodic.record(2, "User asked about billing", timestamp, embedding)
memory.procedural.learn(3, "handle_refund", steps, embedding, confidence=0.9)
```

### RAG with Metadata Filtering

Vector search alone returns noise. VelesDB combines vector search with metadata filters (secondary indexes + planner-chosen pre/post-filtering) to eliminate irrelevant results.

```sql
SELECT * FROM docs
WHERE vector NEAR $query AND department = 'engineering' AND updated_at > NOW() - INTERVAL '30 days'
LIMIT 10
```

### E-commerce: Vector + Graph + Filters in One Query

Find products similar to a query, filter by price/stock, and traverse co-purchase relationships — all in a single VelesQL statement.

```sql
MATCH (product)-[:BOUGHT_TOGETHER]->(related)
WHERE similarity(product.embedding, $query) > 0.7
  AND related.price < 200 AND related.in_stock = true
RETURN related.name, related.price
ORDER BY similarity() DESC LIMIT 20
```

### Desktop & Mobile AI

Ship AI features without a server. VelesDB embeds directly into Tauri, iOS, and Android apps.

| Platform | Integration | Binary size |
|----------|-------------|-------------|
| Desktop (Tauri) | `tauri-plugin-velesdb` | ~9 MB |
| iOS (Swift) | UniFFI bindings | ~4 MB |
| Android (Kotlin) | UniFFI bindings | ~4 MB |
| Browser | WASM module | ~430 KB gzipped |

---

## Roadmap

| Milestone | Status |
|-----------|--------|
| v1.0 — Core engine (vector + graph + VelesQL) | ✅ Shipped |
| v1.5 — Python SDK, WASM, Mobile bindings | ✅ Shipped |
| v1.10 — Agent Memory SDK, hybrid search, quantization | ✅ Shipped |
| v1.11 — Cross-collection MATCH, bitmap pre-filter, CSR graph | ✅ Shipped |
| v1.12 — Cross-collection MATCH (graph/BM25/HNSW hybrids), Sprint 4 Phase B (TS SDK stability) | ✅ Shipped |
| v1.13 — Pre-seed remediation: BM25 O(1) cold-start, sparse search 16× speedup, HNSW prefetch, EXPLAIN/CBO routing, VelesQL window functions, SIFT1M standardized harness | ✅ Shipped |
| v1.14 — DX correctness: MSRV 1.89 alignment, Dockerfile auto-sync; **Haystack 2.x DocumentStore** completes the LangChain + LlamaIndex + Haystack Python RAG trio | ✅ Shipped |
| v1.15 — ACT-R Phase 1 procedural learning, CBO calibration in `EXPLAIN ANALYZE`, Python auto-dimension + `SearchOptions` builder | ✅ Shipped |
| v1.16 — `audit-2026q2` security-hardening wave (9 PRs), first-party embedding adapters (Python + TypeScript), multi-arch GHCR image | ✅ Shipped |
| v1.17 — VelesQL error hints with did-you-mean suggestions, payload-WAL torn-tail crash recovery, OpenAPI id-type accuracy | ✅ Shipped |
| v1.18 — Engine artifacts realigned to VelesDB Core License 1.0, agent-memory parity (Python/Tauri bindings, TS procedural recall) | ✅ Shipped |
| v2.0.0 — Agent-memory graph dimension (`relate()` API + the NEAR + MATCH flagship query verbatim), GraphFirst anchored retrieval, PQ/RaBitQ quantization wired end-to-end across restarts, durable TTL on every read path, `GET /metrics` by default | ✅ Shipped |

> VelesDB Core is source-available (readable, modifiable, redistributable under the VelesDB Core License 1.0 — not an OSI-approved license; see [docs/LICENSING.md](docs/LICENSING.md)). Enterprise features (distributed replication, managed cloud, RBAC) are available separately via [VelesDB Premium](https://velesdb.com).

> We ship weekly. [Full changelog](CHANGELOG.md) | [Contributing guide](CONTRIBUTING.md)

---

## Full Ecosystem

| Domain | Component | Install |
|--------|-----------|---------|
| **Core** | [velesdb-core](crates/velesdb-core) — Vector + Graph + ColumnStore + VelesQL | `cargo add velesdb-core` |
| **Server** | [velesdb-server](crates/velesdb-server) — REST API (48 endpoints, OpenAPI) | `cargo install velesdb-server` |
| **CLI** | [velesdb-cli](crates/velesdb-cli) — Interactive VelesQL REPL | `cargo install velesdb-cli` |
| **Python** | [velesdb-python](crates/velesdb-python) — PyO3 bindings + NumPy | `pip install velesdb` |
| **TypeScript** | [typescript-sdk](sdks/typescript) — Node.js & Browser SDK | `npm install @wiscale/velesdb-sdk` |
| **WASM** | [velesdb-wasm](crates/velesdb-wasm) — Browser-side vector search | `npm install @wiscale/velesdb-wasm` |
| **Agent memory (MCP)** | [velesdb-memory](crates/velesdb-memory) — local-first MCP memory server (`why()` wedge) | `cargo install velesdb-memory` |
| **Agent memory (Node)** | [velesdb-node](crates/velesdb-node) — in-process napi binding of the memory wedge | `npm install @wiscale/velesdb-memory-node` |
| **Mobile** | [velesdb-mobile](crates/velesdb-mobile) — iOS (Swift) & Android (Kotlin) | [Build instructions](docs/guides/INSTALLATION.md#-mobile-iosandroid) |
| **Desktop** | [tauri-plugin](crates/tauri-plugin-velesdb) — Tauri v2 AI-powered apps | `cargo add tauri-plugin-velesdb` |
| **LangChain** | [langchain-velesdb](integrations/langchain) — Official VectorStore | [From source](integrations/langchain/README.md) |
| **LlamaIndex** | [llama-index-vector-stores-velesdb](integrations/llamaindex) — Document indexing | [From source](integrations/llamaindex/README.md) |
| **Haystack** | [haystack-velesdb](integrations/haystack) — Haystack 2.x DocumentStore | [From source](integrations/haystack/README.md) |
| **Migration** | [velesdb-migrate](crates/velesdb-migrate) — From Qdrant, Pinecone, Supabase | `cargo install velesdb-migrate` |

> **Python RAG framework parity**: VelesDB ships a first-party connector for the three major Python RAG frameworks — **LangChain** (`VectorStore`), **LlamaIndex** (`VectorStoreIndex`), and **Haystack 2.x** (`DocumentStore`) — so you can swap VelesDB into any existing RAG pipeline with a single dependency change.

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
<summary>API Reference (48 REST endpoints)</summary>

| Category | Key Endpoints |
|----------|--------------|
| **Collections** | `POST /collections`, `GET /collections`, `GET/DELETE /collections/{name}` |
| **Points** | `/collections/{name}/points`, `/collections/{name}/points/scroll`, `/collections/{name}/stream/insert`, `/collections/{name}/points/{id}/relations`, `/collections/{name}/points/{id}/ttl`, `/collections/{name}/relations` |
| **Search** | `/collections/{name}/search`, `/collections/{name}/search/batch`, `/collections/{name}/search/hybrid`, `/collections/{name}/search/text`, `/collections/{name}/search/multi`, `/collections/{name}/search/ids`, `/collections/{name}/match` |
| **Graph** | `/collections/{name}/graph/edges`, `/collections/{name}/graph/edges/{id}`, `/collections/{name}/graph/edges/count`, `/collections/{name}/graph/traverse`, `/collections/{name}/graph/traverse/stream`, `/collections/{name}/graph/traverse/parallel`, `/collections/{name}/graph/nodes`, `/collections/{name}/graph/nodes/{id}/degree`, `/collections/{name}/graph/nodes/{id}/edges`, `/collections/{name}/graph/nodes/{id}/payload`, `/collections/{name}/graph/search` |
| **Indexes** | `GET/POST /collections/{name}/indexes`, `DELETE /collections/{name}/indexes/{label}/{property}`, `/collections/{name}/index/rebuild` |
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

```bash
cd examples/ecommerce_recommendation && cargo run --release
```

| Demo | Description | Tech |
|------|-------------|------|
| [ecommerce_recommendation](examples/ecommerce_recommendation/) | Vector + Graph + ColumnStore (5K products) | Rust |
| [velesdb-memory](crates/velesdb-memory/) | MCP memory server — the graph answers *why* a decision was made | Rust |
| [rag-pdf-demo](demos/rag-pdf-demo/) | PDF document Q&A with RAG | Python, FastAPI |
| [tauri-rag-app](demos/tauri-rag-app/) | Desktop RAG application | Tauri v2, React |
| [wasm-browser-demo](examples/wasm-browser-demo/) | In-browser vector search | WASM, vanilla JS |
| [mini_recommender](examples/mini_recommender/) | Product recommendations | Rust |

---

<details>
<summary>Research Foundations</summary>

VelesDB's performance is built on peer-reviewed research — five of the six techniques below are implemented and production-active in the engine; Dual-Precision (VSAG) ships as a public API with a benchmark harness, with engine integration tracked.

| Technique | Paper | Status |
|-----------|-------|--------|
| HNSW | [Malkov & Yashunin, 2016](https://arxiv.org/abs/1603.09320) | Production-active |
| VAMANA / DiskANN | [Subramanya et al., 2019](https://arxiv.org/abs/1907.05024) | Production-active (alpha pruning) |
| RaBitQ | [Gao & Long, 2024](https://arxiv.org/abs/2405.12497) | Production-active (query path, restarts included) |
| Dual-Precision (VSAG) | [Xu et al., 2025](https://arxiv.org/abs/2503.17911) | Public API + benchmark; engine integration tracked |
| Software Pipelining | [Jiang et al., 2025](https://arxiv.org/abs/2505.07621) | Production-active (search pipeline) |
| PDX Layout | [Pirk et al., 2025](https://arxiv.org/abs/2503.04422) | Production-active (columnar layout via `ANALYZE` reorder) |

</details>

## Contributing

```bash
git clone https://github.com/cyberlife-coder/VelesDB.git && cd VelesDB
cargo test --workspace --features persistence,gpu,update-check --exclude velesdb-python -- --test-threads=1
```

Looking for a place to start? Check out issues labeled [`good first issue`](https://github.com/cyberlife-coder/VelesDB/labels/good%20first%20issue).

---

## Powered by VelesDB

| Project | Use case |
|---------|----------|
| [WPLink](https://wplink.ai) | AI-powered semantic analysis to find and apply internal linking opportunities for WordPress sites |
| *Your project here* | [Get listed →](mailto:contact@wiscale.fr?subject=VelesDB%20Showcase) |

[![Built with VelesDB](https://img.shields.io/badge/Built_with-VelesDB-blue?style=flat-square)](https://github.com/cyberlife-coder/VelesDB)

Using VelesDB in production? Open a [GitHub Discussion](https://github.com/cyberlife-coder/VelesDB/discussions) or email [contact@wiscale.fr](mailto:contact@wiscale.fr) to get featured. Your feedback shapes the roadmap.

---

## License

VelesDB Core License 1.0 (based on ELv2). Free for production use, including commercial applications. Two restrictions: no offering VelesDB as a hosted/managed database service, and no building a competing database product. [Read the full license](LICENSE).

---

<p align="center">
  <strong>VelesDB</strong> &mdash; The Local Knowledge Engine for AI Agents<br/>
  <a href="https://velesdb.com">velesdb.com</a> &bull; <a href="https://github.com/cyberlife-coder/VelesDB">GitHub</a>
</p>
