<p align="center">
  <img src="velesdb_icon_pack/favicon/android-chrome-512x512.png" alt="VelesDB Logo" width="180"/>
</p>
<h1 align="center">
  <img src="velesdb_icon_pack/favicon/favicon-32x32.png" alt="" width="32" height="32" style="vertical-align: middle;"/> VelesDB
</h1>
<h3 align="center">
  Three engines. One language. A memory your agents can explain.
</h3>
<p align="center">
  <strong>One ~9 MB binary fuses vector + graph + columnar under <a href="docs/VELESQL_SPEC.md">VelesQL</a> —
  persistent agent memory with an evidence trail (<code>why()</code>) and a deterministic
  context optimizer that cuts your <em>real, billed</em> token spend.</strong><br/>
  Zero cloud, zero LLM, zero API key in the memory path. Every number on this page links to a
  <a href="crates/velesdb-memory/BENCHMARK.md">committed, reproducible harness</a>. Measured, not vibes.
</p>
<p align="center">
  <sub><em>The name nods to <strong>Veles</strong>, a deity of old Slavic myth — a keeper of hidden knowledge and boundaries.</em></sub>
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
</p>
<p align="center">
  <a href="#get-started-in-60-seconds">Quick Start</a> &bull;
  <a href="#the-numbers--every-one-from-a-committed-harness">Proof</a> &bull;
  <a href="#velesdb-premium--the-enterprise-control-plane">Premium</a> &bull;
  <a href="ARCHITECTURE.md">Architecture</a> &bull;
  <a href="ROADMAP.md">Roadmap</a> &bull;
  <a href="https://velesdb.com/en/">velesdb.com</a> &bull;
  <a href="https://deepwiki.com/cyberlife-coder/VelesDB">DeepWiki</a>
</p>

---

## Why VelesDB exists

Every AI agent today has the same two problems:

1. **It forgets.** Session ends, context gone. Teams bolt on three databases to fix it — vectors for *"what feels similar"*, a graph for *"what is connected"*, SQL for *"what I know for sure"* — three deployments, three query languages, and a pile of glue code.
2. **It overpays.** Most of the budget goes to re-reading redundant context, and "memory" products fix it by putting *another paid model call* inside every memory write — generative, non-reproducible, unexplainable by construction.

**VelesDB answers both with two layers that ship as one project:**

| | What it is | How you use it |
|---|---|---|
| **1 · The tri-engine agentic database** | Vector + graph + columnar fused in one ~9 MB Rust binary, queried in **[VelesQL](docs/VELESQL_SPEC.md)** — one language across all three. | Embedded (Rust/Python), REST server, WASM in the browser, mobile. |
| **2 · The agent memory model** | Built *on* that database: `remember` / `relate` / `why()` with typed links and an evidence trail, resumable working contexts, and the deterministic **context token optimizer** — measured against **real provider bills**. | A local **MCP server** for any agentic CLI (Claude Code, Codex, …) + **Python and Node bindings**. |

The database stands on its own. The memory model is why agents pick it.

| If you are… | What you get |
|---|---|
| **A developer living in an agentic CLI** (Claude Code, Codex, …) | Your agent remembers across sessions, answers `why()` with an evidence trail, and burns measurably fewer tokens — [installed in 3 commands](#give-your-agent-a-memory-3-commands). |
| **A CTO** | One auditable ~9 MB binary instead of 3 databases + glue. Deterministic writes, an audit trail per decision — the kind of explainability the [EU AI Act (enforceable Aug 2026)](https://artificialintelligenceact.eu/implementation-timeline/) asks of AI systems — running in your jurisdiction, air-gapped if needed. |
| **A company running agents at scale** | The same engine with an enterprise control plane on top: RBAC, audit trail, multi-tenancy — see [VelesDB Premium](#velesdb-premium--the-enterprise-control-plane). |

## The differentiator no one else combines

The leading agent-memory products ([Mem0, Zep, Letta — detailed comparison, as of mid-2026](docs/WHY_VELESDB.md)) put an AI model **in the memory write path**: every save runs model calls — by default paid, cloud, keyed. VelesDB is built on the opposite bet:

| Property | What it means concretely |
|---|---|
| 🎯 **Deterministic** | The same input always compiles to the **same bytes** — asserted twice per turn in every committed benchmark. No model in the loop: no drift, no surprise rewrites, and a [byte-stable cache prefix](crates/velesdb-memory/examples/context_savings/real_measures/cache_prefix.mjs) provider prompt-caching can actually hit. |
| 🔍 **Explainable** | `why()` returns the **evidence trail** behind every recall; `explain_compilation` gives every kept/dropped fragment a stable rule id, a reason, and a risk level. A built-in audit trail, not a slide-deck promise. |
| ♻️ **Reversible** | Nothing is silently lost. Over-budget content becomes a recoverable `ctx://source/` handle — `retrieve_context_source` brings the original bytes back on demand, always. |
| 🏠 **Local-first** | One ~9 MB binary on your machine: vector + graph + columnar in-process, **zero AI calls and zero API keys** to store a memory. Works offline, air-gapped, in your jurisdiction. |

---

## Get started in 60 seconds

**Fastest path — Python, under 5 seconds median, [measured](docs/quickstart/timing-results.md):**

```bash
pip install velesdb
curl -O https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/examples/python/hello_velesdb.py
python hello_velesdb.py
```

Expected output (byte-for-byte — [read the 25-line script](examples/python/hello_velesdb.py)):

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

### Give your agent a memory (3 commands)

```bash
# 1. The MCP memory server (or grab a prebuilt .mcpb bundle from the latest release)
cargo install velesdb-memory

# 2. Point your agentic CLI at it (Claude Code shown; any MCP client works)
claude mcp add velesdb-memory -- ~/.cargo/bin/velesdb-memory

# 3. Teach the agent the workflow — both skills, no repo clone needed
curl -L https://github.com/cyberlife-coder/VelesDB/releases/latest/download/velesdb-skills.tar.gz \
  | tar -xz -C ~/.claude/skills/
```

Want the memory used *continuously*, not just available? [`integrations/agent-hooks/`](integrations/agent-hooks/README.md) wires `SessionStart`/`Stop`/`PreCompact` hooks that resume and save the working context automatically — one global install covers every project. No Rust toolchain? `npm i @wiscale/velesdb-memory-node`.

Want Claude Code, Claude Desktop, and Windsurf sharing the *same* memory instead of one client at a time? [`scripts/install-memory-daemon.sh`](crates/velesdb-memory/README.md#http-transport-multi-client) runs `velesdb-memory` as a single local HTTP daemon and wires every client to it.

<details>
<summary><strong>Other install paths — Rust, Docker, WASM, REST server</strong></summary>

**Cargo (Rust + REST server):**
```bash
cargo install velesdb-server velesdb-cli
```

**Docker (REST server, multi-arch linux/amd64 + linux/arm64):**
```bash
docker run -d -p 8080:8080 -v velesdb_data:/data --name velesdb \
  ghcr.io/cyberlife-coder/velesdb:latest
curl http://localhost:8080/health
```

**Browser / edge:** the WASM build is ~550 KB gzipped — the engine runs entirely client-side ([TypeScript SDK](sdks/typescript)).

**REST:** the server exposes 54 REST endpoints ([OpenAPI spec](docs/openapi.yaml)).

</details>

---

## The numbers — every one from a committed harness

No figure below is an estimate from a slide. Each links to the log or script, in this repo, that produced it — rerun them yourself.

| Claim | Measured | Source |
|---|---|---|
| Real **billed dollars** saved on an A/B agent session (raw vs compiled, same session, real Claude billing) | **10.9 %** at cropped-screenshot weight → **21.9 %** at real Retina screenshot weight; **14.7 %** on a 36-turn day-scale arc | [real-session-benchmark](examples/real-session-benchmark#billed-campaign-results-2026-07-19-cli-runner-claude-sonnet-5) · [raw logs](examples/real-session-benchmark/results/2026-07-19-vibe-cli/) · [day-scale logs](examples/real-session-benchmark/results/2026-07-19-day-scale/) |
| Direct Messages-API **input tokens** saved (no CLI cache routing diluting the signal) | **15.1 %** | [api-runner log](examples/real-session-benchmark/results/2026-07-19-day-scale/billed-vibe-api.log) |
| **Answer quality at parity** while saving — deterministic fact-checklist grader, no LLM judge | **22.8–23.0 / 23 facts** in both arms (compiled arm lost nothing) | [real-session-benchmark](examples/real-session-benchmark#billed-campaign-results-2026-07-19-cli-runner-claude-sonnet-5) |
| Real (cl100k) **input-token savings** on a committed 12-turn agent-session corpus | **82.5 %** (per-turn 80–87 % as the session grows) | [context_savings](crates/velesdb-memory/examples/context_savings) |
| **Compile latency**, stateless | **~0.5 ms** mean (12-turn benchmark; ~27 ms with source persistence on) | [context_savings](crates/velesdb-memory/examples/context_savings) |
| **Memory retrieval quality** on public test sets, no AI grader in the loop | **+7.2 pts** multi-hop (HotpotQA), **+9.7 pts** time-scoped recall (TimeQA), **+29 pts** on a controlled task needing both engines at once | [BENCHMARK.md](crates/velesdb-memory/BENCHMARK.md) |
| **Vector search**, end-to-end production path | **450 µs** p50 (10K/384D, WAL ON, recall ≥ 96 %) | [docs/BENCHMARKS.md](docs/BENCHMARKS.md) |

> The 2.5 % billed saving of the no-screenshots variant is published as prominently as the 21.9 % — the delta *is* the measured value of the media mechanisms. [Honest reading, limitations, and full protocol](examples/real-session-benchmark#honest-limitations). Every claim above is CI-guarded by a [promise contract](docs/reference/promise-contract.json) that pins the README to its committed sources.

---

## 1 · The tri-engine agentic database — VelesQL

| Engine | What it does | Performance |
|--------|-------------|-------------|
| **Vector** | Semantic similarity search (HNSW + AVX2/NEON SIMD) | **450us** p50 end-to-end (384D, WAL ON, recall>=96%) [1] |
| **Graph** | Knowledge relationships (BFS/DFS, edge properties) | Native **MATCH** clause |
| **ColumnStore** | Structured metadata filtering (typed columns) | **130x** faster than JSON scanning [2] |

One SQL-like statement crosses all three — similarity, relations, and typed filters, no glue code:

```sql
MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)
WHERE similarity(doc.embedding, $question) > 0.8
  AND author.department = 'Engineering'
RETURN author.name, doc.title
ORDER BY similarity() DESC LIMIT 5
```

| Today (3 systems to maintain) | With VelesDB (1 binary) |
|-------------------------------|------------------------|
| pgvector for embeddings | **Vector Engine** — 450us p50 end-to-end |
| Neo4j for knowledge graphs | **Graph Engine** — MATCH clause, BFS/DFS |
| PostgreSQL/DuckDB for metadata | **Typed ColumnStore + secondary indexes** — 130x faster than JSON scanning at 100K rows [2] |
| Custom glue code + 3 query languages | **VelesQL** — one language for everything |
| 3 deployments, 3 configs, 3 backups | **~9 MB binary** — works offline, air-gapped |

> [1] Reproduce: `python benchmarks/velesdb_benchmark.py --recall` (10K/384D, WAL fsync on, i9-14900KF reference machine; Apple-Silicon cross-check in [docs/BENCHMARKS.md](docs/BENCHMARKS.md)).
> [2] Reproduce: `cargo bench -p velesdb-core --bench column_filter_benchmark` — at 100K rows on the i9-14900KF reference machine: ColumnStore 29.5 us vs JSON scan 3.84 ms. The ratio is hardware-dependent: on Apple Silicon (M5 Pro, 2026-07-20) the JSON scan itself runs ~2.8× faster, so the same bench measures ~50–105x — the ColumnStore's absolute time holds (~27 µs). Full spec: [VelesQL](docs/VELESQL_SPEC.md) · [Architecture](ARCHITECTURE.md)

---

## 2 · The agent memory model — MCP server + Python/Node bindings

Built on the database below it, the memory model is what your agent actually talks to: a local **MCP server** (`velesdb-memory`) for any agentic CLI, and the same semantics as **Python** (`pip install velesdb`) and **Node** (`@wiscale/velesdb-memory-node`) bindings. Two capabilities define it:

### `why()` — the recall that shows its evidence

Most "agent memory" is vector recall: it finds text that *looks like* your query. VelesDB's `MemoryService` **connects** memories with typed links, so it can answer *why* something happened by walking the graph to context that shares **no words** with your question — across process restarts, offline, no API key:

```python
from velesdb import MemoryService            # pip install velesdb

mem = MemoryService("./agent_memory")        # a real on-disk store; survives restarts
reason = mem.remember("Robert is recovering from knee surgery")
mem.remember("Booked the aisle seat on Robert's flight", links=[(reason, "because")])

# A *new* process, weeks later, reopens the same store and asks why:
mem.why("why the aisle seat on Robert's flight?")   # walks booking → reason — recall() can't
```

![recall() finds the booking but misses the reason; why() reaches it through typed links, across a session restart](examples/agent_memory/why_across_sessions.gif)

Memories are permanent by default; `forget(id)` deletes one, `ttl_seconds` gives a fact a durable expiry. Same wedge in **Python**, **Node** (`@wiscale/velesdb-memory-node`), as a local **[MCP server](crates/velesdb-memory)**, and in-memory in the **[TypeScript SDK](sdks/typescript)** (browser, no server).

<details>
<summary><strong>Proof it's not a weak-embedder trick — 4 runnable demos + benchmark position</strong></summary>

| Demo | What it shows |
|---|---|
| [`why_across_sessions.py`](examples/agent_memory/why_across_sessions.py) | the reason survives a process restart — recall stays blind, `why()` reaches it |
| [`why_magic_constant.py`](examples/agent_memory/why_magic_constant.py) | *why* a magic constant has its value — a business reason sharing no words with the code |
| [`memory_builds_its_own_graph.py`](examples/agent_memory/memory_builds_its_own_graph.py) | paste raw prose → a local model auto-wires the graph, `why()` walks it to the root cause |
| [`why_magic_constant.mjs`](crates/velesdb-node/examples/why_magic_constant.mjs) | the same wedge in the **Node** binding |

In each retrieval demo, recall stays blind to the reason **even under a real semantic embedder** (`ollama` / `all-minilm`) — the reason is connected by a decision, not by surface similarity, which is exactly what a vector store cannot follow.

On the standard LoCoMo memory test, our fully-local setup answers 56 % of the answerable questions and **55–61 % of time-related questions** — spanning both configurations the leading vendor's own paper reports for itself in that category on powerful cloud models, while we run on a model on your own machine. Cross-lab scores aren't fairly comparable (test setup alone can swing ~21 points), so instead of a bar chart we publish the [full sourced landscape, method, and statistics](crates/velesdb-memory/BENCHMARK.md).

Lower-level blocks (semantic / episodic / procedural memory, TTL, snapshots, reinforcement) and the single-VelesQL-statement recall across similarity + graph + session: [Agent Memory guide](docs/guides/AGENT_MEMORY.md).

</details>

### The context token optimizer — `why()` for your token bill

Agents burn most of their budget re-reading redundant context. The memory layer ships a **deterministic context compiler** (`compile_context` / `compile_transcript` over MCP, `ContextCompiler` in Rust): no LLM, no cloud — duplicates drop, repeated log lines collapse with counts, code / URLs / numbers / negative constraints survive verbatim, and over-budget content becomes a recoverable `ctx://source/` handle instead of a silent loss. Every decision carries a stable rule id, a reason, and a risk level. The same request always compiles to the same bytes.

**Measured against real provider billing** — the same session sent raw vs compiled, graded by a deterministic fact checklist (no LLM judge), raw logs committed verbatim:

| Billed A/B session (2026-07-19, claude-sonnet-5) | Runner | $ saved | Quality (raw vs compiled) |
|---|---|---|---|
| 19-turn feature session, cropped screenshots | Claude CLI | **10.9 %** | 22.8/23 vs 23.0/23 facts |
| Same session, real Retina-weight screenshots | Claude CLI | **21.9 %** | 23.0/23 vs 23.0/23 |
| 36-turn day-scale session | Claude CLI | **14.7 %** | 49.6/50 vs 49.2/50 * |
| 19-turn session, direct Messages API | API | **15.1 %** input tokens | 23.0/23 vs 23.0/23 |

> \* Two turns' grading key was later found defective (both arms scored full marks there; the parity conclusion stands) — [disclosure](examples/real-session-benchmark#billed-results-2026-07-19-all-real-executions). On the committed 12-turn corpus: **[82.5 % real (cl100k) input-token savings](crates/velesdb-memory/examples/context_savings)**, sub-millisecond stateless compiles; over a [36-turn session](examples/real-session-benchmark#long-session-36-turns--context-window-headroom) compiled context grows **1.7× slower**, so one session lasts far longer before hitting the window.

**Not a transparent proxy, and no automatic indexing** — the compiler only compresses what your agent explicitly hands it, never the harness's system prompt; nothing enters recallable memory without an explicit `remember`. Nothing leaves the machine. The [`velesdb-context-optimizer` skill](skills/velesdb-context-optimizer/SKILL.md) teaches the workflow — including when *not* to compress.

<details>
<summary><strong>Tool parity by surface (MCP / Node / Python / WASM) — we'd rather tell you than have you find out</strong></summary>

| Surface | Context-compiler tools today |
|---|---|
| **MCP server** ([`velesdb-memory`](crates/velesdb-memory)) + **Rust** | Full set: `compile_context`, `compile_transcript`, `retrieve_context_source`, `context_savings`, `save/load/list_working_contexts`, `explain_compilation` — MCP covers any other client |
| **Node** ([`@wiscale/velesdb-memory-node`](https://www.npmjs.com/package/@wiscale/velesdb-memory-node)) | `compileContext`, `retrieveContextSource`, `save/loadWorkingContext`, `feedback` — parity for `contextSavings`/`explainCompilation` in progress |
| **Python** (`pip install velesdb`) | Same set as Node plus `context_savings` merged on `develop`; the published wheel predates it — until the next release, Python agents reach the compiler through the MCP server |
| **WASM / TypeScript SDK** | `compileContext`, `retrieveContextSource`, `save/loadWorkingContext`, `listWorkingContexts` — all in-memory, intra-session only: useful to carry state across two calls in the same page load, not to resume after a reload (no IndexedDB/disk backend yet, [#1517](https://github.com/cyberlife-coder/VelesDB/issues/1517)) |

</details>

---

## Engine details, if you're evaluating seriously

### End-to-end search latency (canonical)

| Metric | Value |
|--------|-------|
| Search p50 (10K, 384D, WAL ON) | **450 us** |
| SIMD Dot Product (768D, AVX2) | **21.7 ns** |
| Recall@10 (Balanced) | **98.8%** |
| Quantization | PQ (8–32x), RaBitQ (32x), SQ8 (4x), Binary (32x) — [scope & caveats](docs/guides/QUANTIZATION.md) |

> **Provenance:** Intel Core **i9-14900KF** (x86_64, AVX2), full production path (VelesQL → HNSW → **WAL ON** → payload hydration). Per-machine figures vary; Apple-Silicon cross-checks and the full methodology live in [docs/BENCHMARKS.md](docs/BENCHMARKS.md).

<details>
<summary><strong>Micro-benchmarks, search modes, distance metrics, graph & columnstore</strong></summary>

**Index-only micro-benchmarks** (no WAL, no payload, hot cache — not comparable to end-to-end):

| Component micro-benchmark | Result | How to reproduce |
|-----------|--------|------------------|
| HNSW Search index-only (10K/768D, k=10) | **55 us** | `cargo bench -p velesdb-core --bench hnsw_benchmark -- hnsw_search_latency` |
| SIMD Dot Product (768D, AVX2) | **21.7 ns** | `cargo bench -p velesdb-core --bench simd_benchmark` |
| Recall@10 (Accurate mode) | **100%** | `cargo bench -p velesdb-core --bench recall_benchmark` |
| BM25 Sparse Search index-only (10K docs, top-10) | **57.6 us** | `cargo bench -p velesdb-core --bench sparse_benchmark -- top10_10k_corpus` |

**Search modes:**

| Mode | ef_search | Recall@10 | Use case |
|------|-----------|-----------|----------|
| Fast | 64 | 92.2% | Real-time suggestions, typeahead |
| Balanced (default) | 128 | 98.8% | Production search, RAG pipelines |
| Accurate | 512 | 100% | Evaluation, ground truth comparison |

**Distance metrics** — 5 metrics with SIMD acceleration (AVX-512, AVX2, NEON):

| Metric | Use case | SIMD perf (768D, AVX2, hot cache) |
|--------|----------|------------------|
| **Cosine** | Text embeddings, normalized vectors | 33 ns |
| **Euclidean** | Image features, when magnitude matters | 20 ns |
| **Dot Product** | Pre-normalized vectors, MIPS | 22 ns |
| **Hamming** | Binary embeddings, LSH, fingerprints | 36 ns |
| **Jaccard** | Sparse vectors, tags, set membership | 35 ns |

**Graph engine** — property graph, BFS/DFS, edge labels, Cypher-inspired MATCH integrated with vector search; cross-collection enrichment via `@collection`. [Guide](docs/guides/GRAPH_PATTERNS.md).

**ColumnStore engine** — typed columnar storage (the DuckDB/ClickHouse approach): filtering API **130x faster** than JSON scanning at 100K rows on the i9-14900KF reference (`JSON scan: 3.84 ms @ 100K → ColumnStore: 29.5 us @ 100K`; ~50–105x on Apple Silicon, where the JSON scan itself is faster — see footnote [2]); backs `JOIN` execution and `SELECT … WHERE` through an adaptive per-collection payload mirror compiled to RoaringBitmap scans.

**SIFT1M** standardized ANN benchmark (INRIA TEXMEX, 1M × 128D): [docs/BENCHMARKS.md § 11](docs/BENCHMARKS.md#11-sift1m--standard-ann-benchmark).

</details>

### Quick comparison

| | **VelesDB** | Chroma | Qdrant | pgvector |
|---|---|---|---|---|
| **Architecture** | Unified vector + graph + columnar | Vector only | Vector + payload | Vector extension for PostgreSQL |
| **Metadata filtering** | **Typed ColumnStore + secondary indexes** | JSON scan | JSON payload | SQL (PostgreSQL) |
| **Deployment** | Embedded / Server / WASM / Mobile | Server (Python) | Server (Rust) | Requires PostgreSQL |
| **Binary size** | ~9 MB | ~500 MB (with deps) | ~50 MB | N/A (PG extension) |
| **Graph support** | Native (MATCH clause) | No | No | No |
| **Query language** | VelesQL (SQL + NEAR + MATCH) | Python API | JSON API / gRPC | SQL + operators |
| **Browser (WASM) / Mobile** | Yes / Yes | No | No | No |
| **Offline / Local-first** | Yes | Partial | No | No |

> **Sweet spot:** vector + graph + structured filtering in one engine, local-first, lightweight, auditable. **Not the best fit (yet):** a managed cloud service with a multi-node distributed cluster. Competitor figures are typical public ranges — [run your own](docs/BENCHMARKS.md).

---

## VelesDB Premium — the enterprise control plane

The core engine is source-available and stays that way. **VelesDB Premium** builds the company-grade layer on top of the same binary, for organizations running agent fleets on sensitive data:

| Premium adds | What it means |
|---|---|
| **RBAC** | Granular role-based access on every endpoint — including the memory and context-compiler surfaces |
| **Audit trail** | Every sensitive action logged (who, what, when — metadata only, GDPR-conscious), forensic replay |
| **Multi-tenancy** | Hard per-tenant isolation of memory stores, two-level deletion rights |
| **Self-hosted resilience** | Clustering, air-gapped deployment, your infrastructure, your jurisdiction |
| **WebAdmin** | Administration UI for operators |

Pricing on quote — **contact@wiscale.fr** · [velesdb.com](https://velesdb.com). Built by [Wiscale](https://wiscale.fr) (France; GDPR and data-sovereignty native).

---

## Ecosystem & surfaces

| Surface | Package | Notes |
|---|---|---|
| Rust | [`velesdb-core`](https://crates.io/crates/velesdb-core) | The engine — embed it directly |
| Python | [`velesdb`](https://pypi.org/project/velesdb/) (3.9+) | Fastest onboarding path |
| Node | [`@wiscale/velesdb-memory-node`](https://www.npmjs.com/package/@wiscale/velesdb-memory-node) | Memory wedge ([full engine via server + TS SDK](crates/velesdb-node/README.md#need-the-full-engine)) |
| TypeScript / Browser | [`@wiscale/velesdb-sdk`](https://www.npmjs.com/package/@wiscale/velesdb-sdk) | WASM module ~550 KB gzipped, runs fully client-side |
| MCP server | [`velesdb-memory`](crates/velesdb-memory) | Agent memory + context compiler for any MCP client; prebuilt `.mcpb` bundles on the [MCP registry](https://registry.modelcontextprotocol.io) |
| REST server | [`velesdb-server`](https://crates.io/crates/velesdb-server) | 54 REST endpoints, [OpenAPI](docs/openapi.yaml), Docker multi-arch |
| Mobile / Desktop | [`velesdb-mobile`](crates/velesdb-mobile) · [Tauri plugin](crates/tauri-plugin-velesdb) | iOS / Android / desktop |

<details>
<summary><strong>API Reference (54 REST endpoints)</strong></summary>

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
<summary><strong>Server security</strong></summary>

- **API Key Authentication** — Bearer token auth via `VELESDB_API_KEYS` env var
- **TLS (HTTPS)** — Built-in via rustls (`VELESDB_TLS_CERT` / `VELESDB_TLS_KEY`)
- **Graceful Shutdown** — SIGTERM triggers connection drain + WAL flush. Zero data loss
- **Health Endpoints** — `GET /health` and `GET /ready` always public

> [docs/guides/SERVER_SECURITY.md](docs/guides/SERVER_SECURITY.md)

</details>

**Use cases in production shape:** agent memory that survives restarts · RAG with typed metadata filtering · vector + graph + filters in one query (e-commerce, recommendation) · desktop & mobile AI with zero backend. [Worked examples](examples/README.md) · [Demos](examples/).

---

## Known limitations — honest boundaries

VelesDB is honest about its scope: the items below are deliberate trade-offs or Enterprise-tracked features, **not correctness gaps** — the Community Edition is production-ready for single-node, local-first deployments.

<details>
<summary><strong>The full list, so you can make an informed technical choice</strong></summary>

| # | Limitation | Scope | Tracked |
|---|------------|-------|---------|
| 1 | **Single writer per collection** — WAL is serialized; concurrent writers contend on the same fsync lock. | Design trade-off (local-first, crash-safe by default). Read throughput is unaffected. | Concurrent WAL writer planned for [Premium](#velesdb-premium--the-enterprise-control-plane). See [docs/CONCURRENCY_MODEL.md](docs/CONCURRENCY_MODEL.md). |
| 2 | **No distributed replication** — single-node; no Raft, no sharding, no automatic failover in Core. | Deliberate: the sweet spot is local-first / embedded. | Raft-based replication tracked for Premium. |
| 3 | **No advanced RBAC / multi-tenant isolation in Core** — Core ships the `DatabaseObserver` enforcement seam (firing on every HTTP read path since 3.10.0), not the policy engine. | Core ships the hook, not the policy engine. | [Premium](#velesdb-premium--the-enterprise-control-plane) feature. |
| 4 | **WASM MATCH limited to 2 hops** — 3+ hop `MATCH` works fully in native builds. | Browser-build scope limit, not a correctness issue. | Tracked. |
| 5 | **SIFT1M fingerprint sidecar not yet committed** — the loader falls back to TOFU mode until the reference machine commits the pinned hashes. | Not a correctness issue — shape validation still applies. | Bootstrap shipped; sidecar pending. |
| 6 | **No head-to-head Docker Compose benchmark vs Qdrant / Chroma / FAISS yet** — SIFT1M already gives literature-comparable numbers. | Side-by-side numbers need infrastructure not frozen yet. | Tracked. |

Internal technical limitations (query-planner approximations, plan-cache semantics): [docs/reference/KNOWN_LIMITATIONS.md](docs/reference/KNOWN_LIMITATIONS.md).

</details>

## Quality bar

- `cargo test --workspace` — 9k+ tests across Rust, TypeScript, and Python run in CI on every merge; the exact commands live in [QUALITY_BAR.md](QUALITY_BAR.md).
- Every marketing number is pinned to its committed source by a CI-enforced [promise contract](docs/reference/promise-contract.json).
- Limitations are published next to the strengths — see above — including the ones we haven't fixed yet.

**The story:** VelesDB was born in France out of a simple observation — **EU data sovereignty is an architectural problem, not a legal one**. Hosting on a US provider's EU region is a latency decision, not a sovereignty decision; VelesDB removes the US provider from the chain entirely: one local binary, no API key, no data processor, your jurisdiction. [Full story on dev.to](https://dev.to/wiscale-fr/i-built-a-database-in-france-because-the-cloud-act-makes-eu-data-sovereignty-impossible-5325) · [Roadmap](ROADMAP.md) · [Changelog](CHANGELOG.md)

## Contributing & contact

Contributions welcome — start with [CONTRIBUTING.md](CONTRIBUTING.md) and the [good first issues](https://github.com/cyberlife-coder/VelesDB/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22). Security reports: see [SECURITY.md](SECURITY.md).

**License:** [VelesDB Core License 1.0](LICENSE) (source-available). Premium: commercial license.
**Contact:** contact@wiscale.fr · [velesdb.com](https://velesdb.com)
