# AGENTS.md — TypeScript SDK

Client surface over the wasm/native backends. Root [`AGENTS.md`](../../AGENTS.md)
is the contract.

- **Build:** `npm run build` (tsup → CJS+ESM+DTS). Build-tool bumps verify by
  rebuilding and running the suite, never by diff alone.
- **Tests:** `npx vitest run` — the suite is the grammar/parity contract
  (`error-code-parity` pins VelesQL error codes; backend tests pin supported
  query shapes).
- **Regexes in `src/backends/wasm-search.ts` are CodeQL-audited** for
  polynomial ReDoS: an optional tail must anchor on a literal
  (`(?:;\s*)?$`, never `;?` between two `\s*`).
- The SDK is a client of the engine — no engine behavior may be re-implemented
  here beyond the pure-NEAR grammar shim the wasm backend cannot evaluate.
