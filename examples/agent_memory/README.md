# Agent Memory Examples

Runnable examples for VelesDB's **Agent Memory** — the unified memory layer for
AI agents, combining three subsystems on top of the vector + graph engine:

| Subsystem | What it holds | Core ops |
|-----------|---------------|----------|
| **Semantic** | Long-term knowledge facts | `store` / `query` |
| **Episodic** | The timeline of what happened | `record` / `recent` / `recall_similar` |
| **Procedural** | Learned skills with confidence | `learn` / `recall` / `reinforce` |

Operationally, the layer adds **namespaced TTL** (keyed by memory kind, so a
semantic id never cross-expires an episodic id that shares the same integer),
`auto_expire`, and **versioned snapshots** with rollback.

Every example uses a **deterministic, network-free fake embedder** (a tiny
hash-into-buckets function), so they are fully reproducible and need no API key,
no model download, and no internet.

## Files

| File | Language | What it shows | How to run |
|------|----------|---------------|------------|
| `wedge_quickstart.py` | Python | **The differentiator:** `MemoryService.why()` reaches a 2-hop memory that plain `recall()` misses. Start here. | `python wedge_quickstart.py` |
| `why_across_sessions.py` | Python | A brand-new session reopens the same on-disk memory; `why()` walks typed links to the *reason* plain recall is blind to, across the session boundary. | `python why_across_sessions.py` |
| `why_magic_constant.py` | Python | Why is that timeout 7 seconds? `why()` walks from the config value to the business reason behind it before you "clean it up". | `python why_magic_constant.py` |
| `memory_builds_its_own_graph.py` | Python | `remember_extracted()` reads raw prose with a local LLM and auto-wires the fact↔topic graph — no `relate()` calls. Needs Ollama. | `python memory_builds_its_own_graph.py` |
| `agent_loop.py` | Python | Full agent loop: semantic + episodic + procedural, plus a TTL + snapshot cycle. Doubles as a smoke test (prints a trace and exits 0). | `python agent_loop.py` |
| `snapshot_ttl.rs` | Rust | `velesdb-core` public API: namespaced TTL, `auto_expire`, snapshot save/load round-trip. | `cargo run --bin snapshot_ttl` |
| `agent_memory.ts` | TypeScript | `db.agentMemory()` SDK facade: `storeFact` / `recordEvent` / `learnProcedure` and their recall counterparts. Needs a running `velesdb-server`. | `npx tsx agent_memory.ts` |

## The wedge — `why()` (start here)

`wedge_quickstart.py` uses the high-level `MemoryService` (`remember` / `recall` /
`relate` / `forget` / `why`) and shows the one thing pure vector search can't do:

```bash
cd crates/velesdb-python && maturin develop && cd -   # or: pip install velesdb
python examples/agent_memory/wedge_quickstart.py
```

```text
recall('why did we choose parking_lot')   [vector similarity only]
   0.28  we chose parking_lot to avoid lock poisoning after a panic
   0.16  PR #42 swaps the std Mutex for parking_lot
   └─ EPIC-317 is nowhere here: it shares no words with the question.

why('why did we choose parking_lot')      [vector seed + graph traversal]
   hop 0  we chose parking_lot ...
   hop 1  PR #42 ...
   hop 2  EPIC-317: intermittent CI hang under load
   └─ the graph reached the very ticket the decision fixed.
```

`recall` ranks by resemblance, so the ticket (which shares no words with the
question) is invisible to it. `why` seeds on the closest memory and walks the
typed links to reach it. That connected context is the differentiator.

## Python (`agent_loop.py`) — the primitive layer

Self-contained; it is also the smoke test for this directory.

```bash
# Install the SDK (from source or PyPI)
cd crates/velesdb-python && maturin develop && cd -
# or: pip install velesdb

python examples/agent_memory/agent_loop.py
```

It stores facts, recalls the closest to a question, records a turn timeline,
recalls episodes by similarity, learns a skill and reinforces it (watching
confidence rise), then runs a TTL expiry and a snapshot rollback — asserting
each step along the way.

## Rust (`snapshot_ttl.rs`)

```bash
cd examples/agent_memory
cargo run --bin snapshot_ttl
```

Builds against `velesdb-core` directly. `main()` returns `Result` and propagates
errors with `?` (no `unwrap` / `expect`).

## TypeScript (`agent_memory.ts`)

The TypeScript SDK talks to a running server, so start one first:

```bash
# Terminal 1
velesdb-server --data-dir ./data

# Terminal 2
npm install @wiscale/velesdb-sdk
npx tsx examples/agent_memory/agent_memory.ts
```

## License

MIT License
