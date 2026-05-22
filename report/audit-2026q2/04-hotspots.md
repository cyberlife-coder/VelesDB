# Churn-Based Risk Hotspot Audit — Q2 2026

**Period:** Last 6 months (since ~Nov 2025)  
**Methodology:** Risk Score = (churn_count × NLOC) / 100  
**Focus:** Files changed frequently AND complex = bug clusters

---

## Executive Summary

VelesDB shows **healthy churn patterns** — top hotspots are intentional refactors (GPU integration, NLOC compliance sweeps) rather than chaotic rework. However, **3 critical files combine high churn + heavy concurrency + complex logic**: GPU traversal, HNSW graph state, and the types registry.

**Key finding:** Concurrency markers (Arc, Atomic, parking_lot) heavily concentrated in graph/index modules. Test coverage adequate for query path, **sparse for GPU codepath** (1 test file vs 156 for query). **Dead-end files (test stubs with 1 churn) are low-priority blockers** for refactor sprints.

---

## 🔥 Top Fire Hotspots (High Risk)

### 1. `crates/velesdb-core/src/gpu/gpu_traversal.rs`
- **Churn:** 13 commits | **NLOC:** 616 | **Risk Score:** 80
- **Functions:** 12 (avg 51 NLOC/fn) — **well-factored**
- **Fix-related commits:** 10 (77% of all changes are fixes/perf)
- **Concurrency:** 0 locks, 0 unsafe — **clean**
- **Public API:** 3 exported functions (`traverse_gpu`, `search_auto`, `search_gpu`)
- **Test coverage:** 1 test file mentions gpu_traversal
- **Recent pattern:** Incremental perf/correctness (wgpu 29 migration, frontier accumulation fix, clippy pedantic passes)
- **Verdict:** 🟢 **Actively maintained, low bug accumulation**. High churn justified by GPU integration push.
- **Refactor action:** Add property tests for frontier-ordering invariants and empty-result edge cases in GPU search.

---

### 2. `crates/velesdb-core/src/collection/types.rs`
- **Churn:** 11 commits | **NLOC:** 555 | **Risk Score:** 61
- **Functions:** 2 (avg 277 NLOC/fn) — **concentrated**
- **Fix-related commits:** 4 (36% fix ratio)
- **Concurrency:** 43 Arc, 3 Atomic, 1 RwLock — **heavy Arc usage, shared state**
- **Public API:** 1 pub item (likely `Collection` or `CollectionConfig`)
- **Test coverage:** 56 test files mention types (highest coverage)
- **Recent pattern:** Rare direct changes; last 5 commits are broad refactors (Sprint 0-4 pre-seed, explain features, concurrency docs)
- **Verdict:** 🟡 **Stable but monolithic**. Large function bodies + Arc-heavy shared state = tight coupling.
- **Refactor action:** Extract `CollectionConfig` struct and `StateSnapshot` into separate modules to separate immutable schema from mutable runtime state. Target: 2 functions × 250 NLOC each.

---

### 3. `crates/velesdb-core/src/index/hnsw/native/graph/mod.rs`
- **Churn:** 11 commits | **NLOC:** 469 | **Risk Score:** 51
- **Functions:** 2 (avg 234 NLOC/fn) — **very concentrated**
- **Fix-related commits:** 7 (64% fix ratio) — **elevated bug activity**
- **Concurrency:** 14 Atomic, 4 Arc, 2 RwLock, 1 parking_lot::Mutex — **complex sync primitives**
- **Public API:** Part of core HNSW API (graph mutation, snapshot mgmt)
- **Test coverage:** 34 test files mention hnsw
- **Recent pattern:** Bug-heavy: cache invalidation race (#640), sentinel wipe race (#643), CSR cache per-instance isolation (#639)
- **Verdict:** 🔴 **Concurrency hotspot**. Arc snapshot cache + RwLock + Atomic version counter = intricate race window.
- **Refactor action:** (1) Add miri testing for version-counter snapshot ordering; (2) Document lock-ordering contract (gpu_vectors_snapshot → CsrCache → version counter); (3) Consider lock-free version via `seqlock` if miri finds races.

---

### 4. `crates/velesdb-core/src/index/hnsw/native_inner.rs`
- **Churn:** 11 commits | **NLOC:** 531 | **Risk Score:** 58
- **Functions:** 2 (avg 265 NLOC/fn)
- **Fix-related commits:** 6 (55% fix ratio)
- **Concurrency:** 1 RwLock, 2 unsafe blocks, 4 Send/Sync bounds
- **Public API:** Top-level HNSW API surface
- **Recent pattern:** GPU integration (search_auto wiring, u32 offset correctness, Devin review fixes). Unsafe blocks tied to GPU buffer casting.
- **Verdict:** 🟡 **Medium risk**. Unsafe code concentrated in GPU path; Send/Sync bounds indicate thread-safety awareness.
- **Refactor action:** Extract GPU-specific unsafe code into `gpu_buffer_cast.rs` submodule with `// SAFETY:` block documenting WGPU lifetime guarantees.

---

### 5. `crates/velesdb-core/src/index/hnsw/native/graph/gpu_search.rs`
- **Churn:** 14 commits (highest) | **NLOC:** 285 | **Risk Score:** 39
- **Functions:** 3 (avg 95 NLOC/fn) — **best-factored of hotspots**
- **Fix-related commits:** 8 (57% fix ratio)
- **Concurrency:** 0 locks, clean — **surprising for GPU path**
- **Public API:** GPU search dispatcher
- **Recent pattern:** Cache coherency fixes (CSR cache invalidation, snapshot renaming). Refactors for lock-ordering clarity.
- **Verdict:** 🟢 **Well-encapsulated**. No locks because Arc snapshot passed from `mod.rs`. Safe extraction point.
- **Refactor action:** None urgent. Document snapshot lifetime contract in rustdoc to clarify why no local locking needed.

---

## ⚠️ Watch List (Medium Risk)

| File | Churn | NLOC | Score | Fix% | Issue |
|------|-------|------|-------|------|-------|
| `collection/search/query/mod.rs` | 12 | 297 | 35 | 42% | Single 297-NLOC function; needs extraction |
| `collection/core/crud_bulk.rs` | 10 | 349 | 34 | 30% | Low fix ratio but 9 fns = high fan-in/out |
| `collection/graph/property_index/composite.rs` | 10 | 281 | 28 | 40% | Pre-seed refactor artifact; dormant pending cleanup |
| `gpu/gpu_csr.rs` | 9 | (est 400) | ? | ? | GPU memory management; not in top churn but critical path |

---

## ✅ Healthy Critical Files

Despite high churn, these show **low fix ratios + strong test coverage**:

- **`crates/velesdb-core/src/collection/types.rs`** — 36% fix, 56 test files, last fixes ~Sprint 0; now stable
- **`crates/velesdb-core/src/gpu/gpu_traversal.rs`** — 77% fixes but all intentional perf/compliance (wgpu, clippy), not bugs

**Signal:** Churn driven by feature + toolchain upgrades, NOT rework loops.

---

## 💤 Dormant Large Files (Low Sprint Priority)

Low churn (≤1 commit/6mo) + large NLOC = safe to defer refactoring:

- `column_store_tests.rs` (2575 NLOC, 1 churn)
- `collection/tests.rs` (1479 NLOC, 1 churn)
- `collection/core/crud_tests.rs` (1476 NLOC, 1 churn)
- `collection/core/index_management_tests.rs` (1210 NLOC, 1 churn)

**Recommendation:** Bundle dormant refactors into a Q3 "test hygiene" milestone, not blocking Q2.

---

## Concurrency Risk Profile

### Files with Atomic/RwLock/Unsafe Clustering

1. **`index/hnsw/native/graph/mod.rs`** — 14 Atomic + 2 RwLock + 2 parking_lot::Mutex → **version counter + cache invalidation race window**
2. **`collection/types.rs`** — 43 Arc + 3 Atomic → **high shared-state fan-out, low lock granularity**
3. **`index/hnsw/native_inner.rs`** — 2 unsafe + Send/Sync bounds → **GPU buffer casting, thread-safety attested**

### No concurrency markers in GPU codepath (gpu_traversal.rs, gpu_search.rs) — **Arc snapshot architecture offloads locking to caller** (graph/mod.rs).

---

## Sprint Priority List (ROI Rank)

**ROI = Risk reduced per hour invested**

1. **`hnsw/native/graph/mod.rs`** — Add miri testing + lock-order docs (4h). Risk: race → data corruption. ROI: **5x** (unblock GPU snapshot stability).
2. **`collection/types.rs`** — Extract `CollectionConfig` submodule (6h). Risk: monolith → maintenance friction. ROI: **3x** (improve dev velocity, clarify state ownership).
3. **`hnsw/native_inner.rs`** — Extract GPU unsafe to submodule (2h). Risk: future unsafe additions. ROI: **4x** (enforce pattern, reduce review load).
4. **`gpu/gpu_traversal.rs`** — Add property tests for frontier invariants (3h). Risk: low (well-factored), but GPU path test-sparse. ROI: **2x** (prevent GPU regressions).
5. **`collection/search/query/mod.rs`** — Split single 297-NLOC function (2h). Risk: missed CBO optimization interactions. ROI: **2x** (improve query planner comprehensibility).

**Total estimated effort:** ~17 hours. **Blockers:** None (all work in parallel after graph miri finishes).

---

## Audit Metadata

- **Scan date:** 2026-05-22
- **Git ref:** HEAD (audit-2026q2 worktree)
- **6-month window:** ~Nov 2025 – May 2026
- **Production crates scanned:** velesdb-core, velesdb-python, velesdb-server, velesdb-mobile
- **Test files excluded:** _tests.rs, /tests/ directories
