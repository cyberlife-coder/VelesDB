---
name: core-parity-audit
description: "Re-run the VelesDB core-vs-ecosystem gap analysis (code + documentation). Use when asked to audit API parity, check that every crate/SDK/integration/doc has caught up to velesdb-core, verify the core↔children architecture is clean, or produce a parity matrix. velesdb-core is the single source of truth; the VelesDB Core License boundary is enforced in every recommendation."
trigger: /core-parity-audit
---

# /core-parity-audit

Audit whether every component that depends on `velesdb-core` has kept up with it — in **code and documentation** — and whether the **core↔children architecture is clean**. Produces two artifacts: a capability-level **parity matrix** and an **architecture-cleanliness verdict**.

## Two non-negotiable rules

1. **`velesdb-core` is the single source of truth — always.** Children (server, cli, python, wasm, mobile, migrate, tauri, ts-sdk, langchain, llamaindex, haystack, integrations/common, docs) may legitimately expose **more** surface than core (idiomatic helpers, ops endpoints, framework adapters) — that is healthy enrichment, **not** a parity defect. Judge two things *separately*: (a) does the child enrich core? (good) vs (b) is the boundary clean? (the real question).

2. **Respect the VelesDB Core License boundary.** See [references/license-boundary.md](references/license-boundary.md). In short:
   - `velesdb-core`, all Rust crates, and the TS SDK are under **VelesDB Core License 1.0** (source-available, Elastic-2.0-based). Its *protected capabilities* are query, administration, indexing, ingestion, storage, graph traversal, knowledge-graph, and columnar filtering.
   - `integrations/{langchain,llamaindex,haystack,common}` are **MIT**.
   - **A reimplementation or copy of core's protected logic inside an MIT integration is BOTH an architecture smell AND a license-leakage risk.** Flag it at higher severity. The only valid remediation is **"expose it via the binding and have the MIT layer *call* it"** — never "copy core logic into the integration," never relicense Core code as MIT or vice-versa.
   - Never recommend stripping `VelesDB®` notices or paid-feature gating. Follow the project **no-AI-attribution** rule in any report/PR/commit you generate.

## Usage

```
/core-parity-audit                 # full audit: parity matrix + architecture verdict
/core-parity-audit --matrix-only   # capability parity matrix only (skip architecture probes)
/core-parity-audit --arch-only     # architecture-cleanliness verdict only (skip the matrix)
/core-parity-audit --component <k> # focus on one component (e.g. wasm, langchain)
```

This audit is read-only. It does not modify source; it writes report artifacts to a scratch dir (default `/tmp`) and summarizes in chat. Use the [Workflow tool](#orchestration) — this is a fan-out, multi-agent job (ultracode-class).

## Components that depend on velesdb-core (the work-list)

Re-derive this from the workspace each run (`Cargo.toml` members + `sdks/` + `integrations/` + `docs/`), but the stable set is:

| key | path | wraps core via | license |
|-----|------|----------------|---------|
| server | `crates/velesdb-server` | direct (Axum REST) | Core 1.0 |
| cli | `crates/velesdb-cli` | direct (embedded REPL) | Core 1.0 |
| python | `crates/velesdb-python` | direct (PyO3 + `.pyi` stub) | Core 1.0 |
| wasm | `crates/velesdb-wasm` | direct, **no `persistence` feature** | Core 1.0 |
| mobile | `crates/velesdb-mobile` | direct (UniFFI) | Core 1.0 |
| migrate | `crates/velesdb-migrate` | direct (ETL tool) | Core 1.0 |
| tauri | `crates/tauri-plugin-velesdb` | direct (commands) | Core 1.0 |
| ts-sdk | `sdks/typescript` | REST + WASM backends | Core 1.0 |
| langchain | `integrations/langchain` | velesdb python binding + common | **MIT** |
| llamaindex | `integrations/llamaindex` | velesdb python binding + common | **MIT** |
| haystack | `integrations/haystack` | velesdb python binding | **MIT** |
| common | `integrations/common` | shared base for the 3 integrations | **MIT** |
| docs | `docs/` + `README.md` | documents the public surface | Core 1.0 |

Per-component inspection hints live in [references/component-inventory.md](references/component-inventory.md).

## Methodology (6 phases)

Author the Workflow scripts fresh each run (the codebase evolves; do **not** hard-code last run's capability list — re-extract it). The pattern that worked:

### Phase 0 — Discover (inline)
Map the work-list: `Cargo.toml` members, `sdks/`, `integrations/`, `docs/`. Capture `velesdb-core`'s public re-exports from `crates/velesdb-core/src/lib.rs` and the public methods of `Database`, `VectorCollection`, `GraphCollection`, `MetadataCollection`, `AnyCollection`, and the `agent::` module. These seed Phase 1.

### Phase 1 — Build the canonical core catalog (barrier)
Fan out ~3 agents by domain (management/config/enums · data plane · query/graph/memory) to extract `velesdb-core`'s **public capabilities** — grouped, not every micro-method (e.g. all `search_with_*` = one "filtered search"). Each capability = `{id (domain-prefixed), group, name, core_symbols[], signature, notes}`. Merge + dedupe by id → **one canonical catalog**. This is the source-of-truth column. (Last run: ~81 capabilities across 20 domains; expect drift.)

### Phase 2 — Map each component (pipeline, one agent per component)
Inject the **same** catalog into every component agent. Each rates **every** capability `full | partial | absent | na` with `file:symbol`/endpoint/doc-section evidence, and lists `extra_surface` (capabilities the child exposes that are NOT in core — the enrichment). For `docs`: `full` = documented with usage, `absent` = undocumented public capability (a doc gap).

### Phase 3 — Adversarially verify gaps (pipeline stage 2)
Hand each component's `absent`/`partial` rows to a **skeptic** agent that tries to **refute** them — grep harder (helpers, aliases, `.pyi` stubs, base classes in `integrations/common`, parent protocols). The parity campaign closed many gaps; naïve greps re-report them. Reconcile mapper status with verifier overturns. (Last run the verifier correctly overturned Python `collection_diagnostics`/`update_guardrails`/`create_index`/`reorder_for_locality` — they *are* exposed.)

### Phase 4 — Architecture-cleanliness audit (barrier)
This answers the real question. Fan out:
- **1 dependency-direction agent**: `velesdb-core/Cargo.toml` must list **no** child; every child depends on core; no cycles. Any inversion = `concern`.
- **1 boundary agent per child**: does it *delegate* canonical logic to core or *reimplement* it? Is the reimplementation justified (WASM has no `persistence` feature → reimplements storage + a VelesQL executor) or a divergence smell? Are contracts (enums, wire codec, `VELES-###` error codes) sourced from core or re-derived (drift risk)? Is the extra surface composed from core primitives or does it bypass core to touch internals?
- **4 cross-cutting divergence probes**: (1) **fusion math** single-sourced vs reimplemented (core `fusion/` vs `velesdb_common.fusion` vs `wasm/fusion.rs` vs ts-sdk); (2) **string→u64 ID hashing** consistency (`velesdb_common.ids.stable_hash_id` vs haystack `_str_id_to_int` vs mobile vs migrate `stable_point_id` — different algorithms = interop divergence); (3) **VelesQL parser/executor sync** (WASM reimplements the executor; ECOSYSTEM_PARITY notes WASM conformance is **parser-only** — executor behaviour is NOT fixture-checked); (4) **distance + quantization kernel fidelity** (WASM/mobile recompute distance/quant — same results as core? recall@10 ≥ 0.95 fidelity?).

Each finding: `severity info|smell|concern`, `divergence_risk` bool, `file:line` evidence. **License lens:** a divergence/duplication in an MIT integration that copies core's *protected* logic is escalated.

### Phase 5 — Verify concerns + synthesize
Adversarially re-check every `concern` (and `smell` with `divergence_risk`) — many "concerns" are justified design (feature-gating, pure-client wire-building, idiomatic enrichment). Then synthesize:
- **Parity matrix** — run `scripts/gen_matrix.py` over the Phase 2/3 JSON results → cell-level markdown matrix + per-component scorecard + gap buckets (user-facing vs core-internal/ops). Exclude core-internal domains (observability counters, durability/WAL tuning, registry generations, column-store kernels) from the "should expose" denominator — they were never meant to cross the binding boundary.
- **Architecture verdict** — per-child cleanliness + the 4 probe verdicts + the dependency-direction result, each concern with its verification status and a **license-aware** remediation.

### Orchestration
Drive Phases 1–5 with the **Workflow tool** (`parallel` for the catalog barrier and the architecture fan-out; `pipeline` for component map→verify). Save each workflow's structured result, feed Phase-2/3 JSON to `scripts/gen_matrix.py`. If agents get rate-limited (it happens at ~25+ concurrent), re-run only the failed components in a small follow-up workflow reusing the existing catalog — do **not** resume-cache the failed nulls.

## Outputs

1. `PARITY_MATRIX.md` — capability × component matrix (`✅ full · ⚠️ partial · ❌ absent · · n/a`), scorecard, gap buckets.
2. Architecture verdict (in chat + optional `ARCHITECTURE_VERDICT.md`): is `velesdb-core` the uncontested source of truth? Is the boundary clean? Concerns ranked, each with license-aware remediation.
3. Reconcile against `docs/reference/ECOSYSTEM_PARITY.md` "Remaining Gaps" — flag anything genuinely new vs already-documented-by-design.

## Interpreting results

- `partial`/`na` are usually **by design** (WASM no-persistence, Haystack DocumentStore protocol bounds, agent-memory not over REST, CLI has no memory surface). Don't report these as "not caught up."
- The actionable signal is **`absent` in a user-facing domain** that is **not** already documented as by-design, **plus** any architecture `concern` confirmed in Phase 5.
- Enrichment (`extra_surface`) is the headline good news — list it, don't penalize it.
