# VelesDB v2 — Core Trust Roadmap

**Version:** 3.1  
**Created:** 2025-02-08  
**Milestone:** v2-core-trust (1 of 2)  
**Next Milestone:** v3-ecosystem-alignment  
**Total Phases:** 0 (merge) + 4 (execution)  
**Findings covered:** 25/47 (all velesdb-core findings)  
**Source:** Devil's Advocate Code Review (`DEVIL_ADVOCATE_FINDINGS.md`)

---

## Architectural Principle

> **velesdb-core = single source of truth.**  
> All external components (server, WASM, SDK, integrations) are bindings/wrappers.  
> Zero reimplemented logic. Zero duplicated code.  
> If a feature doesn't exist in core, it doesn't exist anywhere.

This milestone focuses exclusively on making velesdb-core **trustworthy**. The ecosystem alignment (making everything else a proper binding) is milestone v3.

---

## Phase 0: Merge & Tag v1-refactoring ⬅️ DO FIRST

**Goal:** Ship v1 refactoring as-is. Tag the baseline. All v2 work on a clean branch.

### Tasks

1. **Merge `develop` → `main`** (local merge — PR CI is disabled, that's a v2 fix)
2. **Tag `v1.4.1-refactored`** on main
3. **Push** main + tag to origin
4. **Create `v2-core-trust` branch** from main

### Success Criteria

- `main` = current `develop`
- Tag `v1.4.1-refactored` on origin
- Branch `v2-core-trust` created

---

## Phase 1: CI Safety Net + Core Quality

**Goal:** Every change is validated. No silent failures. Core quality checks enforced.

**Findings addressed:** CI-01, CI-02, CI-03, CI-04

### Tasks

1. **Re-enable PR CI** — Uncomment PR trigger in `ci.yml` with path filtering for cost control
2. **Fix security audit** — Remove `|| true`; document RUSTSEC-2024-0320 ignore with reason
3. **Add `cargo deny check`** to CI — enforce what `local-ci.ps1` already mandates
4. **Tests run multi-threaded** — Remove `--test-threads=1` to catch concurrency bugs
5. **Tests:** PR to develop triggers CI; cargo audit failure blocks; cargo deny runs

### Success Criteria

- PRs trigger CI on main/develop
- `cargo audit` failure blocks merge (except documented exceptions)
- `cargo deny check` in CI
- Tests run with default parallelism

---

## Phase 2: Critical Correctness (Wrong Results)

**Goal:** Fix everything in velesdb-core that produces **mathematically wrong results** today.

**Findings addressed:** C-01, C-02, C-03, C-04, B-03, D-09

### Tasks

1. **GPU: Wire Euclidean/DotProduct shaders** — Connect existing `EUCLIDEAN_SHADER` and `DOT_PRODUCT_SHADER` to real wgpu pipelines (like `batch_cosine_similarity` already does)
2. **GPU: Fix metric dispatch** — `search_brute_force_gpu` must respect the index's `DistanceMetric`
3. **GPU: Clean up `GpuTrigramAccelerator`** — Either implement real GPU kernels or remove the struct entirely
4. **Fusion: Implement real RRF** — `Σ 1/(k + rank_i)` with positional ranks from sorted result lists
5. **Fusion: Fix Weighted ≠ Average** — Add real per-component weight configuration
6. **Fusion: Fix param parsing** — Return `ParseError` for invalid values, not `unwrap_or(0.0)`
7. **Tests:** GPU vs CPU result equivalence for all metrics; RRF known-ranking validation; Weighted differs from Average with non-uniform weights

### Success Criteria

- All 5 distance metrics have real GPU pipelines (or explicit `#[cfg(not(feature = "gpu"))]` CPU fallback)
- `search_brute_force_gpu` dispatches to correct metric
- RRF matches Cormack et al. formula
- Weighted ≠ Average when weights differ
- Invalid fusion params → `ParseError`

---

## Phase 3: Core Engine Bug Fixes

**Goal:** Fix all correctness bugs in velesdb-core's search, traversal, quantization, and validation.

**Findings addressed:** B-01, B-02, B-04, B-05, B-06, D-08, M-03

### Tasks

1. **Block NaN/Infinity vectors** — Validate f32 components in VelesQL extraction; error for non-finite
2. **ORDER BY property paths** — Implement or return `UnsupportedFeature` error
3. **BFS visited_overflow** — Stop inserting new nodes, but do NOT clear visited set
4. **DFS termination** — `break` (not `continue`) when limit reached, consistent with BFS
5. **DualPrecision search** — Use quantized int8 traversal when quantizer is trained
6. **cosine_similarity_quantized** — Cache quantized norm, no full dequantization
7. **QuantizedVector naming** — Disambiguate `scalar::QuantizedVector` from `hnsw::QuantizedVectorInt8`
8. **Tests:** NaN query → error; cyclic graph → 0 duplicates; DualPrecision uses int8; all existing tests pass

### Success Criteria

- NaN/Infinity → clear error
- Graph traversal: zero duplicate target_ids on cycles
- DualPrecision uses quantized distances by default
- 3,117+ existing tests still pass

---

## Phase 4: Performance, Storage & Cleanup

**Goal:** Lock contention, storage throughput, data integrity, dead code removal.

**Findings addressed:** D-01, D-02, D-03, D-04, D-05, D-06, D-07, M-01, M-02

### Tasks

1. **HNSW: single read lock per search** — Acquire `layers.read()` once, not per candidate
2. **Adaptive over-fetch** — Configurable factor replacing hardcoded 10x
3. **ColumnStore: unify deletion** — `RoaringBitmap` only, remove `FxHashSet`
4. **WAL per-entry CRC** — Optional CRC32 for corruption detection
5. **LogPayloadStorage batch flush** — `store_batch()` method
6. **Fix snapshot lock** — `AtomicU64` for WAL position tracking
7. **CART: remove Node4 dead code** — Document leaf splitting as known limitation
8. **Dead code** — Delete unused validation functions, fix `unreachable!()` in OrderedFloat
9. **Benchmarks:** Before/after HNSW latency under concurrency

### Success Criteria

- HNSW: one read lock per search call
- Single `RoaringBitmap` for deletion
- WAL CRC detects corruption
- Zero dead code
- No performance regression

---

## Phase 5: Performance Optimization with Arxiv Research

**Goal:** Optimize velesdb-core performance based on latest research papers from Arxiv, without breaking changes to exposed functions.

**Findings addressed:** Performance optimizations based on academic research

### Tasks

1. **Research survey** — Review latest Arxiv papers on vector database optimizations
2. **Identify applicable techniques** — Select optimizations that can be implemented without API changes
3. **Implement optimizations** — Apply research findings to HNSW, SIMD, and storage layers
4. **Benchmark validation** — Verify performance improvements against baseline
5. **Document findings** — Create performance optimization guide

### Success Criteria

- Performance improvements validated by benchmarks
- No breaking changes to public APIs
- Documentation of applied research findings
- All existing tests pass

---

## Progress Tracker

| Phase | Status | Scope | Priority |
|-------|--------|-------|----------|
| 0 - Merge & Tag v1 | ⬜ Pending | Git workflow | 🔒 Prerequisite |
| 1 - CI Safety Net | ⬜ Pending | CI-01→04 | 🛡️ Infrastructure |
| 2 - Critical Correctness | ⬜ Pending | C-01→04, B-03, D-09 | 🚨 Wrong Results |
| 3 - Core Engine Bugs | ⬜ Pending | B-01→06, D-08, M-03 | 🐛 Correctness |
| 4 - Perf, Storage, Cleanup | ⬜ Pending | D-01→07, M-01→02 | ⚠️ Optimization |
| 5 - Performance Arxiv | ⬜ Pending | Research-based optimizations | 🚀 Performance |

**Execution:** `0 → 1 → 2 → 3 → 4 → 5`
**Findings covered:** 25/47 (core-only)

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

---

## Decision Log

| Date | Decision | Rationale |
|------|----------|-----------|
| 2025-02-08 | GPU: implement real WGSL shaders | User decision — full GPU implementation for all metrics |
| 2025-02-08 | Merge v1 BEFORE starting v2 | Clean baseline, always rollback-able |
| 2025-02-08 | CI first (Phase 1) | Safety net for all subsequent changes |
| 2025-02-08 | Split into 2 milestones | v2 = core trust, v3 = ecosystem alignment (too big for one) |
| 2025-02-08 | Core = single source of truth | All WASM/server/SDK/integrations must be bindings, zero reimplementation |

---
*Milestone v2 — Core Trust. Created from Devil's Advocate Review (47 findings).*  
*See also: v3-ecosystem-alignment (22 findings deferred)*
