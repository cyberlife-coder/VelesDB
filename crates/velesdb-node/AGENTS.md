# AGENTS.md — velesdb-node

N-API binding for Node. Root [`AGENTS.md`](../../AGENTS.md) is the contract.

- **Licence boundary (CI-asserted):** `velesdb-node` must never pull
  `velesdb-core` — `cargo tree` verifies it. Depend on `velesdb-memory` /
  `velesdb-wasm` only.
- **`MemoryService` surface is contract-tested:** adding a method requires
  updating the allowlist in `__test__/index.spec.mjs` plus a behavior test —
  CI fails on an unnamed addition.
- **Build profile:** `release-node` (and `release-mobile` for device builds)
  — both `panic = "unwind"`, required by the N-API link.
- 0.x crate: MINOR is the breaking component (see `velesdb-memory/AGENTS.md`).
- **Documentation & examples are part of the crate change — update them in
  the same commit/PR, never in a follow-up.** Any change to this crate's
  surface or behavior must ship with:
  - `README.md` — the 60-second example must actually run against the
    current API (it is the first thing a user copies);
  - `docs/guides/NODE_ADDON.md` and the `docs/reference/ECOSYSTEM_PARITY.md`
    bullet for the Node binding;
  - `examples/` (e.g. `why_magic_constant.mjs`) and the bundled `skills/`
    (re-sync with `python3 scripts/sync-skills.py --bundle`; staleness is
    pinned by `__test__/skills-sync.spec.mjs`);
  - the contract tests: the method allowlist + behavior tests in
    `__test__/index.spec.mjs` and `__test__/remember_extracted.integration.spec.mjs`.
  - `index.js` / `index.d.ts` / `*.node` are gitignored napi-build artifacts
    — never hand-edit them; regenerate with `npx napi build --platform` (the
    `*.d.ts` contract test in `__test__/` guards staleness).
