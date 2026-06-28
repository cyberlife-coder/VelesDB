# velesdb-memory

**Local-first memory for AI agents — a single MCP server.** Give your coding
agent durable memory that never leaves your machine: it remembers decisions,
recalls them semantically, and — the differentiator — **connects** them so it
can answer *why* a decision was made, not just retrieve look-alike text.

Built on [VelesDB](https://velesdb.com)'s in-core Agent Memory SDK, which fuses
three engines behind its memory tools:

| Tool       | What it does                                               | Engines |
|------------|------------------------------------------------------------|---------|
| `remember` | store a fact, optionally linked + tagged with metadata     | Vector + Graph + ColumnStore |
| `recall`   | semantic retrieval, optional exact-match metadata filter   | Vector + ColumnStore |
| `relate`   | create a typed edge between two memories                   | Graph |
| `forget`   | delete a memory                                            | — |
| `why`      | recall a decision **+ its connected subgraph** (multi-hop) | Vector + Graph + ColumnStore |
| `remember_extracted` | extract facts from raw text + **auto-build the graph** (opt-in backend) | Vector + Graph |

`why` is the wedge: it surfaces related memories (the PR, the ticket, the
benchmark) reachable through typed links **even when they share no words** with
your question — exactly what a pure vector search is blind to.

By design the server exposes **memory semantics only** — never raw database
capabilities (`query`, `create_collection`, `upsert`, `traverse`). See
[License](#license).

## See it (offline, one command)

![velesdb-memory wow demo: a vector recall misses the 2-hop ticket; why() reaches it through the graph](https://raw.githubusercontent.com/cyberlife-coder/VelesDB/develop/crates/velesdb-memory/media/wow.gif)

```bash
cargo run -p velesdb-memory --example wow_offline
```

```text
recall("why we chose parking_lot")   [vector similarity only]
   0.47  we chose parking_lot to avoid lock poisoning after a panic
   0.18  PR #42 swaps the std Mutex for parking_lot
   └─ EPIC-317 is nowhere here — it shares no words with the question.

why("why we chose parking_lot")      [vector seed + graph traversal]
   hop 0  we chose parking_lot ...
   hop 1  PR #42 ...
   hop 2  EPIC-317: intermittent CI hang under load
   └─ the graph reached the very ticket the decision fixed.
```

A vector search ranks by resemblance; the ticket shares no words with the
question, so a pure similarity search is blind to it. `why()` follows the typed
links and reaches it. That gap is the product.

### Four runnable demos of the wedge

Each is a real run that shows what plain recall misses and `why()` recovers:

| Demo | What it shows |
|---|---|
| [`why_across_sessions.py`](../../examples/agent_memory/why_across_sessions.py) | the reason survives a process restart — recall of the top 5 of 16 memories stays blind, `why()` reaches it |
| [`why_magic_constant.py`](../../examples/agent_memory/why_magic_constant.py) | *why* a magic constant has its value — a business reason that shares no words with the code |
| [`memory_builds_its_own_graph.py`](../../examples/agent_memory/memory_builds_its_own_graph.py) | paste raw prose → a local model auto-wires the graph (no `relate()`), `why()` walks it to the root cause |
| [`why_magic_constant.mjs`](../velesdb-node/examples/why_magic_constant.mjs) (Node) | the same engine and wedge in the `@wiscale/velesdb-memory-node` binding |

> **Not a weak-embedder trick.** In each retrieval demo, recall stays blind to the
> reason **even under a real semantic embedder** (`ollama` / `all-minilm`), not just
> the offline `hash` default — the reason is connected by a *decision*, not by surface
> similarity, which is exactly what a vector store cannot follow.

### Benchmark

`cargo run --release -p velesdb-memory --example bench_multihop` isolates the
graph's contribution — 24 `decision → PR → problem` chains, the same embedder
throughout, only the graph toggled. Each question (`"why did we adopt <tech>"`)
has a 1-hop answer (the decision, shares words) and a 2-hop answer (the original
problem, shares none):

| embedder | direct recall | multi-hop, vector-only | multi-hop, **vector + graph** |
|----------|:-------------:|:----------------------:|:-----------------------------:|
| `hash` (deterministic) | 100% | 0% | **100%** |
| real model (Ollama `all-minilm`) | 100% | 33% | **100%** |

Read it this way: the **direct** control confirms the vector engine is healthy
(100% — it aces look-alike retrieval). On **multi-hop**, a real semantic embedder
still recovers only a third of the answers (the problem shares no words with the
question); the graph recovers all of them — **+67 pp** with a real model
(structurally +100 pp with the deterministic one). Run the real one yourself:

```bash
cargo build --release -p velesdb-memory --features ollama && ollama pull all-minilm
VELESDB_MEMORY_EMBEDDER=ollama \
  cargo run --release -p velesdb-memory --features ollama --example bench_multihop
```

> **Engine isolation, and extraction.** `bench_multihop` measures the *engine's*
> contribution on controlled data with the graph pre-wired, so the numbers
> reflect retrieval, not an LLM. For end-to-end *extraction* (turning raw text
> into the graph automatically), the server ships an opt-in layer — the
> `remember_extracted` tool / `MemoryService::remember_extracted`, backed by the
> dependency-free `Extractor` trait (bring your own LLM) or the built-in
> `OllamaExtractor` behind `--features extract`. The apples-to-apples comparison
> on the real [LoCoMo](https://github.com/snap-research/locomo) dataset lives in
> [`examples/locomo/`](examples/locomo/README.md): it builds a fact↔entity graph
> from the conversations and scores the graph's QA contribution with a hybrid
> LLM-judge + deterministic metric. The core stays bring-your-own-links;
> extraction is a commodity on top.

### On public benchmarks — each engine, measured

The controlled demo above proves the *idea*; these run the same engines on
**public, third-party datasets** with **generation-free** metrics (pure retrieval
recall — no LLM in the scoring loop, so the number is the memory, not a model).
Each engine is isolated against a pure-vector baseline. Full method, tables and
honest limits in [`BENCHMARK.md`](BENCHMARK.md) and [`POSITIONING.md`](POSITIONING.md);
every figure reproduces from the bundled examples.

| Engine | Public benchmark | What it measures | Vector → fused |
|---|---|---|---|
| **Graph** (`why()` BFS) | HotpotQA (3 000 dev, distractor) | retrieving *both* bridge facts of a multi-hop question | **+7.2pp** both-facts on bridge questions (+5.6pp all types) |
| **Graph** — *replicated* | 2WikiMultiHopQA (1 000 dev) | same metric, second independent dataset | **+3.1pp** on bridged types (+2.1pp overall) |
| **ColumnStore** (`recall_where`) | TimeQA (real Wikipedia bios) | time-scoped recall a year-range filter can do and cosine can't | **+9.7pp** gold-sentence recall |
| **Tri-engine** (compound) | synthetic, multi-hop **and** time-scoped | do the engines *stack*? | **+29pp** together — more than the sum of each alone |

Read it straight: the graph helps exactly where a second hop is required — and the
lift survives moving to a *different* multi-hop dataset (more modest there, +2.1pp
overall, stated as measured — not a one-dataset fluke). The ColumnStore wins where
the answer hinges on a number cosine cannot rank. And on a task that needs *both*,
they compound rather than merely coexist. A pure vector store / RAG orchestrator
has none of these — it ranks by similarity and stops.

## Install

```bash
# Default build: tiny, zero-dependency, fully offline.
cargo build --release -p velesdb-memory
# → target/release/velesdb-memory
```

The binary speaks MCP over **stdio**, so client and server run on the same
machine and the memory never leaves it.

## Configure your client

All clients use the same stdio shape — point `command` at the built binary.

**Claude Code**

```bash
claude mcp add velesdb-memory \
  --env VELESDB_MEMORY_PATH="$HOME/.velesdb-memory" \
  -- /path/to/velesdb-memory
```

**Cursor** — `~/.cursor/mcp.json` (global) or `.cursor/mcp.json` (per project)

```json
{ "mcpServers": { "velesdb-memory": {
  "command": "/path/to/velesdb-memory",
  "env": { "VELESDB_MEMORY_PATH": "/home/you/.velesdb-memory" }
} } }
```

**Cline** — `cline_mcp_settings.json` — same `mcpServers` block as Cursor.

**Zed** — `settings.json`

```json
{ "context_servers": { "velesdb-memory": {
  "command": { "path": "/path/to/velesdb-memory", "args": [],
    "env": { "VELESDB_MEMORY_PATH": "/home/you/.velesdb-memory" } }
} } }
```

**Codex CLI** — `codex mcp add`, or a `[mcp_servers.*]` table in `~/.codex/config.toml`

```bash
codex mcp add velesdb-memory \
  --env VELESDB_MEMORY_PATH="$HOME/.velesdb-memory" \
  -- /path/to/velesdb-memory
```

```toml
# equivalent ~/.codex/config.toml entry
[mcp_servers.velesdb-memory]
command = "/path/to/velesdb-memory"
args = []
env = { VELESDB_MEMORY_PATH = "/home/you/.velesdb-memory" }
```

**opencode** — `opencode.json` (per project) or `~/.config/opencode/opencode.json` (global)

```json
{ "mcp": { "velesdb-memory": {
  "type": "local",
  "command": ["/path/to/velesdb-memory"],
  "enabled": true,
  "environment": { "VELESDB_MEMORY_PATH": "/home/you/.velesdb-memory" }
} } }
```

## Using the tools

Once configured, your agent discovers the tools automatically (via MCP
`tools/list`). Each takes JSON and returns JSON:

```jsonc
// remember — store a fact; returns a stable, content-derived id
//            (re-remembering identical text is idempotent — same id, updated in place)
remember { "fact": "we chose parking_lot to avoid lock poisoning",
           "metadata": { "project": "checkout" },                  // optional → enables filtering
           "links":   [ { "target": 1234, "relation": "decided_in" } ] }  // optional typed edges
→ { "id": 9876543210 }

// relate — add a typed edge between two existing memories
relate { "from": 9876543210, "to": 1234, "relation": "depends_on" }
→ { "edge_id": 42 }

// recall — semantic search; optional exact-match metadata filter (ColumnStore)
recall { "query": "billing retries", "limit": 5, "filter": { "project": "checkout" } }
→ { "memories": [ { "id": 9876543210, "score": 0.59, "content": "…" }, … ] }

// why — the differentiator: best match + its connected subgraph (multi-hop)
why { "decision": "why did we choose parking_lot", "max_hops": 2,
      "filter": { "project": "checkout" } }
→ { "nodes": [ { "id": …, "content": "…", "hop": 0 }, … ],
    "edges": [ { "from": …, "to": …, "relation": "decided_in" }, … ] }

// forget — delete a memory by id
forget { "id": 9876543210 } → { "id": 9876543210 }

// remember_extracted — extract facts from raw text and auto-wire the graph
//   (opt-in: needs a server built with --features extract + VELESDB_MEMORY_EXTRACTOR)
remember_extracted { "text": "Met Dana at the Rust meetup; she now leads the parser rewrite." }
→ { "ids": [ 11122233, 44455566 ] }   // stored facts; topics become shared graph hubs
```

`limit` defaults to 10 (capped at 1000); `max_hops` defaults to 2 (capped at 10);
`links`, `metadata`, and `filter` are optional.

**IDs & linking.** `remember` returns a stable id derived from the fact's
content. Pass it to `relate` / `forget`, or as a `links[].target` on a later
`remember` — that is how the graph gets built, and what `why` traverses.

**A natural agent pattern.** At the end of a task, `remember` the decision with
`metadata` (project, author, status) and a `link` to the PR or ticket. Days
later, `why("…")` recovers not just the decision but the PR, ticket, and
benchmark linked to it — where `recall` alone returns only look-alike text.

> **Embedding the library directly?** The same wedge is available without the
> MCP server: as a **Rust** API (`MemoryService::remember/recall/relate/forget/why`,
> see the rustdoc on [docs.rs](https://docs.rs/velesdb-memory)), in **Python**
> (`from velesdb import MemoryService`), and in **Node.js**
> (`npm install @wiscale/velesdb-memory-node`).

## Embedding backend

`remember` / `relate` / `why` / `forget` behave the same regardless of the
embedder — the graph is what makes `why` shine. Only `recall`'s semantic
quality (and `why`'s seed match) depend on it.

| `VELESDB_MEMORY_EMBEDDER` | Recall quality | Footprint | Needs |
|---------------------------|----------------|-----------|-------|
| `hash` (default)          | keyword-ish, deterministic | tiny, **fully offline, zero-dep** | nothing |
| `ollama`                  | real semantic  | tiny binary + your local model | a running Ollama; build `--features ollama` |

The default keeps the *single tiny offline binary* promise intact. For real
semantic recall, build with the `ollama` feature and point it at a local model
— the model runs in your own Ollama, so memory still never leaves the machine:

```bash
cargo build --release -p velesdb-memory --features ollama
ollama pull all-minilm
VELESDB_MEMORY_EMBEDDER=ollama \
VELESDB_MEMORY_OLLAMA_MODEL=all-minilm \
  /path/to/velesdb-memory
```

Env vars: `VELESDB_MEMORY_OLLAMA_URL` (default `http://localhost:11434`),
`VELESDB_MEMORY_OLLAMA_MODEL` (default `all-minilm`). The embedding dimension is
probed from the model, so a store is fixed to one embedder — don't switch
embedders on an existing store.

## Auto-extraction backend (opt-in)

By default the graph is **bring-your-own-links**: you wire edges with `relate`
or `links`. The `remember_extracted` tool turns that into a commodity — a local
LLM reads raw text and the server stores its facts + auto-builds the fact↔topic
graph. It is off by default (it pulls an HTTP dependency), so the standard
binary stays tiny and offline:

```bash
cargo build --release -p velesdb-memory --features extract
VELESDB_MEMORY_EXTRACTOR=ollama \
VELESDB_MEMORY_EXTRACTOR_MODEL=qwen3.6:35b-mlx \
  /path/to/velesdb-memory
```

Env vars: `VELESDB_MEMORY_EXTRACTOR` (`ollama` to enable), `VELESDB_MEMORY_EXTRACTOR_URL`
(default `http://localhost:11434`), `VELESDB_MEMORY_EXTRACTOR_MODEL` (required, a
generative model). Without a backend the tool returns a clear "not configured"
error. To plug a different model, implement the dependency-free `Extractor`
trait and pass it to `MemoryService::remember_extracted` from Rust.

## License

The distributed binary embeds `velesdb-core` and is therefore governed by the
**VelesDB Core License 1.0** (source-available): redistribution must keep the
license and notices, with [velesdb.com](https://velesdb.com) attribution for
public apps. The wrapper source in this crate is intentionally readable and
forkable.

**By design, this server exposes memory semantics only** —
`remember/recall/relate/forget/why`, which return *results*. It never exposes
raw database capabilities (`query`, `create_collection`, `upsert`, `traverse`).
Run locally over stdio, you operate the software for yourself: this is the
license's expressly-permitted **embedded, local-first use** — not a hosted
service to third parties.
