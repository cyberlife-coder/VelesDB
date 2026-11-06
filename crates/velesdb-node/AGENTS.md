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
