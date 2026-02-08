# Phase 6: Unified Cross-Store Query Engine & Full-Text Search

## Overview

**Goal:** Implement and validate the unified query capabilities claimed on velesdb.com: seamless combination of vector search (NEAR), graph traversal (MATCH), and full-text search (BM25/Trigram) in single queries.

**Requirements:** VP-010, VP-011, VP-012
**Estimate:** 20-25h
**Priority:** 🚨 Critical — Site claims features not yet fully implemented

---

## Problem Statement

The velesdb.com website showcases three advanced capabilities that require deeper implementation:

### 1. Cross-Store Unified Queries
Site claim: `WHERE vector NEAR $v AND MATCH (a)-[:KNOWS]->(b)`
- Current: Each store (Vector, Graph, Column) works independently
- Gap: No unified query planner that coordinates execution across stores
- Impact: User expects single query, gets coordination complexity

### 2. BM25 Full-Text / Trigram Index
Site claim: "Hybrid Search combine BM25 + vector + graph" and "Trigram Index 22-128x faster"
- Current: No BM25 implementation, no trigram indexing
- Gap: Full-text search falls back to slow LIKE scans
- Impact: "Fast full-text" claim is misleading

### 3. NEAR_FUSED Multi-Vector Execution
Site claim: `WHERE vector NEAR_FUSED [$v1, $v2] USING FUSION 'rrf'`
- Current: Parsing works, but full multi-vector execution not validated
- Gap: Need to verify actual multi-query execution and fusion at runtime
- Impact: Feature may be partially implemented

---

## Research Insights

### BM25 Implementation Approaches (2024-2025 State of Art)

**PostgreSQL Ecosystem:**
- `pg_textsearch` (Tiger Data): Native BM25 in Postgres — 2025 release
- `VectorChord-bm25`: Block-WeakAnd algorithm for ranking
- Pattern: Store inverted index + document frequency tables

**Implementation Strategy for VelesDB:**
```
TrigramIndex (new module)
├── trigram_tokenizer.rs    — Split text into 3-char tokens
├── inverted_index.rs       — token → Vec<doc_id, frequency>
├── bm25_scorer.rs          — Calculate BM25 scores
└── query_executor.rs       — AND/OR trigram intersection
```

**Performance Target:**
- Claim: 22-128x faster than LIKE scan
- Baseline: LIKE scan on 100K docs = ~50ms
- Target: Trigram BM25 = <2ms (25x improvement)

### Unified Query Architecture

**Pattern from Research:**
1. Query Planner analyzes cross-store predicates
2. Executes store-specific queries in parallel (where possible)
3. Joins results on unified ID space
4. Applies post-filters for cross-store conditions

**VelesDB Architecture:**
```
UnifiedQueryExecutor
├── Parse: Extract NEAR, MATCH, BM25, WHERE clauses
├── Plan: Determine execution order (cost-based)
├── Execute Phase 1: Parallel vector + graph + text search
├── Intersect: Find common IDs across results
├── Execute Phase 2: Fetch full records from ColumnStore
└── Sort/Rank: Apply fusion strategies
```

---

## Success Criteria

### Cross-Store Queries (VP-010)
- [ ] `NEAR` + `MATCH` in same WHERE clause executes correctly
- [ ] `NEAR` + column filters in same WHERE clause works
- [ ] `MATCH` + column filters in same WHERE clause works
- [ ] All three combined: `NEAR` + `MATCH` + column filters
- [ ] Query planner optimizes execution order
- [ ] Benchmark: Cross-store query < 5ms for 10K vectors + 1K graph edges

### BM25 Full-Text Search (VP-011)
- [ ] Trigram tokenizer implemented (UTF-8 safe)
- [ ] Inverted index for trigrams → document IDs
- [ ] BM25 scoring with k1/b parameters tunable
- [ ] `MATCH` operator for full-text search
- [ ] Hybrid query: `MATCH text 'keyword' AND vector NEAR $v`
- [ ] Benchmark: 25x faster than LIKE scan (2ms vs 50ms on 100K docs)

### NEAR_FUSED Complete Implementation (VP-012)
- [ ] Multi-vector query execution validated end-to-end
- [ ] Fusion strategies tested: RRF, Average, Maximum, Weighted
- [ ] Memory-efficient batched execution
- [ ] Results include fusion metadata (source contributions)
- [ ] Benchmark: Fusion overhead < 20% vs single vector

---

## Plans (to be detailed)

### Wave 1: BM25 Foundation
- **06-01**: Trigram Tokenizer & Inverted Index
- **06-02**: BM25 Scoring Engine
- **06-03**: Full-Text MATCH Operator

### Wave 2: Cross-Store Query Engine
- **06-04**: Unified Query Planner
- **06-05**: Cross-Store Execution Coordinator
- **06-06**: Parallel Store Execution

### Wave 3: NEAR_FUSED & Integration
- **06-07**: NEAR_FUSED Full Execution
- **06-08**: Fusion Strategy Optimization
- **06-09**: E2E Tests & Benchmarks

---

## Key Files (to create/modify)

**New Modules:**
- `crates/velesdb-core/src/index/trigram/` — Full-text indexing
- `crates/velesdb-core/src/query/unified/` — Cross-store execution

**Modified:**
- `crates/velesdb-core/src/velesql/execution.rs` — Add unified path
- `crates/velesdb-core/src/collection/search/` — Coordinate across stores

---

## Performance Targets

| Metric | Current (LIKE) | Target (BM25) | Gain |
|--------|----------------|---------------|------|
| Full-text search 100K docs | 50ms | 2ms | **25x** |
| Cross-store query (V+G+C) | N/A | <5ms | New capability |
| NEAR_FUSED 3 vectors | N/A | <10ms | New capability |

---

## Notes

- Site claims "Trigram Index 22-128x faster" — target 25x as realistic baseline
- BM25 formula: `score = idf * (f * (k1 + 1)) / (f + k1 * (1 - b + b * dl/avgdl))`
- Cross-store joins leverage VelesDB's unified ID space (node_id = vector_id = row_id)

---
*Created: 2026-02-09*
*Phase: 6 of v4-verify-promise milestone*
