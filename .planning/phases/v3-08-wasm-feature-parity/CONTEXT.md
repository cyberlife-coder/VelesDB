# Phase v3-08: WASM Feature Parity — Context

**Captured:** 2026-02-09

## Vision

VelesDB's promise is **Vector + Graph + Column, unified by VelesQL, local-first**.
The WASM crate currently delivers only Vector + Graph. The Column Store — a core
differentiator — is entirely absent from the browser target. This phase closes that
gap and exposes all relevant non-persistence core features to WASM.

## User Experience

A developer using `velesdb-wasm` in a browser (PWA, Tauri, extension) should have
access to the full triptyque:
- **VectorStore** — similarity search on embeddings
- **GraphStore** — knowledge graph with traversal
- **ColumnStore** — structured metadata with typed columns, filtering, TTL

All three stores should be independently usable and persistable to IndexedDB.
VelesQL should be able to query across all three store types.

## Essentials

Things that MUST be true:
- ColumnStore is available in WASM with full CRUD (schema, insert, upsert, delete, TTL)
- Column filtering works (eq, gt, lt, range, in — both scalar and bitmap)
- String interning works in WASM (StringTable)
- IndexedDB persistence for ColumnStore (like GraphPersistence)
- IR metrics (recall, precision, nDCG, MRR) are exposed for quality evaluation
- Half precision (f16/bf16) is exposed for memory optimization

## Boundaries

Things to explicitly AVOID:
- Do NOT port `from_collection.rs` — it depends on `Collection` (persistence)
- Do NOT attempt HNSW in WASM — rayon dependency, brute-force is fine for browser datasets
- Do NOT build a full Database abstraction in WASM — the 3 stores stay independent
- Do NOT add `roaring` as a direct WASM dependency if it bloats the bundle — use Vec<usize> fallback if needed
- Do NOT break any existing WASM API — additive only

## Implementation Notes

- `column_store` module has ZERO persistence deps (roaring + rustc_hash only)
- Only `from_collection.rs` needs `Collection` — gate it behind `persistence`, extract the rest
- Same pattern as Phase v3-01 Plan 02: extract pure-data types from persistence gate
- `metrics`, `half_precision`, `cache` are already available without `persistence` feature
- WASM ColumnStore bindings follow the same pattern as GraphStore: thin #[wasm_bindgen] wrapper + conversion at boundary

## Open Questions

- Should RoaringBitmap be exposed to WASM or should we use Vec<usize> for filter results?
- Should VelesQL execution (not just parsing) be wired to the 3 stores in WASM?
- Priority of IndexedDB persistence for ColumnStore vs other tasks?

---
*This context informs planning. The planner will honor these preferences.*
