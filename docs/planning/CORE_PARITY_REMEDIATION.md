# Core ↔ Children Parity & Architecture Remediation Plan

Status: **in progress** — opened 2026-06-14 after the core-vs-ecosystem gap + architecture audit.
Re-runnable any time via the `/core-parity-audit` skill (`.claude/skills/core-parity-audit/`).

## Context (the audit verdict)

`velesdb-core` is the single source of truth, and the architecture is **clean**: dependency
direction is a perfect star (core out-degree 0, no inversions, no cycles); children legitimately
add idiomatic surface. The only real signal is a handful of spots where a **child forks canonical
logic instead of calling core** — none questions core. User-facing feature parity is caught up;
the residual gaps are core-internal/ops plumbing (by design) or already tracked in
`docs/reference/ECOSYSTEM_PARITY.md` "Remaining Gaps".

## TODOs

| # | Item | Rationale | Effort | Risk / guard-rail | License lens |
|---|------|-----------|--------|-------------------|--------------|
| **T1** | WASM `crates/velesdb-wasm/src/fusion.rs` → call `velesdb_core::FusionStrategy::fuse` instead of re-implementing all 5 strategies | Removes fusion-ranking divergence (WASM computes locally, no server) | M | **Touches ranking → QUALITY_BAR Gate 1 recall@10 ≥ 0.95.** Add WASM-vs-core fusion equivalence test. Guarded by T2. | Core-licensed, pure architecture |
| **T2** | Executor-level conformance fixtures for WASM (+CLI): compare result rows/counts/ordering, not just parse-ok (`conformance/velesql_parser_cases.json` is parser-only today) | Auto-catches future JOIN/agg/setops divergence; safety net for T1 | M | test-only; **do FIRST** | Core |
| **T3** | ID-hash single-sourcing: Haystack imports `velesdb_common.ids.stable_hash_id` (delete `_str_id_to_int`). migrate `stable_point_id` (numeric-else-FNV-1a) is intentionally different for checkpoint resumability → **document, do NOT change** | Same logical doc must map to the same point ID across components | S | bit-identical today → no data migration | Haystack is **MIT** touching ID logic → escalated; valid fix = call shared helper |
| **T4** | Mobile: re-export core `GraphNode/GraphEdge/TraversalResult/GraphSchema` via UniFFI instead of redefining `MobileGraphNode/Edge/...` (already diverged: core `TraversalResult.path: Vec<u64>` absent in mobile). The forked `MobileGraphStore` engine itself = design decision, not a rewrite | Stops type shadowing / divergence | M | changes Swift/Kotlin API shape → care | Core |
| **T5** | LangChain/`integrations/common` re-declare `ALLOWED_METRICS`/`ALLOWED_STORAGE_MODES` parallel to the Rust enums → derive from the binding or single-source with a sync test | Avoid enum-set drift | S | low | MIT layer; keep canonical set sourced from core |
| **T6** | Docs: add the 3 architecture limitations to `KNOWN_LIMITATIONS.md` (ID-hash interop, WASM/CLI parser-only conformance, WASM fusion fork until T1) + thicken thin user-facing topics (HNSW tuning at create, collection-diagnostics, CollectionType, metadata upsert) | Real doc gap surfaced by the audit | S | doc-only | Core |

### Explicitly NOT doing (kept relevant)
- Do **not** expose core-internal metrics / durability / ColumnStore-batch / registry-introspection over bindings just to fill the matrix — by-design internal surface.
- Do **not** change migrate's `stable_point_id` algorithm — it would break existing resumable checkpoints (see `KNOWN_LIMITATIONS.md` #4). Document the intentional difference instead.
- Do **not** rewrite `MobileGraphStore`'s in-memory engine — only fix the type shadowing (T4).
- Pre-existing tracked parity gaps (RSF in Haystack, named-sparse-index *creation* in LangChain/LlamaIndex, `@collection` MATCH propagation) stay in `ECOSYSTEM_PARITY.md` "Remaining Gaps" / roadmap.

## Waves (each item = a feature branch off `develop` → PR, per Git Flow)

- **Wave 1 — safety net + quick wins (no search path):** T2, T3-Haystack, T5.  ← *in progress*
- **Wave 2 — divergence removal:** T1 (under the recall gate, protected by T2's net), T4 type re-export.
- **Wave 3 — docs:** T6.

EPIC IDs: highest referenced today is EPIC-078 → candidates EPIC-079…084 (confirm next free ID in the tracker before tagging code TODOs).

## Artifacts produced by the audit
- `PARITY_MATRIX.md` (81 capabilities × 13 components) — regenerate via `.claude/skills/core-parity-audit/scripts/gen_matrix.py`.
- `/core-parity-audit` skill — re-runs the whole analysis (code + docs), core = source of truth, VelesDB Core License boundary enforced.

## Resume pointer
If context is cleared: read this file + memory `core-children-arch-audit-2026-06-14` and
`core-source-of-truth-rule`. Wave 1 branches: `feature/velesql-executor-conformance` (T2),
`feature/idhash-haystack-single-source` (T3), `feature/integration-enum-single-source` (T5).
