# velesdb-memory

[![crates.io](https://img.shields.io/crates/v/velesdb-memory?logo=rust&label=crates.io)](https://crates.io/crates/velesdb-memory)
[![docs.rs](https://img.shields.io/docsrs/velesdb-memory?logo=docsdotrs&label=docs.rs)](https://docs.rs/velesdb-memory)
[![npm](https://img.shields.io/npm/v/%40wiscale%2Fvelesdb-memory-node?logo=npm&label=npm)](https://www.npmjs.com/package/@wiscale/velesdb-memory-node)
[![PyPI](https://img.shields.io/pypi/v/velesdb?logo=pypi&logoColor=white&label=PyPI)](https://pypi.org/project/velesdb/)
[![MCP registry](https://img.shields.io/badge/MCP_registry-io.github.cyberlife--coder%2Fvelesdb--memory-1f6feb?logo=modelcontextprotocol&logoColor=white)](https://registry.modelcontextprotocol.io)
[![license: VelesDB Core 1.0](https://img.shields.io/badge/license-VelesDB_Core_1.0_(source--available)-e8702a)](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE)

**The explainable, local-first memory engine for AI agents — as a single MCP
server.** Give your coding agent durable memory that never leaves your machine:
it remembers decisions, recalls them semantically, and — the differentiator —
**connects** them so it can answer *why* a decision was made, not just retrieve
look-alike text. That auditable `why()` recall trail is the kind of
traceability the [EU AI Act](https://artificialintelligenceact.eu/implementation-timeline/)
(enforceable from Aug 2026) asks of AI systems; running fully local, it **helps
meet** those data-residency and explainability expectations rather than claiming
certified compliance.

> **Release 0.8.0** — deterministic context compiler (`compile_context`,
> `context_savings`, `explain_compilation`, `retrieve_context_source`);
> published to the registries by the `velesdb-memory-v0.8.0` tag, so the
> links below may briefly lag right after merge. `velesdb-memory` ships on
> [crates.io](https://crates.io/crates/velesdb-memory) and on the
> [official MCP registry](https://registry.modelcontextprotocol.io)
> (`io.github.cyberlife-coder/velesdb-memory`, with **5 prebuilt `.mcpb` bundles**:
> macOS arm64/x64, Linux arm64/x64, Windows x64). Bindings: Node
> [`@wiscale/velesdb-memory-node`](https://www.npmjs.com/package/@wiscale/velesdb-memory-node) **0.8.0**
> and Python in [`velesdb`](https://pypi.org/project/velesdb/) **3.12.0**
> (memory API — the context compiler is **not exposed in Python yet**;
> Python agents reach it through the MCP server).
> **`cargo install velesdb-memory` installs the latest published release.**

> **Bring your own reranker (Rust)**: `compile_context_reranked` hands the
> full fused candidate pool (vector + graph, pre-cutoff) to any
> [`Reranker`] you inject — a cross-encoder, an LLM judge — and its
> ordering decides which memories get compiled in. Never a default, and
> deliberately not on the wire: the shipped `DeterministicReranker` is
> lexical, and a lexical second stage demotes exactly the
> zero-vocabulary-overlap evidence the graph walk rescues (both behaviours
> pinned by tests). `recall_fused_reranked` is the same seam for plain
> recall.

Built on [VelesDB](https://velesdb.com)'s in-core Agent Memory SDK, which fuses
three engines behind its memory tools:

| Tool       | What it does                                               | Engines |
|------------|------------------------------------------------------------|---------|
| `remember` | store a fact, optionally linked + tagged with metadata, with an optional expiry (`ttl_seconds`) | Vector + Graph + ColumnStore |
| `recall`   | semantic retrieval, optional exact-match metadata filter   | Vector + ColumnStore |
| `relate`   | create a typed edge between two memories                   | Graph |
| `recall_fused` | recall with graph-aware re-ranking (vector + typed links fused) | Vector + Graph |
| `recall_where` | recall filtered by typed column predicates (ranges, comparisons) | Vector + ColumnStore |
| `forget`   | delete a memory                                            | — |
| `why`      | recall a decision **+ its connected subgraph** (multi-hop) | Vector + Graph + ColumnStore |
| `feedback` | reinforce a recalled fact (**useful/noise**) — `recall` re-ranks by this learned confidence, so the memory **improves with use** without retraining | Vector |
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

## How it compares — and who it's for

velesdb-memory is **embedded memory, not a cloud memory service.** The
difference isn't a benchmark bar chart — it's three things no competitor
counters: an **evidence trail you can audit** (`why()` shows which facts an
answer came from), **zero AI calls to store a memory** (the incumbents run 2–3
AI-model calls per save — by default, paid cloud calls), and **published
retrieval numbers** — we measure, with no AI grader in the loop, how often the
memory finds the right information; to our knowledge, nobody else in this
market publishes that at all:

| | **velesdb-memory** | Mem0 | Zep / Graphiti |
|---|---|---|---|
| What it is | one embedded binary (vector + graph + column engines) | coordinator over separate services (Qdrant + Postgres) | coordinator, graph-centric (needs Neo4j/FalkorDB) |
| AI calls to store a memory | **zero required** (optional extraction runs on your local model) | AI-model calls on every write (cloud by default) | AI-model calls on every write (cloud by default) |
| Runs | **100% local / offline** | self-host still needs an AI service in the write path | Zep's self-hosted edition was discontinued; Graphiti needs a graph database + an AI service |
| Explains its answers | **yes** — `why()` returns the evidence trail | no — returns an answer only | no — returns an answer only |
| Publishes retrieval accuracy | **yes** — [+7.2pts multi-hop, +9.7pts time-scoped, no AI grader](BENCHMARK.md) | no | no |
| Time-related questions on LoCoMo | **55–61%** on a fully local model — floor = without the optional scaffold ([method + stats](BENCHMARK.md)) | 55.5% base / 58.1% graph-enhanced "Mem0g" (its own best score), both on cloud AI ([own paper](https://arxiv.org/abs/2504.19413)) | 49.3% on cloud AI — [as measured in Mem0's evaluation](https://arxiv.org/abs/2504.19413), which Zep disputes |

*Why no single "overall score" comparison row? Because overall scores from
different labs can't be fairly compared: the same product's score can swing
~21 points between two test setups, and vendor headlines often diverge widely
from what other labs measure. Our fully-local 56% aggregate comes with the
full method and statistics disclosed, and instead of a bar chart we publish
the complete sourced landscape — who measured what, with which AI models, and
which figures are disputed: [`BENCHMARK.md`](BENCHMARK.md).*

**Choose velesdb-memory when local-first is a requirement, not a preference:**
- **Regulated / sovereign data** (health, legal, finance, defense) — context can't transit a third-party LLM API; `why()` gives both data residency and an auditable recall trail.
- **Air-gapped / on-prem / edge** — a self-contained binary against a local model is the only shape that deploys with no outbound internet.
- **Cost-sensitive, high-volume agents** — running extraction + recall on a local stack removes the per-token cloud bill.

If you're cloud-native and want the largest community, Mem0 is the default reach. If your
data can't leave the box — or you need to *audit why* it recalled something — this is the
one that fits. (Deeper positioning: [`POSITIONING.md`](POSITIONING.md).)

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
| **Graph** — *replicated* | 2WikiMultiHopQA (1 000 dev) | supporting-fact recall, second independently built dataset | **+2.6 to +3.1pp** on its three bridged types (+2.1pp overall) |
| **ColumnStore** (`recall_where`) | TimeQA (real Wikipedia bios) | time-scoped recall a year-range filter can do and cosine can't | **+9.7pp** gold-sentence recall |
| **Tri-engine** (compound) | synthetic, multi-hop **and** time-scoped | do the engines *stack*? | **+29pp** together — more than the sum of each alone |

Read it straight: the graph helps exactly where a second hop is required — and the
lift survives moving to a *different* multi-hop dataset (more modest there, +2.1pp
overall, stated as measured — not a one-dataset fluke). The ColumnStore wins where
the answer hinges on a number cosine cannot rank. And on a task that needs *both*,
they compound rather than merely coexist. A pure vector store / RAG orchestrator
has none of these — it ranks by similarity and stops.

## Install

**One command (recommended, with a Rust toolchain present):**

```bash
cargo install velesdb-memory
# → installs the `velesdb-memory` MCP server binary onto your PATH
```

The binary is tiny, zero-dependency, and fully offline. It speaks MCP over
**stdio**, so client and server run on the same machine and the memory never
leaves it.

**From the workspace (for hacking on the server itself):**

```bash
cargo build --release -p velesdb-memory   # → target/release/velesdb-memory
```

> **In an MCP client (no Rust toolchain needed):** velesdb-memory is listed on the
> [official MCP registry](https://registry.modelcontextprotocol.io) as
> `io.github.cyberlife-coder/velesdb-memory`. Registry-aware clients can install it
> straight from the per-platform `.mcpb` bundles attached to each
> [GitHub release](https://github.com/cyberlife-coder/VelesDB/releases). A
> `curl | sh` / Homebrew installer is a tracked follow-up; with a Rust toolchain,
> `cargo install velesdb-memory` is the supported one-liner.

## Configure your client

All clients use the same stdio shape — point `command` at the built binary.
`cargo install velesdb-memory` puts it at `~/.cargo/bin/velesdb-memory`
(or the path of your local build, `target/release/velesdb-memory`).

**Claude Code**

```bash
claude mcp add velesdb-memory \
  --env VELESDB_MEMORY_PATH="$HOME/.velesdb-memory" \
  -- ~/.cargo/bin/velesdb-memory
```

**Cursor** — `~/.cursor/mcp.json` (global) or `.cursor/mcp.json` (per project)

```json
{ "mcpServers": { "velesdb-memory": {
  "command": "~/.cargo/bin/velesdb-memory",
  "env": { "VELESDB_MEMORY_PATH": "/home/you/.velesdb-memory" }
} } }
```

**Cline** — `cline_mcp_settings.json` — same `mcpServers` block as Cursor.

**Zed** — `settings.json`

```json
{ "context_servers": { "velesdb-memory": {
  "command": { "path": "~/.cargo/bin/velesdb-memory", "args": [],
    "env": { "VELESDB_MEMORY_PATH": "/home/you/.velesdb-memory" } }
} } }
```

**Codex CLI** — `codex mcp add`, or a `[mcp_servers.*]` table in `~/.codex/config.toml`

```bash
codex mcp add velesdb-memory \
  --env VELESDB_MEMORY_PATH="$HOME/.velesdb-memory" \
  -- ~/.cargo/bin/velesdb-memory
```

```toml
# equivalent ~/.codex/config.toml entry
[mcp_servers.velesdb-memory]
command = "~/.cargo/bin/velesdb-memory"
args = []
env = { VELESDB_MEMORY_PATH = "/home/you/.velesdb-memory" }
```

**opencode** — `opencode.json` (per project) or `~/.config/opencode/opencode.json` (global)

```json
{ "mcp": { "velesdb-memory": {
  "type": "local",
  "command": ["~/.cargo/bin/velesdb-memory"],
  "enabled": true,
  "environment": { "VELESDB_MEMORY_PATH": "/home/you/.velesdb-memory" }
} } }
```

## Teach your agent the flow (skill)

Wiring the MCP server gives your agent the *tools*; it doesn't tell it *when* to
use them — and the differentiator (`why`) only pays off if the agent builds the
graph as it works. Ship it the flow with the bundled **agent skill**:

```bash
# Claude Code / opencode: copy the skill into your skills directory
cp -r crates/velesdb-memory/skill/velesdb-memory ~/.claude/skills/
```

[`skill/velesdb-memory/SKILL.md`](skill/velesdb-memory/SKILL.md) teaches the agent
the loop — *recall before acting → remember decisions with metadata **and** links →
`relate` facts as relationships appear → `why` to explain → `feedback` to reinforce* —
with concrete scenarios (incident→decision→"why?", onboarding, cross-session
continuity). Without it, an agent will call `recall` at best and never build the
graph that makes `why` shine.

A second bundled skill, **`velesdb-context-optimizer`**, teaches the compiler
workflow below (when/what to compress, how to read `risk`). Install it the
same way:

```bash
cp -r skills/velesdb-context-optimizer ~/.claude/skills/
```

[`skills/velesdb-context-optimizer/SKILL.md`](../../skills/velesdb-context-optimizer/SKILL.md)
— see [The context compiler tools](#the-context-compiler-tools) below.

## Using the tools

Once configured, your agent discovers the tools automatically (via MCP
`tools/list`). Each takes JSON and returns JSON:

```jsonc
// remember — store a fact; returns a stable, content-derived id
//            (re-remembering identical text is idempotent — same id, updated in place)
remember { "fact": "we chose parking_lot to avoid lock poisoning",
           "metadata": { "project": "checkout" },                  // optional → enables filtering
           "links":   [ { "target": 1234, "relation": "decided_in" } ],  // optional typed edges
           "ttl_seconds": 604800 }                                 // optional → expires in 7 days
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

### The context compiler tools

**Why:** agents spend most of their tokens re-reading redundant context.
`compile_context` compresses it **deterministically** — no LLM, no cloud, no
API key: same request, byte-identical output. What must survive verbatim
does (code fences, URLs, numbers/dates/ids, negative constraints, anything
marked `{"verbatim": true}`); duplicates drop; repeated log lines collapse
with counts (`ERROR timeout (x50)`); over-budget content becomes a
recoverable `ctx://source/` handle instead of a silent loss; and every
fragment gets one auditable decision (stable rule id, reason, relevance,
risk). Guarantees, per compilation:

- **Budget**: the assembled content never exceeds `token_budget`.
- **Provenance**: `sources` + per-decision `content_hash` identify the exact
  bytes; `retrieval_handles` list what was externalized.
- **Nothing critical silently lost**: losing preserve-classified content
  raises the compilation's `risk` to `"high"` — check it before use.

```jsonc
// compile_context — deterministic compression under a token budget
compile_context { "query": "state of the canary deploy",
                  "token_budget": 4000,
                  "project": "veles",
                  "memory_scope": { "k": 5 },                 // optional: pull relevant memories in
                  "fragments": [
                    { "content": "You are the deploy assistant.", "metadata": { "cache": true } },
                    { "content": "<600 lines of CI logs>", "kind": "log" },
                    { "content": "Never restart the primary during a rebalance." } ] }
→ { "content": "…", "sections": […], "decisions": […], "sources": […],
    "retrieval_handles": […], "insights": { "tokens_in": 2244, "tokens_out": 545,
    "tokens_saved": 1699, … }, "risk": "low" }

// retrieve_context_source — what was externalized is recoverable, byte for byte
retrieve_context_source { "handle": "ctx://source/1234567890" }
→ { "handle": "ctx://source/1234567890", "content": "…original bytes…" }

// explain_compilation — "why was this fragment dropped/shortened?" (stateless:
//   compilation is deterministic, so the request is re-compiled)
explain_compilation { "request": { …same request… }, "fragment_id": 1234567890 }
→ { "action": "drop", "rule_id": "drop.duplicate", "reason": "…", "risk": "low", … }

// context_savings — aggregate recorded savings, optionally per project
context_savings { "project": "veles" }
→ { "events": 12, "tokens_in": …, "tokens_saved": …, "truncated": false }
```

Preservation rules (stable ids, first match wins): `preserve.marked_verbatim`,
`cache.stable_prefix` (cache-marked fragments form a stable prefix for
provider prompt caching), `preserve.code_fence`,
`preserve.negative_constraint`, `abstract.log_dedup`,
`preserve.exact_values`, `preserve.url`, `preserve.default`; the budget layer
adds `budget.externalize` and dedup adds `drop.duplicate` /
`drop.near_duplicate`.

`insights.tokens_saved` is a **local estimate**, calibrated against a real
BPE (cl100k) to deliberately over-count every measured content class
(+13 %…+55 %) — not the provider's count, not billed tokens, not cache reads.
The reproducible benchmark ([`examples/context_savings`](examples/context_savings))
measures **82.5 % real (cl100k) token savings on a committed 12-turn agent-session benchmark** (sub-ms stateless compiles), 75–82 % estimated savings on its static corpus in ~2 ms compile, and — with `memory_scope`'s fused HNSW + graph-walk recall over `relate`-linked fact chains — **9/9 answer facts surfaced vs 3/9 for vector-only recall** on the committed tri-engine benchmark
latency. The [`velesdb-context-optimizer`](../../skills/velesdb-context-optimizer/SKILL.md)
skill teaches an agent the full workflow — including when *not* to compress.

**IDs & linking.** `remember` returns a stable id derived from the fact's
content. Pass it to `relate` / `forget`, or as a `links[].target` on a later
`remember` — that is how the graph gets built, and what `why` traverses.

**A natural agent pattern.** At the end of a task, `remember` the decision with
`metadata` (project, author, status) and a `link` to the PR or ticket. Days
later, `why("…")` recovers not just the decision but the PR, ticket, and
benchmark linked to it — where `recall` alone returns only look-alike text.

**Forgetting & expiry.** Facts are permanent by default. Delete one explicitly
with `forget { "id": … }`. To make a fact self-expire, pass `ttl_seconds` to
`remember` (a durable TTL persisted with the fact, so it survives a restart;
expired facts stop being recalled). Set `VELESDB_MEMORY_DEFAULT_TTL` (seconds) to
apply a default expiry to every fact that doesn't set its own. To wipe everything,
delete the store directory at `VELESDB_MEMORY_PATH`.

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

### License FAQ

**Is this open source?** It is **source-available**: the full source is
readable, modifiable, and redistributable under the **VelesDB Core License 1.0**
(a derivative of the Elastic License 2.0). It is not an OSI-approved license.

**Can I use it at work / in a commercial product?** **Yes.** Running the server
locally, or embedding the library inside your own application where *your* users
only ever receive results (a memory, a `why()` subgraph), is expressly permitted
— the license's **embedded, local-first use** clause.

**What's actually forbidden?** Re-hosting VelesDB as a multi-tenant *service*
where third parties drive the database (run arbitrary queries, manage
collections/indexes/graph nodes). This server makes that impossible by design:
it exposes **memory semantics only** (`remember/recall/relate/forget/why`),
never raw `query` / `create_collection` / `upsert` / `traverse`.

**Why this license?** So that *you* can embed agent memory locally and freely,
while a third party cannot turn *our* engine into a memory-as-a-service and
resell it. The moat protects the project, not your usage.

**What do I owe when I redistribute?** Keep the LICENSE file and copyright
notices, and add a [velesdb.com](https://velesdb.com) attribution in any public
app that ships the binary. Internal, dev, and test use need no attribution.

> Full terms and the canonical FAQ: [LICENSE](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE).
> Questions: contact@wiscale.fr.
