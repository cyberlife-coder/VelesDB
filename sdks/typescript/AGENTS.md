# AGENTS.md — TypeScript SDK

Client surface over the wasm/native backends. Root [`AGENTS.md`](../../AGENTS.md)
is the contract.

- **Build:** `npm run build` (tsup → CJS+ESM+DTS). Build-tool bumps verify by
  rebuilding and running the suite, never by diff alone.
- **Tests:** `npx vitest run` — the suite is the grammar/parity contract
  (`error-code-parity` pins VelesQL error codes; backend tests pin supported
  query shapes).
- **Docs & examples ship with the change.** Any change to the public surface
  (a `VelesDB` method, an options/result type, backend behavior) must update
  `README.md` in the same PR — every documented `db.X` must exist in
  `src/client.ts`, every documented option must exist in the types, and every
  code block must compile (extract-and-`tsc` a block if it is not covered by
  `tsconfig.examples.json`). `examples/*.ts` are type-checked in CI via
  `npm run typecheck` (`tsconfig.examples.json` includes `examples/**`), so
  keep them compiling and extend them when they illustrate the changed
  surface. Test fakes must reproduce the observed server wire shape, not a
  kinder one (#2282).
- **Regexes in `src/backends/wasm-search.ts` are CodeQL-audited** for
  polynomial ReDoS: an optional tail must anchor on a literal
  (`(?:;\s*)?$`, never `;?` between two `\s*`).
- The SDK is a client of the engine — no engine behavior may be re-implemented
  here beyond the pure-NEAR grammar shim the wasm backend cannot evaluate.
