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
| `agent_loop.py` | Python | Full agent loop: semantic + episodic + procedural, plus a TTL + snapshot cycle. Doubles as a smoke test (prints a trace and exits 0). | `python agent_loop.py` |
| `snapshot_ttl.rs` | Rust | `velesdb-core` public API: namespaced TTL, `auto_expire`, snapshot save/load round-trip. | `cargo run --bin snapshot_ttl` |
| `agent_memory.ts` | TypeScript | `db.agentMemory()` SDK facade: `storeFact` / `recordEvent` / `learnProcedure` and their recall counterparts. Needs a running `velesdb-server`. | `npx tsx agent_memory.ts` |

## Python (`agent_loop.py`) — start here

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
