# AGENTS.md — velesdb-cli

The `velesdb` command: an interactive VelesQL REPL and offline admin CLI. Root [`AGENTS.md`](../../AGENTS.md) is the contract; this file holds what only applies inside this crate.

- **CLI surface is user-facing contract:** REPL commands and flags are documented in the crate README; a new flag needs a README row and a clap test, not just a new subcommand.
- **Offline agent mode** (bounded ReAct over whitelisted VelesQL tools) lives here: the tool allowlist is validated before any VelesQL is built — an unvalidated identifier must never reach the parser.
- **Tests:** `cargo test -p velesdb-cli -p velesdb-mobile --features gpu,update-check,serve -- --test-threads=1` (CI leg `cli-mobile`). The REPL shares no filesystem state with core tests, but keep `--test-threads=1` anyway — the contract is workspace-wide.
- **No premium references**, no `.unwrap()` in non-test code — root AGENTS.md constraints apply unchanged.
- Docs: crate [README.md](README.md); VelesQL reference: `docs/reference/`.
