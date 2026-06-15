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

EPIC IDs: highest referenced today is EPIC-080 (`ci.yml`) → candidates EPIC-081… (confirm next free ID in the tracker before tagging code TODOs).

## EPIC-081 (proposed) — Ordered-index `ORDER BY <col> LIMIT k` pushdown (B001 perf follow-up)

**Why.** The B001 fix made scalar `ORDER BY <col> … LIMIT k` *correct* by fetching the full
matching set before sorting — but at O(n): measured **312 ms** for `ORDER BY year DESC LIMIT 10`
over 50k rows (capped at `MAX_LIMIT`=100k). An ordered index serves top-k without scanning all rows.

**Substrate decision (from the design fan-out).** Build on **`SecondaryIndex::BTree`**
(`crates/velesdb-core/src/index/secondary/mod.rs`) — the only flat-payload ordered structure that is
actually *populated and maintained* on upsert/delete/bulk, keyed by field name, with a total-`Ord`
`JsonValue` key. **Do not** use `RangeIndex` (`collection/graph/range_index.rs`): it is **inert**
(`self.range_index.insert` is never called from any CRUD path), label-bound, and its range queries
return *unordered* bitmaps. Tradeoff: `SecondaryIndex` is **not snapshotted** (rebuilt on `create_index`
via backfill), so the optimization is **opt-in per field** — which matches existing secondary-index
semantics and needs no new persistence format. (A later phase may add snapshotting in `flush.rs`.)

**Phase 1 — primitive (DONE, shipped in PR #1127).** `SecondaryIndex::ordered_ids` + golden tests.
Pure BTreeMap walk (`.values()`, `.rev()` for DESC), O(log n + k), ascending IDs within an equal-key
bucket for determinism.

**Phase 2 — minimal planner routing (DONE).** Implemented as `Collection::try_ordered_index_scan`
(`collection/search/query/ordered_index_scan.rs`), wired into `execute_select_pipeline`. Coverage is
checked atomically via `SecondaryIndex::ordered_ids_if_covered` (Σ bucket lengths == `config.point_count`
under one read lock). The ORDER BY sort gained an ascending point-id final tie-break (`ordering.rs`) so it
is deterministic AND identical to the index walk. Score is 1.0 to match the exhaustive metadata-scan path.
`ordered_ids_if_covered` returns `Vec<u64>` (no bitmap), so ids > `u32::MAX` are fine — no id-range
restriction needed. Equivalence gate `tests/ordered_index_order_by_equivalence.rs` (13 tests) proves the
same query yields identical (id, score) sequences with vs without the index across ASC/DESC/OFFSET/ties/
k>n/k==n/k=0/not-covered/coverage-break/id>u32::MAX/WHERE-fallback. Perf: ~89 ms exhaustive → ~0.013 ms
index (50k rows). recall@10 unaffected. *(Original design notes below, kept for reference.)*

**Phase 2 design (reference).** At the B001 hook
`Collection::order_by_requires_exhaustive_fetch` (`select_dispatch.rs:98`), add an `Option<OrderedScanPlan>`
that fires **only** when ALL hold: primary `ORDER BY` is a single plain `Field` (not Aggregate/Arithmetic/
similarity); that field has a secondary index; the field is **fully covered** (index id-count == point count —
no missing/JSON-null rows, which a full sort places first/last but the index omits); and there is **no**
WHERE / JOIN / graph / DISTINCT / vector-search. Then fetch the top-k IDs from `ordered_ids`, **snapshot the
IDs and release the index lock before hydrating** (`get(ids)` re-reads payload, so it tolerates concurrent
writes and respects lock ordering), and skip the `MAX_LIMIT` exhaustive fetch + in-memory sort. Else fall
straight through to today's exact behavior — zero change. **The similarity() HNSW fast path is untouched**
(the hook already returns false for a leading `similarity()` key).

**Gate (Phase 2).** A correctness matrix asserting the index path's result **equals** the unbounded sort
truncated to k (KNOWN_LIMITATIONS #9) across ASC/DESC × {OFFSET, ties, exact-k, k>n}; an explicit
tie-determinism decision (the full-scan uses an *unstable* sort, so ties have no canonical order — the index
path is deterministic by ID, arguably better but a behavior change to ratify); a recall@10 non-regression run;
and a perf benchmark showing sub-linear top-k vs the 312 ms/50k baseline. Disable the route entirely for any
collection containing an id > `u32::MAX` (the bitmap paths can't represent it).

**Phase 3 — broaden (separate, optional).** Four independent sub-items, sequenced as gated PRs (a design
fan-out found the three perf sub-items each *sound-with-fixes*, never *sound* — see their must-fix lists):

- **3a — auto-index advisor (DONE).** Recommendation-only: `Collection::order_by_advisor` records eligible
  `ORDER BY <field>` queries that decline the fast path for want of a covering secondary index;
  `VectorCollection::order_by_index_advice(min)` surfaces them with live-derived state `Missing` /
  `BuiltButUncovered` (a now-covering field is *resolved* and dropped). Behavior-neutral (never mutates an
  index or a result), shape-isolated to the eligible `ORDER BY` shape, observations capped. Never
  auto-creates (a backfill is `O(n)` on the query thread). `tests/ordered_index_advisor.rs`.
- **3b — WHERE-filtered top-k (DONE).** A pure-metadata `WHERE` is now eligible: the route walks the
  covering ordered index applying the **same** predicate the exhaustive path applies
  (`Filter::new(Condition::from(extract_metadata_filter(where)))`), snapshots all covered ids under one lock
  + coverage check, releases, then hydrates in 1024-row batches and stops at offset+limit matches. Declines
  (no advisor observation) when `point_count > MAX_LIMIT` (matches the capped baseline) or estimated
  selectivity `< 0.1` (`CostEstimator` — the exhaustive bitmap prefilter wins for selective filters); u64
  ids only, no bitmap intersection. Equivalence matrix (range/OR/NOT/IN/offset+ties/uncovered) +
  TTL-expired-row regression in `tests/ordered_index_order_by_equivalence.rs`. Also fixed a latent phase-2
  divergence the review surfaced: the plain route now backfills past deleted/TTL-expired rows (or declines)
  instead of returning a short/misaligned page.
- **3c — multi-column `ORDER BY` (DONE).** `ORDER BY <lead>, <more…>` (2+ plain `Field` keys, no WHERE) on a
  covering lead index: `SecondaryIndex::ordered_prefix_if_covered` absorbs whole leading lead-key buckets
  (check-then-absorb) until ≥ offset+limit rows, then `Collection::apply_order_by` runs the **exact**
  exhaustive multi-key sort over the prefix + OFFSET/LIMIT. Correct because the lead key dominates the
  comparator (every trailing-bucket row sorts strictly after the prefix). The index `JsonValue` Ord
  (`Bool<Number<String`, f64 `total_cmp`) matches `compare_json_values`, so heterogeneous primitive lead
  types are safe; non-primitive / non-f64 lead values aren't indexed → coverage breaks → fall-through.
  Declines above `MAX_LIMIT`, on multi-key+WHERE, on any non-`Field` key, and (like the plain route) when a
  deleted/TTL-expired row sits in the prefix. Gate refactored to a `ScanPlan` enum.
  `tests/ordered_index_multikey.rs` (9). Adversarial review: SHIP (830+ equivalence assertions + a 700-case
  fuzz, no divergence).
- **3d — secondary-index snapshotting.** Persist the BTree in `flush.rs` ATOMICALLY under the payload write
  lock; reconcile the coverage denominator for vector collections; reconstruct F64 keys from `u64` bits (no
  `f64` round-trip); config-persisted indexed-field-names as authority; restart-equivalence + concurrency tests.

## Artifacts produced by the audit
- `PARITY_MATRIX.md` (81 capabilities × 13 components) — regenerate via `.claude/skills/core-parity-audit/scripts/gen_matrix.py`.
- `/core-parity-audit` skill — re-runs the whole analysis (code + docs), core = source of truth, VelesDB Core License boundary enforced.

## Resume pointer
If context is cleared: read this file + memory `core-children-arch-audit-2026-06-14` and
`core-source-of-truth-rule`. Wave 1 branches: `feature/velesql-executor-conformance` (T2),
`feature/idhash-haystack-single-source` (T3), `feature/integration-enum-single-source` (T5).
