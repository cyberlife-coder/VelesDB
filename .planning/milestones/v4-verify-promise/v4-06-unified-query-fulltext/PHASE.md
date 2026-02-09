# Phase 6: Unified Cross-Store Query Engine & Full-Text Search

## Overview

**Goal:** Wire existing BM25/Trigram/Fusion/Planner infrastructure into the VelesQL execution pipeline and validate cross-store query capabilities claimed on velesdb.com.

**Requirements:** VP-010, VP-011, VP-012
**Estimate:** 14-19h (revised from 20-25h after research — see RESEARCH.md)
**Priority:** 🚨 Critical — Site claims features not fully wired

---

## Research Summary (2026-02-09)

**Key finding:** Most infrastructure already exists. The GAPS are wiring, not implementation:

| Component | Status | Gap |
|-----------|--------|-----|
| BM25 Index (`index/bm25.rs`) | ✅ 402 lines, production | None — fully implemented |
| Trigram Index (`index/trigram/`) | ✅ 7 files, SIMD | None — fully implemented |
| Hybrid Search (V+BM25) | ✅ `text.rs` 301 lines | Partial VelesQL integration |
| Fusion Strategies | ✅ `fusion/strategy.rs` | None — 4 strategies ready |
| Multi-Query Search | ✅ `batch.rs` 404 lines | Not wired to NEAR_FUSED |
| Query Planner | ✅ `planner.rs` 353 lines | Not wired to execute_query() |
| MATCH Planner | ✅ `match_planner.rs` 315 lines | Not wired to execution |
| Hybrid Fusion (V+G) | ✅ `hybrid.rs` 301 lines | `#[allow(dead_code)]` — unused |
| NEAR_FUSED parsing | ✅ Grammar + AST | **No execution path** |

Full details: `RESEARCH.md`

---

## Problem Statement (Revised)

### 1. NEAR_FUSED Execution Gap (VP-012)
- **Parsed:** ✅ Grammar, AST (`VectorFusedSearch`), validation
- **Executed:** ❌ `execute_query()` has NO code path for `VectorFusedSearch`
- **Fix:** Wire to existing `multi_query_search()` — all pieces exist

### 2. Cross-Store Query Coordination (VP-010)
- **Planner:** ✅ Exists (`QueryPlanner`, `MatchQueryPlanner`)
- **Wired:** ❌ Comment: "Future: Integrate QueryPlanner into execute_query()"
- **hybrid.rs:** ✅ Fusion functions exist but marked `dead_code`
- **Fix:** Integrate planner, handle NEAR + Graph MATCH in same query

### 3. BM25 VelesQL Integration (VP-011)
- **BM25:** ✅ Fully implemented
- **VelesQL:** Partial — only `MATCH 'keyword'` alone works, not combined with NEAR
- **Fix:** Ensure NEAR + MATCH 'keyword' dispatches to `hybrid_search()`

---

## Success Criteria

### VP-012: NEAR_FUSED Execution
- [ ] `NEAR_FUSED [$v1, $v2] USING FUSION 'rrf'` executes end-to-end
- [ ] All 4 fusion strategies work: RRF, Average, Maximum, Weighted
- [ ] Metadata filters applied post-fusion
- [ ] ORDER BY + LIMIT work with fused results

### VP-010: Cross-Store Queries
- [ ] `NEAR $v AND MATCH 'keyword'` → hybrid_search()
- [ ] QueryPlanner integrated into execute_query()
- [ ] hybrid.rs fusion functions no longer dead_code
- [ ] E2E tests for combined query patterns

### VP-011: BM25 Full Integration
- [ ] VelesQL `MATCH 'keyword' AND vector NEAR $v` dispatches correctly
- [ ] Benchmarks validate performance claims (trigram 22x+ vs LIKE scan)

---

## Plans (4 plans, 2 waves)

### Wave 1 (can be parallel):
- **06-01**: NEAR_FUSED Execution Wiring (VP-012) — 3-4h
- **06-02**: BM25 + NEAR VelesQL Integration (VP-011) — 2-3h

### Wave 2 (sequential, depends on Wave 1):
- **06-03**: Cross-Store Query Planner Integration (VP-010) — 8-10h
- **06-04**: Benchmarks, Validation & Quality Gates — 3-4h

---

## Key Files

**Modified (wiring):**
- `collection/search/query/mod.rs` — Add NEAR_FUSED + cross-store dispatch
- `collection/search/query/extraction.rs` — Extract NEAR_FUSED vectors
- `velesql/hybrid.rs` — Remove dead_code, wire into execution
- `velesql/planner.rs` — Wire into execute_query()

**New (tests):**
- `tests/near_fused_e2e.rs` — NEAR_FUSED end-to-end tests
- `tests/cross_store_query.rs` — Cross-store query tests
- `benches/bm25_trigram.rs` — Performance benchmarks

---

## Performance Targets

| Metric | Current | Target | Notes |
|--------|---------|--------|-------|
| Trigram vs LIKE scan (100K) | ~450ms | <20ms | 22x+ improvement |
| NEAR_FUSED 3 vectors | N/A | <10ms | Parallel batch search |
| Cross-store V+BM25 | N/A | <5ms | RRF fusion |

---
*Created: 2026-02-09*
*Updated: 2026-02-09 (post-research revision)*
*Phase: 6 of v4-verify-promise milestone*
