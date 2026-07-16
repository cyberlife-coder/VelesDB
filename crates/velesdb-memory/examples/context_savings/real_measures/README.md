# real_measures — committed harnesses behind the cross-check figures

The parent README quotes real-tokenizer and latency figures; these scripts
are how they were produced, so every number stays reproducible.

| Script | What it measures | Prereqs |
|---|---|---|
| `measure.mjs` | Real cl100k tokens (gpt-tokenizer) of the benchmark corpus, raw vs compiled, per budget — verifies the output fits the budget in *real* tokens and that the Node stack is byte-deterministic | `cd crates/velesdb-node && npm ci && npm run build && npm install --no-save gpt-tokenizer`, then `node measure.mjs` |
| `stress.mjs` | Compile latency and peak RSS at the DoS caps (1024×1 KB, 10 MB, 64 MB, 1 MB repetitive log, 1024-duplicate avalanche) on the release addon | same build, then `node stress.mjs` |
| `mcp_e2e.py` | Drives the real MCP server over stdio JSON-RPC and exercises all four context tools end-to-end (also usable as a latency probe) | `cargo build --release -p velesdb-memory`, then `python3 mcp_e2e.py` from the repo root |

Token figures are machine-independent; latency and RSS vary with hardware
(reference figures in the parent README were measured on Apple Silicon,
release profiles).
