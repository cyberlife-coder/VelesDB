# VelesDB v2 — Core Trust Roadmap

**Version:** 3.2  
**Created:** 2025-02-08  
**Updated:** 2026-02-08  
**Milestone:** v2-core-trust (1 of 2)  
**Next Milestone:** v3-ecosystem-alignment  
**Total Phases:** 0 (merge) + 4 (execution)  
**Total Tasks:** 28  
**Findings covered:** 23/47 (velesdb-core findings, after triage)  
**Source:** Devil's Advocate Code Review (`DEVIL_ADVOCATE_FINDINGS.md`)

---

## Architectural Principle

> **velesdb-core = single source of truth.**  
> All external components (server, WASM, SDK, integrations) are bindings/wrappers.  
> Zero reimplemented logic. Zero duplicated code.  
> If a feature doesn't exist in core, it doesn't exist anywhere.

This milestone focuses exclusively on making velesdb-core **trustworthy**. The ecosystem alignment (making everything else a proper binding) is milestone v3.

---

## Findings Triage (v3.2 — 2026-02-08)

Code audit revealed that 2 findings are **already fixed** in `fusion/strategy.rs`:

| Finding | Status | Evidence |
|---------|--------|----------|
| C-04 (RRF formula wrong) | ✅ **FIXED** | `fusion/strategy.rs:224-249` — real `1/(k + rank+1)` with positional ranks |
| B-03 (Weighted = Average) | ✅ **FIXED** | `fusion/strategy.rs:252-300` — `avg_weight × avg + max_weight × max + hit_weight × hit_ratio` |

**Critical discovery:** Two parallel fusion implementations coexist:
- `fusion/strategy.rs` — **CORRECT** (standalone module, tested)
- `collection/search/query/score_fusion/mod.rs` — **BROKEN** (old RRF: `1/(k + (1-score)*100)`, used in query execution path)

**Decision:** Phase 2 will delete the old broken implementation and wire the query execution path to `fusion::FusionStrategy`.

**Adjusted scope:** 25 → 23 active findings (C-04, B-03 removed from scope).

---

## Phase 0: Merge & Tag v1-refactoring ✅ DONE

**Goal:** Ship v1 refactoring as-is. Tag the baseline. All v2 work on a clean branch.

### Status

- ✅ `main` = `develop` (commit `39385fbc`)
- ✅ Tag `v1.4.1-refactored` exists on origin
- ⬜ Branch `v2-core-trust` — to create

### Remaining Task

1. **Create `v2-core-trust` branch** from main

---

## Phase 1: CI Safety Net

**Goal:** Every change is validated automatically. No silent failures. Safety net before all subsequent phases.

**Findings addressed:** CI-01, CI-02, CI-03, CI-04  
**Estimate:** ~2h | **Risk:** 🟢 Low

### Tasks

| # | Task | File | Decision |
|---|------|------|----------|
| 1.1 | **Re-enable PR CI** | `.github/workflows/ci.yml:22-29` | Uncomment PR trigger. Keep path filtering (`crates/**`, `Cargo.*`). Cost controlled via `concurrency: cancel-in-progress`. |
| 1.2 | **Fix audit `\|\| true`** | `.github/workflows/ci.yml:135` | Replace `cargo audit --ignore RUSTSEC-2024-0320 \|\| true` with `cargo audit --ignore RUSTSEC-2024-0320`. Add inline comment explaining why RUSTSEC-2024-0320 is ignored. |
| 1.3 | **Add `cargo deny check` to CI** | `.github/workflows/ci.yml` | Add step in `security` job: `cargo install cargo-deny --locked && cargo deny check`. |
| 1.4 | **Tests multi-threaded** | `.github/workflows/ci.yml:114,116` | Remove `--test-threads=1` and `RUST_TEST_THREADS: 1`. Flaky tests get `#[serial]` individually — never mask concurrency bugs globally. |

### Success Criteria

- [ ] PRs to main/develop trigger CI
- [ ] `cargo audit` failure blocks merge (except documented RUSTSEC-2024-0320)
- [ ] `cargo deny check` runs in CI
- [ ] Tests run with default parallelism (no `--test-threads=1`)

---

## Phase 2: Critical Correctness — GPU + Fusion Unification

**Goal:** Fix everything that produces **mathematically wrong results**. Unify duplicated fusion logic.

**Findings addressed:** C-01, C-02, C-03, D-09  
**Findings already fixed (removed from scope):** ~~C-04~~, ~~B-03~~  
**Estimate:** ~8-10h | **Risk:** 🔴 High (WGSL shaders)

### Tasks

| # | Task | Finding | Decision | Complexity |
|---|------|---------|----------|-----------|
| 2.1 | **GPU Euclidean WGSL shader** | C-01 | Write real WGSL shader `batch_euclidean`. Same buffer layout as cosine shader. Add `euclidean_pipeline` field to `GpuAccelerator`. Shader computes `sqrt(Σ(q[i]-v[i])²)`. | 🔴 |
| 2.2 | **GPU DotProduct WGSL shader** | C-01 | Write real WGSL shader `batch_dot_product`. Simpler than cosine (no normalization): `Σ(q[i]*v[i])`. Add `dot_product_pipeline` field. | 🟡 |
| 2.3 | **GPU metric dispatch** | C-03 | `search_brute_force_gpu` dispatches on `self.metric`: Cosine→`batch_cosine_similarity`, Euclidean→`batch_euclidean_distance`, DotProduct→`batch_dot_product`. Hamming/Jaccard → CPU fallback with `tracing::warn!("GPU not implemented for {:?}, falling back to CPU", metric)`. | 🟡 |
| 2.4 | **DELETE GpuTrigramAccelerator** | C-02 | **Delete** the struct entirely. Rename to `TrigramAccelerator` (CPU). No GPU trigram planned. Lying struct name removed. | 🟢 |
| 2.5 | **Unify fusion: delete old score_fusion RRF** | D-09 | The query execution path in `score_fusion/mod.rs` has a broken `FusionStrategy` enum with fake RRF (`1/(k+(1-score)*100)`). **Delete** that enum. Wire query path to use `crate::fusion::FusionStrategy` instead. Extract `ScoreBreakdown` + `ScoreBoosting` into dedicated files if `score_fusion/mod.rs` > 300 lines post-change. | 🔴 |
| 2.6 | **Fusion params → ParseError** | D-09 | In parser `conditions.rs`, replace `unwrap_or(0.0)` with `return Err(ParseError::InvalidFusionParam { value, reason })`. | 🟢 |
| 2.7 | **Tests: GPU equivalence + fusion** | All | For each GPU metric: `gpu.batch_X() ≈ simd_native::X()` (tolerance 1e-5). Verify fusion paths produce identical results via old and new code paths. | 🟡 |

### Module Structure (Martin Fowler)

After Phase 2, `score_fusion/` will contain:
- `mod.rs` — `ScoreBreakdown`, apply logic (uses `crate::fusion::FusionStrategy`)
- `boost.rs` — `ScoreBoost`, decay, freshness, property boosting
- No more local `FusionStrategy` enum — single source of truth in `crate::fusion`

### Success Criteria

- [ ] Euclidean + DotProduct have real WGSL shaders with GPU pipeline
- [ ] `search_brute_force_gpu` dispatches to correct metric
- [ ] Hamming/Jaccard → CPU fallback with warning (not silent)
- [ ] `GpuTrigramAccelerator` deleted, replaced by `TrigramAccelerator`
- [ ] Only ONE `FusionStrategy` exists (`crate::fusion::FusionStrategy`)
- [ ] Old broken RRF formula (`1/(k+(1-score)*100)`) deleted
- [ ] Invalid fusion params → `ParseError`
- [ ] GPU vs CPU equivalence tests pass for Cosine, Euclidean, DotProduct

---

## Phase 3: Core Engine Bug Fixes

**Goal:** Fix silent bugs in search, traversal, quantization, and validation.

**Findings addressed:** B-01, B-02, B-04, B-05, B-06, D-08, M-03  
**Estimate:** ~6-8h | **Risk:** 🟡 Medium

### Tasks

| # | Task | Finding | Decision | Complexity |
|---|------|---------|----------|-----------|
| 3.1 | **Block NaN/Infinity vectors** | B-01 | In `extraction.rs`, the 3 `else` branches that cast NaN/Inf f64 → f32: change to return `None`. Pattern: `if !f.is_finite() { None }`. | 🟢 |
| 3.2 | **ORDER BY property paths → error** | B-02 | Return `UnsupportedFeature("ORDER BY property paths not yet supported")` error. No silent no-op. User gets clear feedback. | 🟢 |
| 3.3 | **BFS visited_overflow: stop inserting, don't clear** | B-05 | Replace `streaming.rs:210` `self.visited.clear()` with just `self.visited_overflow = true;`. Already-visited nodes stay detected. New nodes stop being tracked (bounded by `max_depth`). | 🟢 |
| 3.4 | **DFS: break not continue** | M-03 | In `traverser.rs`, `dfs_single`: change `continue` to `break` when limit reached. Consistent with BFS termination semantics. | 🟢 |
| 3.5 | **DualPrecision: use int8 by default** | B-04 | Modify `search_dual_precision()` to call the real int8 traversal path (like `search_with_config` does) when quantizer is trained. Fallback to f32 only when quantizer is NOT trained. | 🟡 |
| 3.6 | **cosine_similarity_quantized: no full dequant** | B-06 | Compute norm directly on quantized data: `norm² = Σ(int8[i]² × scale² + 2×int8[i]×scale×offset + offset²)`. Avoids `dimension × 4` bytes allocation per call. | 🟡 |
| 3.7 | **QuantizedVector → QuantizedVectorInt8** | D-08 | Rename `hnsw::native::quantization::QuantizedVector` to `QuantizedVectorInt8`. Clear disambiguation from `scalar::QuantizedVector`. | 🟢 |

### Success Criteria

- [ ] NaN/Infinity vector → `None` (filtered out), not silently cast
- [ ] ORDER BY property path → clear error, not silent no-op
- [ ] Cyclic graph BFS: zero duplicate `target_id` when `visited_overflow` triggers
- [ ] DFS stops immediately when limit reached (like BFS)
- [ ] `DualPrecisionHnsw::search()` uses int8 traversal when quantizer trained
- [ ] `cosine_similarity_quantized` allocates 0 extra bytes for norm
- [ ] No naming collision between `QuantizedVector` types
- [ ] 3,117+ existing tests still pass

---

## Phase 4: Performance, Storage & Cleanup

**Goal:** Latency, throughput, data integrity, dead code removal. Benchmarks before/after mandatory.

**Findings addressed:** D-01, D-02, D-03, D-04, D-05, D-06, D-07, M-01, M-02  
**Estimate:** ~8-10h | **Risk:** 🟡 Medium

### Tasks

| # | Task | Finding | Decision | Complexity |
|---|------|---------|----------|-----------|
| 4.1 | **HNSW: single read lock per search** | D-02 | Acquire `layers.read()` once at start of `search_layer`, store guard in local variable. Not per-candidate. Benchmark before/after under concurrent load. | 🟡 |
| 4.2 | **Adaptive over-fetch** | D-04 | Replace hardcoded `10 * count` with configurable via `WITH (overfetch = N)` in VelesQL. Default: 10. Range: 1-100. Add `WithClause::get_overfetch()` getter. | 🟢 |
| 4.3 | **ColumnStore: unify deletion** | D-01 | Delete `deleted_rows: FxHashSet<usize>`. Keep only `deletion_bitmap: RoaringBitmap`. Adapt all filter operations. If `column_store/mod.rs` > 300 lines, split into `deletion.rs` + `filter.rs`. | 🟡 |
| 4.4 | **WAL per-entry CRC** | D-05 | Each WAL entry header: `[len:u32][crc32:u32][payload]`. CRC computed on payload. Verified during replay. Enabled by default. Detects bit-flips and partial writes. | 🟡 |
| 4.5 | **LogPayloadStorage batch flush** | D-06 | Add `store_batch(&self, items: &[(u64, &[u8])]) → Result<()>`: write all entries, flush once. Existing `store()` unchanged for single insertions. | 🟢 |
| 4.6 | **Snapshot lock → AtomicU64** | D-07 | Replace `self.wal.write().get_ref().metadata()` with `AtomicU64` tracking WAL position. Updated on each write. `should_create_snapshot` becomes lock-free. | 🟢 |
| 4.7 | **CART: delete Node4 dead code** | D-03 | Remove `Node4` variant and its `#[allow(dead_code)]`. Add comment: `// Note: Leaf splitting is a known limitation. Leaves grow unbounded for high-cardinality prefixes.` | 🟢 |
| 4.8 | **Dead code cleanup** | M-01, M-02 | Delete `contains_similarity()` and `has_not_similarity()` from `validation.rs`. Replace `unreachable!()` in OrderedFloat with `Ordering::Equal` (safe, correct for NaN edge case). | 🟢 |
| 4.9 | **Benchmarks baseline + validation** | All | Save HNSW concurrency latency, WAL write throughput, ColumnStore deletion benchmarks BEFORE Phase 4. Re-run after. Zero regression tolerance on HNSW search latency. | 🟡 |

### Module Structure (Martin Fowler)

Post-Phase 4, if any file exceeds 300 lines:
- `column_store/mod.rs` → split `deletion.rs` + `filter.rs`
- `storage/log_payload.rs` → split `wal.rs` + `snapshot.rs`

### Success Criteria

- [ ] HNSW: one `layers.read()` per search call (not per candidate)
- [ ] Over-fetch factor configurable via `WITH (overfetch = N)`
- [ ] Single `RoaringBitmap` for deletion (no `FxHashSet` duplicate)
- [ ] WAL CRC32 detects corruption during replay
- [ ] `store_batch()` flushes once for N entries
- [ ] `should_create_snapshot` lock-free (AtomicU64)
- [ ] Zero dead code (`Node4`, unused validation functions)
- [ ] No performance regression vs baseline benchmarks

---

## Progress Tracker

| Phase | Status | Tasks | Findings | Estimate | Priority |
|-------|--------|-------|----------|----------|----------|
| 0 - Merge & Tag v1 | ✅ Done | 1 | — | 15 min | 🔒 Prerequisite |
| 1 - CI Safety Net | ✅ Done | 4 | CI-01→04 | 2h | 🛡️ Infrastructure |
| 2 - Critical Correctness | ✅ Done | 7 | C-01→03, D-09 | 8-10h | 🚨 Wrong Results |
| 3 - Core Engine Bugs | ✅ Done | 7 | B-01,02,04→06, D-08, M-03 | 6-8h | 🐛 Correctness |
| 4 - Perf, Storage, Cleanup | ✅ Done | 9 | D-01→07, M-01→02 | 8-10h | ⚠️ Optimization |

**Total:** 28 tasks | ~25-30h  
**Execution:** `0 → 1 → 2 → 3 → 4`  
**Findings covered:** 23/47 (core-only, after triage)

---

## What's Deferred to v3-ecosystem-alignment

The following findings require the **architectural principle** (core = single source of truth, everything else = binding). They form a separate milestone because they involve rewriting WASM, server, SDK, and integrations as proper wrappers:

| Finding | Subsystem | Why deferred |
|---------|-----------|-------------|
| S-01, S-02, S-03, S-04 | Server | Server needs auth + must bind to core graph, not reimplement |
| BEG-01, BEG-05, BEG-06 | WASM | WASM VectorStore must become a binding, not reimplementation |
| W-01, W-02, W-03 | WASM | Bugs in reimplemented code — will be deleted in v3 |
| T-01, T-02, T-03, BEG-07 | SDK | SDK bugs — fixable independently, grouped with ecosystem |
| I-01, I-02, I-03, BEG-02, BEG-03, BEG-04 | Integrations | Dead params + duplication — needs shared Python base |
| I-04 | GPU | Hamming/Jaccard GPU shaders — nice to have |

**Total deferred:** 22 findings → Milestone v3

---

## Quality Gates (per phase)

```powershell
cargo fmt --all --check
cargo clippy -- -D warnings
cargo deny check
cargo test --workspace
cargo build --release
.\scripts\local-ci.ps1
```

**Rule:** If ANY gate fails → fix before proceeding. No exceptions.

---

## Key Decisions (v3.2)

| Date | Decision | Rationale |
|------|----------|-----------|
| 2025-02-08 | GPU: implement real WGSL shaders | User decision — full GPU implementation for Cosine, Euclidean, DotProduct |
| 2025-02-08 | Merge v1 BEFORE starting v2 | Clean baseline, always rollback-able |
| 2025-02-08 | CI first (Phase 1) | Safety net for all subsequent changes |
| 2025-02-08 | Split into 2 milestones | v2 = core trust, v3 = ecosystem alignment (too big for one) |
| 2025-02-08 | Core = single source of truth | All WASM/server/SDK/integrations must be bindings, zero reimplementation |
| 2026-02-08 | C-04/B-03 removed from scope | Already fixed in `fusion/strategy.rs` — verified by code audit |
| 2026-02-08 | Delete old score_fusion FusionStrategy | Broken RRF formula `1/(k+(1-score)*100)` still in query path — must be replaced by `fusion::FusionStrategy` |
| 2026-02-08 | Delete GpuTrigramAccelerator | No GPU trigram planned. Rename to `TrigramAccelerator`. Don't lie in struct names. |
| 2026-02-08 | BFS overflow: stop inserting, don't clear | `visited.clear()` causes duplicates in cyclic graphs. Keep visited set intact, just stop growing it. |
| 2026-02-08 | ORDER BY property → error | Silent no-op is worse than error. User gets clear feedback. |
| 2026-02-08 | DualPrecision default → int8 | `search()` must use int8 when quantizer trained. Fallback f32 only when untrained. |
| 2026-02-08 | Tests multi-thread in CI | `--test-threads=1` masks concurrency bugs. Fix tests individually with `#[serial]`. |
| 2026-02-08 | Module split at 300 lines | Martin Fowler tiny steps. Any file >300 lines post-change gets split. |
| 2026-02-08 | Hamming/Jaccard GPU → CPU fallback | Not worth WGSL shader investment now. Explicit `tracing::warn!` on fallback. Deferred to v3 (I-04). |

---
*Milestone v2 — Core Trust. Created from Devil's Advocate Review (47 findings).*  
*Version 3.2: Triage of already-fixed findings + concrete task-level plans.*  
*See also: v3-ecosystem-alignment (22 findings deferred)*
