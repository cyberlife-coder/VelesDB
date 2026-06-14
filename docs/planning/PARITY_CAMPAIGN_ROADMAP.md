# VelesDB Parity Campaign — Executable Roadmap

> **Directive**: close **every community-scope** gap, defer nothing, subject to
> (a) the VelesDB Core License boundary and (b) genuine technical feasibility.
> Generated 2026-06-13 from a re-control audit (verification workflow) +
> per-item feasibility design workflow against `develop@7a6153aa`.

## Re-control verdict

The 4-PR API-parity campaign (#1096/#1098/#1099/#1100) was **verified real** —
23 Python methods, all REST routes, mobile/tauri/TS surfaces confirmed by
`file:line` with **zero false positives**. The old informal matrix is obsolete.
**One functional regression escaped** the docs: the CLI silently dropped
`@collection` cross-collection MATCH enrichment → **fixed in PR-A**.

## License boundary (PASS gate)

🔒 **Premium — never touched here**: `WalBatchConfig` concurrent-writer
same-collection; RBAC; SSO; Audit Logging; Multi-Tenancy; Encryption at Rest;
Snapshots; GPU acceleration. Key nuance: **single-writer streaming ingestion is
community** (allowed); only concurrent-writer-same-collection is premium.
Observer PRs ship the veto *hook* with a default-allow `DefaultObserver` and **no
policy engine** (RBAC stays premium).

## Excluded (correctly, not dodged)

- `6-observer-wasm` — architectural N/A (WASM never instantiates `core::Database`).
- `6-observer-typescript-rest` — needs an SSE/WS server feature (future RFC), not a parity gap.
- `6-observer-mobile` — feasible via UniFFI callback interface but ~40 commits, low demand → later/premium.
- `6-raw-bulk-mobile` — follow-up after PR-J proves the wire format.
- `I4` cross-collection `similarity()` in WHERE — undesigned new feature, recall-gated; separate epic.

## PR roadmap (dependency-ordered)

| PR | Title | Items | Risk / gate | Depends |
|----|-------|-------|-------------|---------|
| **A** ✅ | fix(cli): route `@collection` MATCH through `Database::execute_query` | C0 | done, green | — |
| **B** ✅ | feat(core): persist AutoReindexConfig + StreamingConfig; schema v1→v2 | W2, STREAM-7 | backward-compat test | — |
| ~~**C**~~ DROPPED | ~~feat(core): wire `deferred_indexing`/`async_index_builder` into VelesConfig TOML~~ | W3 | — | — |
| **D** ✅ | refactor(search): SearchConfig SSOT via `effective_ef_search` | W4 | **RECALL Gate 1**, solo | — |
| **E** ✅ | feat(core): enforce LimitsConfig 3 fields, <1% bench | W5 | **benchmark**, after D (shares vector.rs) | D |
| **F** ✅ | feat(streaming): StreamingConfig + delta CAS + Tauri/REST enable_streaming | STREAM-1/4/5/9 | standard | — |
| **G** ✅ | feat(python): enable_streaming via pyo3-asyncio | STREAM-2 | pyo3-asyncio | F |
| **H** ✅ | feat(ts,mobile): enableStreaming SDK + mobile wrapper | STREAM-6/3 | TS strict | F/G |
| **I** | feat(streaming): WAL bypass via WriteMode | STREAM-8 | **crash-recovery parity**, optional | G |
| **J** ✅ | feat(rest,ts): binary wire-format `upsert_bulk_from_raw` | raw-bulk rest+ts | document contract | — |
| **K** ✅ | feat(wasm,cli): raw-bulk typed-array + CLI ingest | raw-bulk wasm+cli | wasm no-default-features | — |
| **L** ✅ | feat(mobile): diagnostics + multi_query_search_ids; feat(cli): `.diagnostics` | M1, M2, C1 | standard (C1 after A) | A (CLI) |
| **M** ✅ | feat(wasm): sparse_search + validate dedup | M3 | wasm check, after K | K |
| **N** ✅ | feat(integrations): Haystack fusion + named-sparse creation (LC/LI/Haystack) | I1, I2 | py integration tests | — |
| **O** ✅ | feat(wasm,integrations): `@collection` MATCH propagation | I3 | net-new WASM wiring | — |
| **P** ✅ | feat(observer): Python PyO3 lifecycle callbacks + server e2e | observer python+server | done 2026-06-14; **GIL safety**, default-allow | — |
| **Q** ✅ | feat(observer): Tauri event-based lifecycle notifications | observer tauri | no policy engine | — |

## C — DROPPED (not a real parity gap)

Re-scoped 2026-06-14 against the code: `deferred_indexing` / `async_index_builder`
are already per-collection `CollectionConfig` fields, configurable via
`apply_advanced_config` / create-with-options. There is **no checked-in gap doc**
backing C (no `CORE_WIRING_DEBT.md`; `ECOSYSTEM_PARITY.md` doesn't list it — it
came only from the generated roadmap). Crucially, the global `VelesConfig` (TOML)
does **not** drive per-collection creation today (only `limits` are read;
`hnsw`/`search` are already inert there), so adding these fields to `VelesConfig`
would be either cosmetic/inert or require a new "VelesConfig as per-collection
defaults" architecture — beyond a parity fix. Dropped with maintainer sign-off.

## I — DEFERRED (no real parity gap, gate logically impossible)

Re-scoped 2026-06-14 against the code: STREAM-8 "WAL bypass via WriteMode" has no
closeable gap. The `WriteMode{Api,Streaming}` enum is dead-code provenance —
`pub(crate)` and unused (`collection/streaming/ingester.rs`, doc-commented
"currently unused"). The real durability bypass already exists and is tested:
`storage::DurabilityMode::None` (`storage/log_payload.rs`), threaded through
`LogPayloadStorage::new_with_durability` — it is simply not surfaced as a
user-facing knob by design (a durability downgrade is a footgun). There is no
checked-in gap doc backing I, and its "crash-recovery parity" gate is logically
impossible: a mode whose contract is "may lose the WAL tail on crash" cannot be
asserted to match a durable-recovery baseline. Deferred with maintainer sign-off.

## Execution batches (worktree-isolated)

- **Batch 0 (blocking)**: PR-A ✅ done.
- **Batch 1 (parallel, disjoint files)**: B, F, J, L, N, P, Q.
- **Batch 2 (after Batch 1)**: C←B, G←F, K, O.
- **Batch 3 (serialized — recall/file collisions)**: D (solo, recall) → E (after D); H←F/G; M←K.
- **Batch 4 (optional/hard, last)**: I (only with crash-recovery proof).

### Must-serialize collisions
- `collection/search/vector.rs` → D then E
- `collection/collection_config.rs` + `lifecycle.rs` → B then C
- `velesdb-wasm/src/{lib,vector_store}.rs` → K then M
- streaming config type → F → G → H

## Documentation actions (from re-control drift)

1. `ECOSYSTEM_PARITY.md:56` — CLI `@collection` claim true once PR-A lands.
2. `ECOSYSTEM_PARITY.md:220` action item 1 — relabel (CLI has no HTTP layer; REST error-shape belongs to server).
3. `CORE_WIRING_DEBT.md:298` (6.11) — soften "verified no-op": WASM reimplements `validate_dimension` + doesn't enforce `validate_collection_name`.
4. `CORE_WIRING_DEBT.md` §6 rows 6.6/6.9/6.10 — note surfaces still missing (mobile/tauri) before PR-L closes them.
