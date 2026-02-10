# Phase 6 Research: Unified Cross-Store Query Engine & Full-Text Search

**Date:** 2026-02-09
**Researcher:** Cascade
**Duration:** ~45 min deep audit + external research

---

## Executive Summary

**The PHASE.md assumptions are SIGNIFICANTLY outdated.** Most of the claimed "missing" features already exist in the codebase. The actual gaps are much smaller and more focused than originally estimated.

### Revised Estimate: ~12-16h (down from 20-25h)

---

## Existing Implementation Audit

### ✅ BM25 Full-Text Search — ALREADY IMPLEMENTED

**File:** `crates/velesdb-core/src/index/bm25.rs` (402 lines)

| Feature | Status | Quality |
|---------|--------|---------|
| BM25 scoring (k1/b tunable) | ✅ Done | Production-grade |
| Inverted index (FxHashMap) | ✅ Done | Thread-safe (RwLock) |
| Adaptive PostingList (HashSet → RoaringBitmap) | ✅ Done | Auto-promotes at 1000+ docs |
| Document add/remove | ✅ Done | Handles updates correctly |
| Top-k search with partial sort | ✅ Done | O(n log k) via select_nth_unstable |
| Tokenizer (whitespace + punctuation) | ✅ Done | Basic but functional |

**Missing:** Stemming, stop-word removal (acceptable for v1 — these are optimizations).

### ✅ Trigram Index — ALREADY IMPLEMENTED

**Directory:** `crates/velesdb-core/src/index/trigram/` (7 files, ~500+ lines)

| Feature | Status | Quality |
|---------|--------|---------|
| Trigram extraction (pg_trgm style) | ✅ Done | Zero-copy, padded |
| Inverted index (RoaringBitmap) | ✅ Done | Compressed |
| LIKE pattern search | ✅ Done | Bitmap intersection |
| Jaccard similarity scoring | ✅ Done | For ranked results |
| SIMD acceleration (AVX-512/AVX2/NEON) | ✅ Done | Multi-arch |
| Thread safety | ✅ Done | Tested |
| TrigramAccelerator (batch ops) | ✅ Done | CPU compute backend |

### ✅ Hybrid Search (Vector + BM25) — ALREADY IMPLEMENTED

**File:** `crates/velesdb-core/src/collection/search/text.rs` (301 lines)

| Feature | Status | Quality |
|---------|--------|---------|
| `text_search()` — BM25 only | ✅ Done | Production |
| `text_search_with_filter()` — BM25 + metadata | ✅ Done | 4x over-fetch |
| `hybrid_search()` — Vector + BM25 with RRF | ✅ Done | Streaming top-k |
| `hybrid_search_with_filter()` — All combined | ✅ Done | Production |

### ✅ Fusion Strategies — ALREADY IMPLEMENTED

**File:** `crates/velesdb-core/src/fusion/strategy.rs` (308 lines)

| Strategy | Status | Notes |
|----------|--------|-------|
| Average | ✅ Done | Mean of scores |
| Maximum | ✅ Done | Max score per doc |
| RRF (k=60) | ✅ Done | Standard reciprocal rank fusion |
| Weighted (avg + max + hit_ratio) | ✅ Done | Validated weight sum |

### ✅ Multi-Query Search — ALREADY IMPLEMENTED

**File:** `crates/velesdb-core/src/collection/search/batch.rs` (404 lines)

| Feature | Status | Notes |
|---------|--------|-------|
| `multi_query_search()` | ✅ Done | Multi-vector + fusion + filter |
| `multi_query_search_ids()` | ✅ Done | Lightweight variant |
| `search_batch_parallel()` | ✅ Done | Parallel HNSW |
| Overfetch factor (adaptive) | ✅ Done | Based on top_k |

### ✅ Query Planner — ALREADY IMPLEMENTED

**Files:** `velesql/planner.rs` (353L) + `collection/search/query/match_planner.rs` (315L)

| Feature | Status | Notes |
|---------|--------|-------|
| VectorFirst / GraphFirst / Parallel | ✅ Done | Heuristic-based |
| QueryStats (EMA latency) | ✅ Done | Thread-safe atomic |
| HybridExecutionPlan | ✅ Done | Over-fetch, early termination |
| MatchQueryPlanner | ✅ Done | EXPLAIN support |
| Cost estimation | ✅ Done | Based on runtime stats |

### ✅ Hybrid Fusion (Vector + Graph) — ALREADY IMPLEMENTED

**File:** `velesql/hybrid.rs` (301 lines)

| Feature | Status | Notes |
|---------|--------|-------|
| `fuse_rrf()` | ✅ Done | Vector + Graph |
| `fuse_weighted()` | ✅ Done | Normalized scores |
| `fuse_maximum()` | ✅ Done | Max across sources |
| `intersect_results()` | ✅ Done | AND semantics |
| `normalize_scores()` | ✅ Done | Min-max normalization |

**Note:** Marked `#[allow(dead_code)]` — not yet wired into main execution.

---

## ❌ Actual Gaps Identified

### Gap 1: NEAR_FUSED Execution (VP-012) — NOT WIRED

**Problem:** `VectorFusedSearch` is parsed, validated, and in the AST, but `execute_query()` has NO code path to actually execute it.

**Evidence:**
- `filter/conversion.rs:101`: `VectorFusedSearch(_) => Self::And { conditions: vec![] }` — identity (ignored)
- `where_eval.rs:90`: `VectorSearch(_) | VectorFusedSearch(_) => Ok(true)` — pass-through
- `execute_query()` in `mod.rs`: Only handles `VectorSearch` (NEAR), not `VectorFusedSearch`

**Solution:** Wire `VectorFusedSearch` → `multi_query_search()` in execute_query(). All pieces exist, they just need connection.

**Estimate:** 3-4h (including tests)

### Gap 2: Cross-Store Query Coordination (VP-010) — NOT WIRED

**Problem:** The planner exists but is NOT integrated into `execute_query()`. Comment in `mod.rs:16`: "Future: Integrate QueryPlanner::choose_hybrid_strategy() into execute_query()".

**Evidence:**
- `execute_query()` handles NEAR OR MATCH but not NEAR + MATCH in same WHERE
- `hybrid.rs` fusion functions are `#[allow(dead_code)]`
- No code coordinates Vector + Graph + Column in single execution

**Specific missing combinations:**
1. `WHERE vector NEAR $v AND MATCH 'keyword'` — partially works (calls `hybrid_search`)
2. `WHERE vector NEAR $v AND MATCH (a)-[:REL]->(b)` — NOT supported
3. Three-way: NEAR + Graph MATCH + Column filters — NOT supported

**Solution:** 
1. Integrate `QueryPlanner` into execute_query() for strategy selection
2. Add execution path for NEAR + Graph MATCH (vector candidates → graph validation)
3. Wire hybrid.rs fusion into main execution pipeline

**Estimate:** 6-8h (complex, touches core query engine)

### Gap 3: BM25 Integration with VelesQL (VP-011 partial)

**Problem:** `text_search()` and `hybrid_search()` work at the Collection API level, but VelesQL `MATCH 'keyword'` in execute_query() only calls `text_search()` in one specific branch.

**Evidence:**
- `execute_query()` line 344: Only handles `Condition::Match` in the `(None, None, Some)` branch
- No integration of BM25 with NEAR in the VelesQL execution path
- Trigram index exists but not wired into BM25 as pre-filter

**Solution:**
1. Ensure `MATCH 'keyword' AND vector NEAR $v` dispatches to `hybrid_search()`
2. Optional: Wire trigram as pre-filter for BM25 (performance optimization)

**Estimate:** 2-3h

### Gap 4: Benchmarks & Validation

**Problem:** No benchmarks proving the 22-128x speedup claim for trigram or BM25 performance.

**Solution:** Add criterion benchmarks for BM25 and trigram on 10K/100K datasets.

**Estimate:** 2-3h

---

## External Research Findings

### BM25 Best Practices (2024-2025)

1. **bm25 crate** (Michael-JB/bm25): Rust BM25 with embedding support, tokenizer-agnostic
2. **ParadeDB/pg_search**: BM25 in PostgreSQL via Rust extension, uses block-WeakAND
3. **Exa.ai BM25 optimization**: In-memory inverted list, pre-computed IDF, BlockMax WAND
4. **Key insight**: Our implementation already follows best practices (inverted index, adaptive postings, partial sort)

### Cross-Store Query Patterns (2024-2025)

1. **SingleStore-V (VLDB 2024)**: Integrated vector DB — uses plan-level fusion, not query-level
2. **GOpt (GraphScope)**: Unified graph+relational optimizer — relevant for MATCH + WHERE
3. **Weaviate**: BM25 + vector hybrid via inverted index + HNSW, fused at retrieval level
4. **Key pattern**: Execute store-specific queries → intersect on ID space → post-filter → re-rank

### NEAR_FUSED Execution Pattern

Standard approach from Milvus, Qdrant, Weaviate:
1. Execute each vector query independently (parallel batch)
2. Apply per-query filters if any
3. Fuse results using RRF/weighted/max strategy
4. Return top-k from fused results

**VelesDB already has all pieces** — `multi_query_search()` does exactly this.

---

## Revised Plan Structure

### Wave 1: NEAR_FUSED Execution (VP-012) — 1 plan
- **06-01**: Wire VectorFusedSearch → multi_query_search() in execute_query()
  - Resolve vector parameters from NEAR_FUSED
  - Map FusionConfig → FusionStrategy
  - Extract metadata filters for post-fusion filtering
  - E2E tests for all fusion strategies

### Wave 2: Cross-Store Integration (VP-010) — 2 plans
- **06-02**: Integrate QueryPlanner into execute_query()
  - Wire HybridExecutionPlan selection
  - Handle NEAR + BM25 MATCH in VelesQL
  - Remove dead_code from hybrid.rs
- **06-03**: NEAR + Graph MATCH execution
  - Vector-first → graph validation path
  - Graph-first → vector scoring path  
  - Parallel execution with result fusion
  - E2E tests for cross-store queries

### Wave 3: Validation & Benchmarks (VP-011 completion) — 1 plan
- **06-04**: BM25/Trigram Benchmarks + Quality Gates
  - Criterion benchmarks (10K/100K datasets)
  - Validate 22x+ speedup claim
  - Trigram → BM25 pre-filter integration (optional optimization)
  - E2E tests for full-text + vector hybrid queries

**Total: 4 plans, ~12-16h (down from 9 plans, 20-25h)**

---

## Risk Assessment

| Risk | Probability | Impact | Mitigation |
|------|------------|--------|------------|
| execute_query() complexity (already 390 lines) | High | Medium | Extract cross-store dispatch to separate module |
| Graph MATCH + NEAR ordering conflicts | Medium | High | Vector-first always for ORDER BY similarity() |
| Benchmark doesn't meet 22x claim | Low | Medium | Adjust claims to match reality |
| Breaking existing query paths | Medium | High | Comprehensive regression tests first |

---

*Research complete. Ready to create detailed PLAN.md files.*
