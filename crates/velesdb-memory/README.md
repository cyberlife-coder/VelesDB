# velesdb-memory

> The explainable, local-first memory engine for AI agents, as a single MCP server.

[![crates.io](https://img.shields.io/crates/v/velesdb-memory?logo=rust&label=crates.io)](https://crates.io/crates/velesdb-memory)
[![docs.rs](https://img.shields.io/docsrs/velesdb-memory?logo=docsdotrs&label=docs.rs)](https://docs.rs/velesdb-memory)
[![npm](https://img.shields.io/npm/v/%40wiscale%2Fvelesdb-memory-node?logo=npm&label=npm)](https://www.npmjs.com/package/@wiscale/velesdb-memory-node)
[![PyPI](https://img.shields.io/pypi/v/velesdb?logo=pypi&logoColor=white&label=PyPI)](https://pypi.org/project/velesdb/)
[![MCP registry](https://img.shields.io/badge/MCP_registry-io.github.cyberlife--coder%2Fvelesdb--memory-1f6feb?logo=modelcontextprotocol&logoColor=white)](https://registry.modelcontextprotocol.io)
[![License](https://img.shields.io/badge/license-VelesDB_Core_1.0_(source--available)-e8702a)](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE)

> **Portability**: ✅ the server and all of its tools work in any MCP client
> over stdio · ⚙️ sharing one store between several clients at once requires
> the HTTP transport (`--features http`) · ⚠️ the `.mcpb` bundles are a
> one-click packaging format (Claude Desktop and registry-aware clients), not
> a transport, and are built stdio-only · ⚠️ the automatic
> [agent hooks](../../integrations/agent-hooks/README.md) are harness-specific,
> and tool-result replacement (`updatedToolOutput`) is Claude Code only.

## Objective

A coding agent forgets everything between sessions, and a vector store only
gives it back text that *looks like* the question. Neither can answer "why did
we do this?", because the answer is usually a fact that shares no words with
the question — the ticket behind the decision, the incident behind the
constant.

velesdb-memory gives an agent durable memory that never leaves the machine: it
remembers facts, recalls them semantically, **connects** them with typed links,
and walks those links to return the evidence trail behind an answer. It also
ships a deterministic context compiler that shrinks an agent's prompt under a
hard token budget with no model call at all.

## What you actually gain

Two problems cost you real money and real quality every day, and this fixes both.

**Your agent forgets.** Close the session, and everything it learned about your
codebase is gone. Tomorrow you explain it again. velesdb-memory keeps those
facts on your disk and hands them back at the start of the next session.

**Every turn re-sends the whole conversation.** That is what you are billed for,
and a context stuffed with repeated logs is also a context where the model pays
less attention to what matters. The compiler shrinks that payload before it is
sent — deterministically, with no AI call of its own.

| What improves | Measured | How it was measured |
|---|---|---|
| Context sent to the model | **82.5 % smaller** on a 12-turn coding session (80–87 % per turn as it grows) | [committed corpus, real cl100k tokenizer](examples/context_savings) — recompiled twice per run, byte-identical |
| Your actual bill | **10.9 % to 21.9 %** saved on the same session, A/B, real billing | [billed campaign](../../examples/real-session-benchmark#billed-campaign-results-2026-07-19-cli-runner-claude-sonnet-5) |
| Cost of storing a memory | **zero AI calls** — nothing is sent anywhere | the write path never calls a model |
| A 55 KB build log entering context | **767 characters**, error and file:line kept | the [`PostToolUse` hook](../../integrations/agent-hooks/README.md) |

The honest part: those percentages come from *our* corpus on *our* sessions.
The spread is published as prominently as the best figure, and every number
above is pinned to its source by a [contract the CI
enforces](../../docs/reference/promise-contract.json) — if a figure drifts from
what the code produces, the build goes red.

## How it works, in four steps

Nothing here needs an AI provider. Everything runs on your machine.

**1. It stores facts, not transcripts.** You (or your agent) call `remember`
with one fact: *"the API port is 6333 because 3000 collided with the web UI"*.
It is written to a local file store. No model call, no network.

**2. It finds them by meaning, not keywords.** `recall` matches on sense, so
asking about *"which port did we settle on"* finds that fact even though the
words differ. This uses a local embedding model of your choosing.

**3. It connects them, which is the part that matters.** Facts are linked to
the topics they mention. `why` starts from the best match and then *walks those
links*, so it returns the answer **plus the facts that explain it** — including
ones sharing no words with your question. A plain search cannot do that.

> Those links have to exist. If you only ever call `remember`, the graph stays
> flat and `why` behaves like a search. Point `remember_extracted` at a
> paragraph and it splits it into facts and wires the links for you.

**4. It compresses what is too big, at the right moment.** Four [agent
hooks](../../integrations/agent-hooks/README.md) fire automatically in Claude
Code: two remind it to save and reload its state around a session, one does the
same before a compaction, and `PostToolUse` is the one that *replaces* an
oversized tool result with a compiled view — so the payload
never enters the conversation at all. Nothing is deleted: the untouched
original is written to a file and its path is quoted in the replacement, so the
agent can read the full thing whenever the summary is not enough.

## Use cases

- A coding agent that must still know, three weeks later, *why* a timeout is
  set to 8 s — and can show the PR and the incident it came from.
- Regulated or air-gapped work where context cannot transit a third-party LLM
  API, and "show why it recalled that" has to be answerable.
- Long agent sessions that hit the context window: compile the prompt instead
  of summarizing and restarting.

## Prerequisites

| Requirement | Minimum version | Note |
|---|---|---|
| Rust | 1.90 | Only to install or build. The binary itself has no runtime dependency. |
| An MCP client | — | Claude Code, Claude Desktop, Codex CLI, Cursor, Cline, Zed, opencode, Windsurf, Devin CLI. |
| Ollama | any | **Optional** — only for real semantic recall (`--features ollama`). The default embedder is offline and dependency-free. |
| Node.js | any LTS | **Optional** — only for the Claude Desktop stdio→HTTPS bridge. |

## Installation

```bash
cargo install velesdb-memory
```

No Rust toolchain? velesdb-memory is on the
[MCP registry](https://registry.modelcontextprotocol.io) as
`io.github.cyberlife-coder/velesdb-memory`, with prebuilt `.mcpb` bundles on
each [release](https://github.com/cyberlife-coder/VelesDB/releases). Full
options: [MCP server setup](../../docs/guides/MCP_SERVER_SETUP.md#install).

## First success in 60 seconds

Wire it into Claude Code:

```bash
claude mcp add velesdb-memory \
  --env VELESDB_MEMORY_PATH="$HOME/.velesdb-memory" \
  -- ~/.cargo/bin/velesdb-memory
```

Then ask your agent to store a decision and recall it. These are the MCP calls
it makes, and exactly what comes back:

```jsonc
remember { "fact": "we chose parking_lot to avoid lock poisoning",
           "metadata": { "project": "checkout" } }
→ { "id": 9876543210, "id_str": "9876543210" }

recall { "query": "locking strategy", "limit": 5 }
→ { "memories": [ { "id": 9876543210, "id_str": "9876543210",
                    "score": 0.59,
                    "content": "we chose parking_lot to avoid lock poisoning",
                    "metadata": { "project": "checkout", "_veles_date": 20260725 } } ] }
```

A non-empty `memories` array means the server is wired and the store is
writable. (`_veles_date` is stamped automatically — see
[automatic dating](../../docs/reference/MCP_TOOLS.md#automatic-dating-_veles_date).)

Every other client — Cursor, Zed, Codex CLI, Claude Desktop, Windsurf, Devin
CLI — is one config block away in
[MCP server setup](../../docs/guides/MCP_SERVER_SETUP.md#configure-your-client-stdio).

## See the wedge (offline, one command)

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

A vector search ranks by resemblance, so it is blind to the ticket. `why()`
follows the typed links and reaches it. That gap is the product.

| Runnable demo | What it shows |
|---|---|
| [`why_across_sessions.py`](../../examples/agent_memory/why_across_sessions.py) | the reason survives a process restart — recall of the top 5 of 16 memories stays blind, `why()` reaches it |
| [`why_magic_constant.py`](../../examples/agent_memory/why_magic_constant.py) | *why* a magic constant has its value — a business reason sharing no words with the code |
| [`memory_builds_its_own_graph.py`](../../examples/agent_memory/memory_builds_its_own_graph.py) | paste raw prose → a local model auto-wires the graph (no `relate()`), `why()` walks it to the root cause |
| [`why_magic_constant.mjs`](../velesdb-node/examples/why_magic_constant.mjs) | the same engine and wedge in the Node binding |

> **Not a weak-embedder trick.** In each retrieval demo, recall stays blind to
> the reason **even under a real semantic embedder** (`ollama` / `all-minilm`),
> not just the offline `hash` default.

### The graph's contribution, isolated

`cargo run --release -p velesdb-memory --example bench_multihop` runs 24
`decision → PR → problem` chains with the same embedder throughout and only the
graph toggled. Each question (`"why did we adopt <tech>"`) has a 1-hop answer
(the decision, which shares words) and a 2-hop answer (the original problem,
which shares none):

| embedder | direct recall | multi-hop, vector-only | multi-hop, **vector + graph** |
|---|:-:|:-:|:-:|
| `hash` (deterministic) | 100% | 0% | **100%** |
| real model (Ollama `all-minilm`) | 100% | 33% | **100%** |

The **direct** control confirms the vector engine is healthy — it aces
look-alike retrieval. On **multi-hop**, a real semantic embedder still recovers
only a third of the answers; the graph recovers all of them, **+67 pp** with a
real model. Run that arm yourself:

```bash
cargo build --release -p velesdb-memory --features ollama && ollama pull all-minilm
VELESDB_MEMORY_EMBEDDER=ollama \
  cargo run --release -p velesdb-memory --features ollama --example bench_multihop
```

`bench_multihop` measures the *engine's* contribution on controlled data with
the graph pre-wired, so the numbers reflect retrieval, not an LLM. The
end-to-end *extraction* comparison on the real
[LoCoMo](https://github.com/snap-research/locomo) dataset lives in
[`examples/locomo/`](examples/locomo/README.md).

## What the server exposes

18 MCP tools in the default build, in three families:

| Family | Tools |
|---|---|
| Durable memory | `remember`, `recall`, `recall_where`, `recall_fused`, `relate`, `unrelate`, `forget`, `entity`, `why`, `feedback`, `remember_extracted` |
| Context compiler | `compile_context`, `compile_transcript`, `explain_compilation`, `retrieve_context_source`, `context_savings`, `suggest_budget` |
| Session resumption | `save_working_context`, `load_working_context`, `list_working_contexts` |

Parameters, returns, and error codes for every one:
**[MCP tool reference](../../docs/reference/MCP_TOOLS.md)**.

By design the server exposes **memory semantics only** — never raw database
capabilities (`query`, `create_collection`, `upsert`, `traverse`).

## Where to go next

| Guide | What it covers |
|---|---|
| [MCP server setup](../../docs/guides/MCP_SERVER_SETUP.md) | every client config, the shared HTTPS daemon, the local CA, Windows, embedding and extraction backends |
| [MCP tool reference](../../docs/reference/MCP_TOOLS.md) | one section per tool: parameters, returns, limits, error model |
| [Context compiler](../../docs/guides/CONTEXT_COMPILER.md) | budgets, preservation rules, `risk`, retrieval handles, media, `path` ingestion, transcripts, the `compile-stdin` CLI and the `PostToolUse` hook |
| [Agent Memory SDK](../../docs/guides/AGENT_MEMORY.md) | the *other* path: the embedded, language-native `AgentMemory` API |
| [`BENCHMARK.md`](BENCHMARK.md) | every published retrieval number, its method, and how to reproduce it |
| [`POSITIONING.md`](POSITIONING.md) | honest comparison against Mem0 and Zep/Graphiti, and where local-first is a hard requirement |
| [`CHANGELOG.md`](CHANGELOG.md) | what changed in each release |

Measured, generation-free retrieval lift against a pure-vector baseline on
public datasets — HotpotQA **+7.2 pp**, 2WikiMultiHopQA **+2.1 pp** overall,
TimeQA **+9.7 pp**, tri-engine **+29 pp** — is tabulated with its full method
in [`BENCHMARK.md`](BENCHMARK.md).

## Compatibility

| Environment | Status | Note |
|---|---|---|
| Any MCP client | Supported | stdio by default; streamable-HTTP with `--features http`. |
| Claude Code | Supported | `claude mcp add`, stdio or `--transport http`. Also the only harness with the `PostToolUse` replacing hook. |
| Claude Desktop | Supported, with a caveat | Its config file accepts stdio only; for the shared daemon the installers wire an `mcp-remote` stdio→HTTPS bridge, which needs Node.js. |
| Codex CLI | Supported | `codex mcp add`, or a `[mcp_servers.*]` table. Two lifecycle hooks ship: `SessionStart` (resume the rolling working context, and compile what a compaction is about to lose) and `Stop` (save it before finishing). `PreCompact`/`PostCompact` are not wired — they have no documented output channel that reaches the model. |
| Windsurf | Supported | stdio (`mcp_config.json`) or `serverUrl` against the daemon. One advisory `pre_user_prompt` hook is wired; it is shown to the user, not injected into the model context. |

Other verified clients: Cursor, Cline, Zed, opencode, Devin CLI.

## Known limits

- **Memory semantics only.** No `query`, `create_collection`, `upsert`, or
  `traverse` — deliberate, and enforced by the tool surface.
- **One process per store.** The store takes a single-writer `flock`, so two
  stdio clients against the same `VELESDB_MEMORY_PATH` fail with
  `Storage(DatabaseLocked)`. Run the [HTTP daemon](../../docs/guides/MCP_SERVER_SETUP.md#http-transport-multi-client)
  to share one memory across clients.
- **The HTTP transport has no authentication.** It binds loopback-only by
  default; a non-loopback bind is refused unless you explicitly allow it, and
  is only safe behind an authenticating reverse proxy.
- **A store is fixed to one embedder.** The embedding dimension is probed from
  the model, so do not switch embedders on an existing store.
- **Bring-your-own-links by default.** The graph is built by `relate` and
  `links`; automatic extraction needs `--features extract` plus a local model.
- **`path` ingestion is off unless allowlisted** via
  `VELESDB_MEMORY_INGEST_ROOTS`.
- **Binding parity is incomplete.** `compile_transcript` is MCP-only; the Node
  binding has no `context_savings` / `explain_compilation`; the published PyPI
  wheel predates the compiler entirely — see the
  [surface matrix](../../docs/guides/CONTEXT_COMPILER.md#where-the-compiler-is-available).
- **No selective source purge.** Compiled sources are kept permanently by
  default; you can set a TTL going forward, or delete the whole store.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `Storage(DatabaseLocked)` | Two processes opened the same store — usually a second client, or a stray stdio process next to the daemon. | Run one `--http` daemon and point every client at it, or give the second client its own `VELESDB_MEMORY_PATH`. |
| The server never starts from a JSON/TOML config | `~` is not expanded: those configs spawn the binary without a shell. | Use an absolute path, e.g. `/home/you/.cargo/bin/velesdb-memory`. |
| `extraction backend not configured` from `remember_extracted` | Built without `--features extract`, or `VELESDB_MEMORY_EXTRACTOR` is unset. | See [auto-extraction](../../docs/guides/MCP_SERVER_SETUP.md#auto-extraction-backend-opt-in). |
| `IngestDisabled` on a `path` fragment | `VELESDB_MEMORY_INGEST_ROOTS` is unset or empty — path ingestion is off by default. | Start the server with an allowlist of absolute directories. |
| `relate` / `forget` reports a missing id from a JS client | Ids exceed 2^53 and lose precision as JSON numbers. | Relay the `id_str` field, or set `"policy": {"ids_as_strings": true}` on compiler calls. |

## License

The distributed binary embeds `velesdb-core` and is governed by the **VelesDB
Core License 1.0** (source-available): a derivative of the Elastic License 2.0,
not an OSI-approved license. The wrapper source in this crate is intentionally
readable and forkable.

- **Can you use it at work, or in a commercial product?** Yes. Running the
  server locally, or embedding the library inside your own application where
  *your* users only ever receive results, is the license's expressly-permitted
  **embedded, local-first use**.
- **What is forbidden?** Re-hosting VelesDB as a multi-tenant *service* where
  third parties drive the database. This server makes that impossible by
  design: memory semantics only, never raw `query` / `create_collection` /
  `upsert` / `traverse`.
- **What do you owe when redistributing?** Keep the LICENSE file and copyright
  notices, and add a [velesdb.com](https://velesdb.com) attribution in any
  public app that ships the binary. Internal, dev, and test use need no
  attribution.

Full terms and the canonical FAQ:
[LICENSE](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE).
Questions: contact@wiscale.fr.

---

`velesdb-memory v0.11.2` · Last updated: 2026-07-25 · [Report a docs error](https://github.com/cyberlife-coder/VelesDB/issues)
