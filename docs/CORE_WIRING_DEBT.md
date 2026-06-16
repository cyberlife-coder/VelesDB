# Core Wiring Debt

> **Last updated**: 2026-06-14 — inventory re-verified against the code on
> this date (entries 1, 2, 3, and 5 re-checked at their call sites; entry 4
> is a qualitative single-source-of-truth cleanup and unchanged; §5
> `LimitsConfig` runtime enforcement and the §6 Observer plane re-checked).

This document lists configuration structures and subsystems that exist in
`velesdb-core` but are **not yet fully wired** to the user-facing runtime.
Each entry captures what exists, what is missing, why it has not been wired,
and what the wiring would cost.

This is an **internal engineering document**. User-facing guides are in
`docs/guides/`.

---

## Why this document exists

During Sprint 2 Wave 3 of the pre-seed remediation, we audited every
`*Config` struct reachable from `VelesConfig` to verify it actually
influences runtime behavior. Most do. A few do not — they are serde-parsed
and stored on `Database` but no code path consults their fields. Rather
than silently removing them (which would hide historical intent) or
quietly exposing them in Python bindings (which would mislead users), we
document them here with a concrete wiring plan.

Three outcomes are possible per entry:

1. **Wired in a future velesdb-core sprint** — the cost/value fits
   Community scope.
2. **Transferred to velesdb-premium** — the effort is significant and
   the feature has enterprise-tier characteristics.
3. **Removed from the schema** — the config was speculative and will be
   dropped in a future breaking release.

Every entry below names its target outcome explicitly.

---

## 1. `WalBatchConfig` — payload WAL group commit

**Outcome**: **Transferred to velesdb-premium** (concurrent WAL writer feature).

**What exists**:
- `crates/velesdb-core/src/config.rs` defines `WalBatchConfig` with
  `enabled: bool`, `commit_delay_us: u64`, `max_batch_size: usize`.
- `crates/velesdb-core/src/storage/wal_batcher.rs` defines a `WalBatcher`
  struct with `submit` / `flush` methods that amortize fsync cost by
  buffering writes and performing a single syscall at batch boundary.

**What is missing**:
- No call site uses `WalBatcher`. It is a dormant utility (re-verified
  2026-06-12: still zero call sites outside `wal_batcher.rs`).

**Why wiring is non-trivial**:
- `LogPayloadStorage` uses a `RwLock<BufWriter<File>>` with offset tracking
  at write time (the in-memory `index: id → offset_in_file` is populated
  inside `write_store_record`). `WalBatcher::submit` buffers data in a
  `Vec<u8>` and defers the flush — so the eventual file offset is not known
  until post-flush. Wiring `WalBatcher` directly would require moving
  offset resolution to a post-flush callback, which requires redesigning
  the CRC + index update path.
- More critically, `Collection::payload_storage` is declared as
  `Arc<RwLock<LogPayloadStorage>>` and every writer acquires `.write()` on
  this outer RwLock **before** entering `LogPayloadStorage` (see
  `crud.rs`, `crud_bulk.rs`, `bulk_import.rs`, `flush.rs`, `graph_api.rs`,
  `crud_read_delete.rs` — ~20 call sites). This outer lock already
  serializes all writers to the same collection, so even a fully
  refactored `LogPayloadStorage` with a lock-free queue + leader-follower
  flush would produce **zero measurable throughput gain**: the
  Collection-level `RwLock::write()` guard upstream acts as a funnel.
- Delivering real concurrent-writer value requires **both** the
  `LogPayloadStorage` internal refactor **and** a cascade promotion of
  the `PayloadStorage` trait from `&mut self` to `&self`, plus migration
  of ~60 call sites across 25+ files in `collection/core/` and
  `collection/search/`. Estimated 13-17 commits, 3-5 focused days, high
  risk of invariant violations in the upsert pipeline.

**Why this is a reasonable Community limit**:
Every open-source vector database enforces single-writer per collection:
Qdrant OSS, Weaviate Community, Chroma (via SQLite), Milvus OSS. The
`store_batch_inner` path already amortizes fsync across an entire batch
for single-writer workloads, so bulk imports run at the hardware limit.
Multi-writer contention only manifests when multiple clients write to
the **same** collection concurrently — a workload pattern typically
associated with multi-tenant SaaS and high-ingestion pipelines.

**Current behavior**:
- `VelesConfig::wal_batch` is parsed from TOML, stored on `Database`,
  and ignored by the runtime.
- `WalBatchConfig::default()` sets `enabled = false` so no user workflow
  is silently affected.
- The Python binding **does not expose** `WalBatchOptions` in the
  `VelesConfigOptions` dataclass. Exposing a no-op toggle would mislead
  users into believing the flag affects throughput.

**Cross-reference**: the full enterprise plan lives in an internal planning
document (`WAVE3_B2_W2_WAL_REDESIGN`, not in this repository) and in the
velesdb-premium backlog. The customer-facing framing is in
`docs/guides/WRITE_CONCURRENCY.md`.

**Future action in Community**: none. The `WalBatchConfig` struct remains
visible (TOML parsing continues to accept it for forward compatibility),
but the Python API does not surface it. A future breaking release may
remove the `enabled` field entirely if it remains unused after the
Enterprise tier ships.

---

## 2. `AutoReindexConfig.cooldown` — Duration serde round-trip

**Outcome**: **Wired** (persisted in schema v2 — W2).

**What exists**:
- `crates/velesdb-core/src/collection/auto_reindex/mod.rs` defines
  `AutoReindexManager` with a policy that uses `std::time::Duration` for
  the cooldown period between reindex triggers.
- `AutoReindexConfig` now derives `Serialize`/`Deserialize`; its `cooldown`
  `Duration` round-trips as whole seconds via the custom serde helper in
  `crates/velesdb-core/src/collection/config_serde.rs` (`duration_secs`).
- `CollectionConfig.auto_reindex_config: Option<AutoReindexConfig>` persists
  the policy in `config.json` under schema v2 (`CURRENT_SCHEMA_VERSION = 2`).
  `VectorCollection::attach_auto_reindex` mirrors the manager's config into
  this field, and `Collection::open` restores the `AutoReindexManager`
  automatically (`restore_auto_reindex_from_config`). No manual re-attach is
  required after a `Database::open` once a flush has persisted the config.

**What is missing**: nothing for the persistence gap. The auto-reconstruct
decision after a divergence event remains caller-driven (out of scope, by
design — see entry 2 history and `notify_auto_reindex_after_bulk`).

---

## 3. `deferred_indexing` / `async_index_builder` nested configs

**Outcome**: **Partially wired** (feature-gated, requires RFC for full exposure).

**What exists**:
- `crates/velesdb-core/src/collection/streaming/` contains
  `DeferredIndexer` and `AsyncIndexBuilder` — both wire up when the
  corresponding field is `Some(...)` on `Collection`.
- The V2 bulk path in `crud_bulk.rs::upsert_bulk_v2_path` uses
  `async_index_builder` when configured.

**What is missing** (re-verified 2026-06-12):
- No way to **configure** either subsystem via `VelesConfig` (TOML). The
  REST `POST /collections` request now accepts `deferred_indexing` and
  `async_index_builder` objects (applied via `apply_advanced_config`), so
  the gap is the embedded/TOML surface, not the server API.
- No Python binding exposes a `DeferredIndexingOptions` or
  `AsyncIndexBuilderOptions` dataclass.

**Why wiring is non-trivial**:
- Both subsystems have subtle interactions with crash recovery: if
  `DeferredIndexer` buffers 10K vectors and the process crashes, the
  gap-detection path on `Collection::open` must re-index them. This is
  tested for the V1 path but not end-to-end for the deferred case.
- `AsyncIndexBuilder` uses a merge threshold that interacts with HNSW
  graph size — choosing a user-facing default requires a benchmark sweep
  and guidance for tuning.

**Future action**: scope an RFC for "Streaming Ingestion Configuration"
that covers both subsystems, the persisted config schema, the recovery
proof, and the tuning guide. Out of scope for pre-seed Sprints 2-4.

---

## 4. `SearchConfig` global defaults vs. per-call overrides

**Outcome**: **Partially wired** (defaults exist, per-call overrides don't cascade).

**What exists**:
- `VelesConfig::search: SearchConfig` holds default `ef_search`,
  `exact_threshold`, and other knobs.
- `Collection::search_with_ef` accepts an `ef` override at call time.

**What is missing**:
- `VelesConfig::search.ef_search` is read at `Collection` construction
  time but some search paths in `collection/search/query/match_exec/`
  hard-code a local default instead of consulting the runtime config.
- The Python binding exposes `search(query, k, ef=...)` but does not
  document the fallback chain (call-time → config → hard-coded).

**Why this is non-critical**:
- The hard-coded defaults match the `SearchConfig::default()` values,
  so observable behavior is consistent in the common case. The debt is
  a lack of single-source-of-truth for the defaults.

**Future action**: audit every search-path function in
`collection/search/` and consolidate the fallback chain through a single
helper. Community-scope cleanup, not a feature.

---

## 5. `LimitsConfig::max_vectors_per_collection` / `max_payload_size` / `max_perfect_mode_vectors`

**Outcome**: **Wired** (2026-06-14 — all 5 `LimitsConfig` fields now enforced).

**What exists**:
- Wave 3 Commit 7 enforces `max_collections` and `max_dimensions` at
  collection creation time via `Database::ensure_collection_name_available`
  and `enforce_vector_dimension_limit`.
- The remaining three fields are now threaded from the live
  `VelesConfig::limits` into each `Collection` (new `runtime_limits` field,
  populated by `Database::push_runtime_limits` at every vector/graph/metadata
  registration and disk/startup open path) and enforced at the cold ingest/
  search boundary:
  - `max_vectors_per_collection` — one O(1) `len() + batch > cap` check
    (shared `enforce_vector_count` helper) on every ingest entry:
    `Collection::upsert` / `upsert_bulk_inner` / `upsert_metadata` (Point
    paths) and `upsert_bulk_from_raw` (the zero-copy Python/REST bulk path),
    before any storage lock or WAL write. It is a conservative pre-count: a
    pure update batch re-supplying existing ids is counted as net-new, so a
    collection exactly at the cap may reject an in-place update (raise the cap
    to update at the limit). Vector-less graph node writes never touch
    `config.point_count`, so this field does not apply to pure-graph node
    ingest.
  - `max_payload_size` — a per-point serialized-size check (shared
    `enforce_payload_value_size` helper) on the same cold ingest boundary for
    every payload path: Point upsert, raw bulk, and graph node writes
    (`store_node_payload`). The size is measured with a bounded counting
    writer that aborts past the cap, so it allocates no throwaway buffer and
    serializes at most `cap + 1` bytes — **not** in the innermost WAL
    `write_store_record` loop.
  - `max_perfect_mode_vectors` — a single gate (`enforce_perfect_mode_limit`)
    consulted by all four index search entry points
    (`search_with_quality` / `search_with_opts` and the filtered path
    `search_with_filter_and_opts`) before `index.search_with_quality`, so a
    `WITH (mode='perfect')` query — filtered or not — cannot trigger an
    unbounded brute-force scan and the HNSW/SIMD inner loop is never touched.

A violation returns `Error::GuardRail` (VELES-027) carrying the actual value,
the cap, and the `limits.<field>` to raise — mirroring the two creation-time
gates. The `runtime_limits` field is **not** persisted to `config.json`; it is
re-pushed from the live `VelesConfig` on every open.

---

## 6. Binding API-parity wiring gaps (core fns not exposed in embedded bindings)

**Outcome**: **Partially wired** — backlog below, prioritised P1–P3.

This entry records the result of a full core→binding API-parity audit
(re-verified 2026-06-13 against `develop` post-#1094, with `file:line`
evidence for every cell). The audit replaced an earlier informal matrix
that was substantially inaccurate — most of its "missing in Python /
nowhere" claims were **false**: `velesdb-python` already exposes
`search_with_ef`, `search_ids`, `search_batch_parallel`,
`multi_query_search`, `multi_query_search_ids`, `sparse_search`,
`hybrid_sparse_search`, `execute_match_with_similarity`, `flush_full`,
`upsert_bulk_from_raw`, `is_delta_active`, `explain_analyze_query`,
`GraphCollection::add_edges_batch`/`delete`/`remove_edge` natively. The
genuine, worth-closing gaps that survived verification are below.

**Audit ground rules** (why some "gaps" are *not* listed as debt):
- **WASM** links `velesdb-core` with `persistence` OFF and reimplements
  its own in-memory engine. Every persistence/runtime-gated core method is
  therefore **N/A** in WASM, not a closeable gap (only the three
  non-gated `validate_*` free-fns are real — see 6.11).
- **Mobile / Tauri / WASM graph stores** are standalone in-memory stores
  that wrap nothing in core; their graph rows are partial-via-VelesQL, by
  design.
- `MetadataCollection::search` is a structural no-op (metadata collections
  have no vector index) — use `text_search`/VelesQL. Documented, not debt.
- `velesdb-server` is the source of truth for `execute_aggregate`,
  `rebuild_index`, `compact_storage`, `apply_advanced_config`,
  `update_guardrails`, `explain_analyze_query`, `push_to_delta_if_active` —
  the TS SDK merely forwards to native server routes.

**What is missing** (backlog — each row: core fn · surfaces lacking it ·
feasibility · target file for the glue):

| # | Core fn | Missing in | Pri | Feasibility | Glue lands in |
|---|---------|-----------|-----|-------------|---------------|
| 6.1 | `VectorCollection::compact_storage` | python, mobile, tauri | P1 | moderate (run under `allow_threads`/`spawn_blocking`) | `velesdb-python/src/collection/mod.rs` (mirror `flush_full`); `velesdb-mobile/src/collection.rs` |
| 6.2 | `GraphCollection::add_edges_batch` (typed route/cmd) | rest, tauri | P1 | trivial | `velesdb-server/src/handlers/` (new `/graph/edges/batch`); `tauri-plugin-velesdb/src/lib.rs` |
| 6.3 | `VectorCollection::search_ids` (true core path) | rest, tauri | P2 | trivial (server route reuses generic search + strips payloads; never calls core `search_ids`) | `velesdb-server/src/handlers/search/mod.rs:344` (route to core fn when no payloads requested); tauri command |
| 6.4 | `VectorCollection::reorder_for_locality` | python, rest, tauri, mobile, ts (**nowhere**) | P2 | moderate — **recall-gated** (QUALITY_BAR Gate 1) | server admin route first (next to `compact_storage`/`rebuild_index`), then `velesdb-python/src/collection/mod.rs` |
| 6.5 | `VectorCollection::apply_advanced_config` | python, mobile, tauri | P2 | moderate — config marshalling across FFI; recall-adjacent (PQ/HNSW) | `velesdb-python/src/collection/` (py-dict→`AdvancedConfig`); mobile UniFFI record |
| 6.6 | `AnyCollection::diagnostics` / `Database::collection_diagnostics` (typed) | python, rest-typed, tauri, mobile | P2 | moderate — define serializable diagnostics DTO once | server `/collections/{name}/diagnostics`; `velesdb-python/src/database.rs` (returns dict) |
| 6.7 | `Database::update_guardrails` + `VectorCollection::guard_rails` (read) | python, mobile, tauri | P2 | moderate — limits struct marshalling | `velesdb-python/src/database.rs`; mobile `lib.rs` |
| 6.8 | `detach_auto_reindex` / `check_auto_reindex_divergence` | python (only `attach`), all others | P2 | moderate — store manager handle on wrapper. **See entry 2** | `velesdb-python/src/collection/` (same wrapper as `attach_auto_reindex`) |
| 6.9 | `VectorCollection::search_batch_parallel` | rest, tauri, mobile | P2 | moderate — server batch route funnels through serial kernel; needs a parallel branch flag | `velesdb-server/src/handlers/search/` (batch handler) |
| 6.10 | `VectorCollection::multi_query_search_ids` | rest, tauri, mobile, ts | P3 | trivial — id-only variant of broadly-exposed `multi_query_search` | server `search/multi.rs` (ids-only mode); TS `search-backend.ts` |
| 6.11 | `validate_collection_name` / `validate_dimension` / `validate_dimension_match` (free-fns) | every binding re-implements them | P3 | trivial — consistency, not a feature | re-export + replace local validators in python `lib.rs`, server validation, ts `client/validation.ts` |

**Status — PR-1 `feat/parity-embedded-ops`** (MERGED, PR #1096 → develop):
- 6.1 `compact_storage` — DONE (Python, Mobile, Tauri; each with a unit test).
- 6.2 `add_edges_batch` — DONE (server `/collections/{name}/graph/edges/batch` route + OpenAPI snapshot + Tauri command; HTTP integration test + Tauri tests).
- 6.3 `search_ids` — Tauri command DONE (functional gap closed; unit-tested). **Server fast-path moved to PR-4** (grouped with 6.9: conditional hot-path opt, core `search_ids` has no filter param).

**Status — PR-2 `feat/parity-ops-observability`** (MERGED, PR #1098 → develop):
- 6.6 `collection_diagnostics` — DONE. New core DTO `CollectionDiagnosticsResponse`; server `GET /collections/{name}/diagnostics` route (+ OpenAPI); Python `Database.collection_diagnostics()` → dict. HTTP integration + pytest.
- 6.7 guardrails get/update — DONE. Server already had it. Added Python (`Database.update_guardrails(dict)` + `collection.guard_rails()` read), Mobile (`MobileQueryLimits` record + `update_guardrails` + `collection.guard_rails`), Tauri (`update_guardrails`/`get_guardrails` commands). Tests on each. Note: `QueryLimits` (runtime) ≠ `LimitsConfig` (creation caps) — separate marshalling.
- 6.8 auto-reindex lifecycle — DONE. Python `collection.detach_auto_reindex()` + `check_auto_reindex_divergence()` (completes the attach-only state; see entry 2). Pytest.

**Status — PR-3 `feat/parity-recall-gated`** (MERGED, PR #1099 → develop):
- 6.4 `reorder_for_locality` — DONE. Server `POST /collections/{name}/locality/reorder` admin route (+ OpenAPI snapshot), Python `collection.reorder_for_locality()`. Server BDD tests (nominal/empty/404) + pytest. Recall-preserving (only physical layout changes); does **not** touch `index/hnsw/`, `simd_native/`, `quantization/`, `fusion/`, or Python result conversion — Gate-1 recall-contract tests (balanced ≥95, accurate ≥99, perfect =100) green.
- 6.5 `apply_advanced_config` — DONE. Python `collection.apply_advanced_config(dict)` with three-state semantics (absent key=unchanged, `None`=clear, value=set) over `pq_rescore_oversampling` / `deferred_indexing` / `async_index_builder`; Mobile `MobileAdvancedConfig` record (+ `MobileDeferredIndexerConfig` / `MobileAsyncIndexBuilderConfig`) and `collection.apply_advanced_config` (mobile maps `None`→unchanged; cannot express clear). pytest + mobile `#[test]`.

**Status — PR-4 `feat/parity-consistency`** (the final campaign PR):
- 6.9 `search_batch_parallel` — DONE. Server `/search/batch` dispatches to `search_batch_parallel` (rayon, no-filter throughput path) when no query carries a filter, else `search_batch_with_filters`. Same HNSW traversal, so the unfiltered case is identical to serial. Parity test (`parity_consistency_tests.rs`) pins batch == per-query `/search`.
- 6.3-server `search_ids` true core path — DONE. `/search/ids` now calls core `search_ids` (skips payload hydration) for plain dense requests (no filter/sparse/`ef_search`/quality `mode`); any other shape falls back to the generic pipeline. `search()` and `search_ids()` share `search_ids_with_adc_if_pq`, so the fast path's id/score ranking is identical — pinned by a `/search` vs `/search/ids` parity test. (Tauri `search_ids` command already landed in PR-1.)
- 6.10 `multi_query_search_ids` — DONE. Server `POST /collections/{name}/search/multi/ids` (+ OpenAPI) calls core `multi_query_search_ids` (rejects filters — the kernel has no filter param); TS `multiQuerySearchIds` across `search-backend.ts` → REST/WASM backends (WASM = not-supported stub, mirroring `searchIds`) → `Backend` interface → client. Server parity + filter-rejection + 404 tests; TS backend tests.
- 6.11 `validate_*` free-fn dedup — **VERIFIED no-op (no real gap).** The matrix row was overstated: no binding re-implements these. Core already re-exports `validate_collection_name` / `validate_dimension` / `validate_dimension_match` from the crate root (`lib.rs`); Python relies on core validation via error-mapping (no local validators in `lib.rs`); the server only matches the `InvalidCollectionName` *error* variant (no local name/dimension validator); TS is client-only and has no `validateCollectionName` / `validateDimension`. Nothing to consolidate.

**Status — Wave 1 PR-L** (`feat/parity-mobile-cli-diagnostics`, MERGED #1106,
2026-06-14): 6.6 mobile `diagnostics` DONE (`collection.rs:620`); 6.10 mobile
`multi_query_search_ids` DONE (`collection_sparse.rs:144`); CLI `.diagnostics`
REPL command added (`repl_collection_cmds.rs`). Still open after PR-L: 6.9
`search_batch_parallel` on mobile (mobile batch still routes through
`search_batch_with_filters`) and all three diagnostics / multi_query /
search_batch_parallel surfaces on Tauri.

**Status — Observer plane** (item P, 2026-06-14): **DONE for Python + server
e2e.** Python `Database(path, observer=cb)` injects a `PyObserver` that bridges
the four core *notify* hooks to one callable `cb(event, **fields)` — events
`collection_created` (`name`, `kind`), `collection_deleted` (`name`), `upsert`
(`collection`, `point_count`), `query` (`collection`, `duration_us`). In the
embedded SDK only `collection_created`/`collection_deleted` fire (the core
emits them directly); `upsert`/`query` are emitted by callers that measure and
call `notify_upsert`/`notify_query` — that is the REST server, now covered
end-to-end by `crates/velesdb-server/tests/observer_lifecycle_tests.rs` (a
`CountingObserver` injected via `Database::open_with_observer` asserts all four
hooks across create/upsert/search/delete). The two **veto** hooks
(`on_ddl_request`/`on_dml_mutation_request`) stay trait-default (allow) and are
intentionally **not** exposed — no policy/RBAC engine in the open SDK. WASM is
architecturally N/A; TypeScript-REST (SSE/WS) and Mobile remain deferred.

**Streaming ingestion (`stream_insert_batch` / `enable_streaming` /
`StreamIngester::*`) — DONE across the first-party bindings (2026-06-14).** The
async-runtime + channel-handle-lifetime concern was solved by giving each
binding a process-wide tokio streaming runtime that the drain task is scheduled
on: Python (`velesdb-python/src/streaming_runtime.rs` + `collection/mutation.rs`
`enable_streaming`/`stream_insert`) and Mobile/UniFFI
(`velesdb-mobile/src/streaming_runtime.rs` +
`collection.rs:481`/`:508`), alongside Server
(`POST /collections/{name}/stream/enable` + `/stream/insert`), the TS SDK
(`enableStreaming()`/`streamInsert()`, REST backend) and Tauri. WASM is N/A (no
async fs / persistence layer; throws `NOT_SUPPORTED`). The CLI reaches it ⚠️ via
the embedded core path with no dedicated REPL command. Only the
LangChain/LlamaIndex/Haystack integrations do not yet expose it. See
`docs/reference/ECOSYSTEM_PARITY.md` (Streaming Ingestion row).

**Explicitly deferred (not worth closing now)**:
- `upsert_bulk_from_raw` beyond Python — REST/TS already have JSON
  `upsert_bulk`; the zero-copy raw path needs a flat-buffer wire format
  for marginal benefit. Keep Python/NumPy-only.

**Future action**: schedule 6.1–6.3 (P1 + trivial-P2) as one embedded-ops
PR; gate 6.4/6.5 behind a recall-validation run (Gate 1); 6.10/6.11 as a
consistency-cleanup PR. Each binding glue must pass that crate's CI line
(no `.unwrap()`, complexity ≤8, clippy pedantic).

---

## Summary table

| Config | Wired? | Outcome | Effort | Target |
|---|---|---|---|---|
| `WalBatchConfig` | No | Transferred to velesdb-premium | 13-17 commits, 3-5 days | Enterprise tier |
| `AutoReindexConfig` | Yes | Wired — persisted in schema v2 (W2) + restored on open | done | Community |
| `deferred_indexing` / `async_index_builder` | REST-only (no TOML/Python) | RFC pending | Unscoped | Community (future sprint) |
| `SearchConfig` global defaults | Partial | Consolidation cleanup | 1-2 commits | Community (future sprint) |
| `LimitsConfig` (5/5 fields) | Yes | Wired — all 5 fields enforced (creation + runtime ingest/search caps) 2026-06-14 | done | Community |
| Binding API-parity gaps (6.1–6.11) | DONE | Closed via 4 PRs #1096/#1098/#1099 + consistency (6.11 = verified no-gap); see §6 | 4 PRs (embedded-ops / ops-observability / recall-gated / consistency) | Community |

## Conventions

- **New entries** use the same structure: what exists, what is missing,
  why wiring is non-trivial, current behavior, future action.
- **Before removing an entry**, update the referenced Sprint plan and
  verify no user-facing docs reference the config struct.
- **Cross-references** to `.planning/` or `velesdb-premium` backlogs are
  welcomed — the audit trail matters more than keeping this file short.
