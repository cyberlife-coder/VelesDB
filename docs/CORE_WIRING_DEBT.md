# Core Wiring Debt

> **Last updated**: 2026-06-12 — inventory re-verified against the code on
> this date (entries 1, 2, 3, and 5 re-checked at their call sites; entry 4
> is a qualitative single-source-of-truth cleanup and unchanged).

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

**Outcome**: **Partially wired** (runtime-only, no persistence).

**What exists**:
- `crates/velesdb-core/src/collection/auto_reindex/mod.rs` defines
  `AutoReindexManager` with a policy that uses `std::time::Duration` for
  the cooldown period between reindex triggers.

**What is missing**:
- `CollectionConfig` does NOT currently persist `AutoReindexConfig` to
  `config.json`. Serializing `Duration` via serde requires either a
  custom representation (seconds-as-u64) or a schema version bump.
- The runtime-only attachment **shipped** (re-verified 2026-06-12):
  users call `VectorCollection::attach_auto_reindex(manager)` after opening
  the collection. The manager is NOT restored on `Database::open`.

**Why this is intentional**:
- Keeping `AutoReindexManager` out of the persisted config avoids the
  schema version bump and the `Duration` serde decision.
- Runtime-only attachment fits the typical agentic-workflow pattern where
  the reindex policy is determined at application startup, not at
  collection creation.

**Future action**: add `serde(with = "serde_duration_secs")` helper and
persist `AutoReindexConfig` in a future schema version. This is a
Community-scope enhancement; no enterprise angle.

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

**Outcome**: **Not yet wired** (Wave 3 Commit 7 wired 2 of 5 fields).

**What exists**:
- Sprint 2 Wave 3 Commit 7 (`ed2a057a`) enforces `max_collections` and
  `max_dimensions` at collection creation time via
  `Database::ensure_collection_name_available` and
  `enforce_vector_dimension_limit`.

**What is missing** (re-verified 2026-06-12 — the three fields are
range-validated in `config_validation.rs` but still not enforced at runtime):
- `max_vectors_per_collection` — would require instrumentation in the
  hot upsert path.
- `max_payload_size` — would require a size check in `write_store_record`
  before the WAL write.
- `max_perfect_mode_vectors` — would require a runtime check in the
  exact-search path.

**Why this is deferred**:
- Commit 7 targeted the two fields where enforcement is cheap and the
  default is safely permissive (1000 collections, 4096 dimensions). The
  remaining three fields add hot-path overhead and need benchmarks to
  validate the cost.

**Future action**: unscheduled Community-scope backlog item (not started as
of 2026-06-12) — wire the remaining three fields with benchmarks proving the
hot-path cost is <1%.

---

## Summary table

| Config | Wired? | Outcome | Effort | Target |
|---|---|---|---|---|
| `WalBatchConfig` | No | Transferred to velesdb-premium | 13-17 commits, 3-5 days | Enterprise tier |
| `AutoReindexConfig` | Runtime-only | Partially wired (attachment API shipped) | 1 commit | Community (schema bump later) |
| `deferred_indexing` / `async_index_builder` | REST-only (no TOML/Python) | RFC pending | Unscoped | Community (future sprint) |
| `SearchConfig` global defaults | Partial | Consolidation cleanup | 1-2 commits | Community (future sprint) |
| `LimitsConfig` (3/5 fields) | Partial | Hot-path instrumentation | 2-3 commits | Community (backlog, unscheduled) |

## Conventions

- **New entries** use the same structure: what exists, what is missing,
  why wiring is non-trivial, current behavior, future action.
- **Before removing an entry**, update the referenced Sprint plan and
  verify no user-facing docs reference the config struct.
- **Cross-references** to `.planning/` or `velesdb-premium` backlogs are
  welcomed — the audit trail matters more than keeping this file short.
