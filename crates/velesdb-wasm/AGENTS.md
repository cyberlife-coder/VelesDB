# AGENTS.md — velesdb-wasm

Browser wasm target. Root [`AGENTS.md`](../../AGENTS.md) is the contract.

- **No `persistence` feature** — anything persistence-gated is absent here;
  do not cfg it back in.
- **No std clock / filesystem** — `SystemTime::now()`, `Instant`, `std::fs`
  abort or are absent on `wasm32-unknown-unknown`; cfg-gate or use the
  backend seam.
- **The functional gate is `wasm-pack test --node crates/velesdb-wasm`** — a
  `cargo check` does not catch std APIs that abort at runtime.
- Query grammar lives in `sdks/typescript/src/backends/wasm-search.ts` —
  regex tails must stay ambiguity-free (polynomial-ReDoS audited by CodeQL;
  optional groups anchor on a literal).
