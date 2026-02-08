# Roadmap — v4: Verify Promise

## Overview

**Project:** VelesDB Core
**Milestone:** v4-verify-promise
**Created:** 2026-02-08
**Phases:** 5
**Estimated effort:** ~40-50h

## Progress

```
Phase 1  ██████████ 100% — MATCH WHERE Completeness ✅ (2026-02-08)
Phase 2  ██████████ 100% — Subquery Decision & Execution ✅ (2026-02-08)
Phase 3  ██████████ 100% — Multi-hop MATCH & RETURN ✅ (2026-02-08)
Phase 4  ██████████ 100% — E2E Scenario Test Suite ✅ (2026-02-09)
Phase 5  ░░░░░░░░░░  0%  — README & Documentation Truth (ready to plan)
Phase 6  ░░░░░░░░░░  0%  — Unified Query & Full-Text Search (planned)
```

---

## Phases

### Phase 1: MATCH WHERE Completeness

**Goal:** Every condition type that works in SELECT also works in MATCH WHERE.
**Requirements:** VP-001, VP-003, VP-006
**Estimate:** 8-10h

**Problem:** `where_eval.rs` has a `_ => Ok(true)` catch-all that silently accepts LIKE, BETWEEN, IN, IsNull, Match (full-text), and temporal conditions without evaluating them. This means MATCH queries with these conditions return **all nodes** instead of filtered results.

**Success Criteria:**
- [x] LIKE/ILIKE conditions evaluated in MATCH WHERE (pattern matching against payload)
- [x] BETWEEN conditions evaluated in MATCH WHERE (range comparison)
- [x] IN conditions evaluated in MATCH WHERE (set membership)
- [x] IsNull/IsNotNull conditions evaluated in MATCH WHERE
- [x] Match (full-text) conditions evaluated in MATCH WHERE
- [x] ~~Temporal comparisons work in MATCH WHERE~~ → VP-003: Fixed in Plan 01-02 (2026-02-08)
- [x] ~~ORDER BY property path works in MATCH results~~ → VP-006: Fixed in Plan 01-01 (2026-02-08)
- [x] No `_ => Ok(true)` catch-all remains — VectorSearch/FusedSearch pass-through only
- [x] Tests for each condition type in MATCH context (15 tests)

**Completed:** 2026-02-08 — Plans 01-01 (VP-006) and 01-02 (VP-003) resolved all gaps

**Reopened plans:**
- 01-01: Wire ORDER BY into MATCH execution pipeline (VP-006)
- 01-02: Wire Temporal resolution into MATCH WHERE comparison (VP-003)

**Key Files:**
- `crates/velesdb-core/src/collection/search/query/match_exec/where_eval.rs` — Main fix location
- `crates/velesdb-core/src/collection/search/query/match_exec/similarity.rs` — ORDER BY extension
- `crates/velesdb-core/src/collection/search/query/match_exec_tests.rs` — Tests

---

### Phase 2: Subquery Decision & Execution

**Goal:** Subqueries either execute correctly OR return a clear error — no more silent Value::Null.
**Requirements:** VP-002
**Estimate:** 10-12h

**Problem:** The parser successfully parses `(SELECT price FROM inventory WHERE sku = product.sku)` into a `Subquery` AST node with correlation info. But:
- `filter/conversion.rs` converts `Value::Subquery(_)` → `Value::Null` (silent data loss)
- No executor exists for subqueries
- ALL 4 business scenarios in the README rely on subqueries

**Decision: Option A — Implement scalar subqueries** (user confirmed 2026-02-08)

Execution reuses `Collection::execute_query()` for inner SELECT, extracts scalar from first result.

**Plans:**
- 02-01: Core Scalar Subquery Executor (Wave 1)
- 02-02: Wire Subquery into MATCH WHERE Path (Wave 2)
- 02-03: Wire Subquery into SELECT WHERE Path + Quality Gates (Wave 2)

**Success Criteria:**
- [x] `Value::Subquery` no longer silently converts to Null
- [x] Subqueries execute and return correct scalar values
- [x] Correlated subqueries resolve outer row references
- [x] Both MATCH WHERE and SELECT WHERE paths support subqueries
- [x] Tests covering subquery execution for both paths (12 tests)

**Completed:** 2026-02-08 — Plans 02-01, 02-02, 02-03 all done

**Key Files:**
- `crates/velesdb-core/src/filter/conversion.rs` — Remove Null conversion
- `crates/velesdb-core/src/collection/search/query/` — New subquery executor (if Option A/C)
- `crates/velesdb-core/src/velesql/ast/values.rs` — Subquery AST types
- `crates/velesdb-core/src/velesql/error.rs` — New error variant

---

### Phase 3: Multi-hop MATCH & RETURN Enhancement

**Goal:** Multi-relationship MATCH patterns traverse correctly with proper binding propagation.
**Requirements:** VP-004, VP-005
**Estimate:** 10-12h

**Problem:** 
1. MATCH patterns like `(a)-[:R1]->(b)-[:R2]->(c)` only use the first pattern's relationships for BFS depth. The intermediate node `b` isn't properly bound, and `c` isn't reached through the correct relationship chain.
2. RETURN clause with aggregation (AVG, COUNT, SUM) isn't implemented for MATCH results.
3. Variable-length paths `*1..3` parse but only set BFS depth — they don't produce per-hop bindings.

**Success Criteria:**
- [x] Multi-hop patterns `(a)-[:R1]->(b)-[:R2]->(c)` correctly traverse two relationships
- [x] Intermediate node bindings are populated (e.g., `b` is accessible in WHERE/RETURN)
- [x] Variable-length paths produce results for each valid path length
- [x] RETURN property projection works across all bound variables in multi-hop
- [x] Basic aggregation (COUNT, AVG) in RETURN clause for MATCH results
- [x] Tests for multi-hop traversal with 2 and 3 hops
- [x] Tests for variable-length path `*1..3` with intermediate results (10 tests)

**Completed:** 2026-02-08 — Plans 03-01 (multi-hop chain) and 03-02 (RETURN aggregation) done

**Key Files:**
- `crates/velesdb-core/src/collection/search/query/match_exec/mod.rs` — Multi-hop execution
- `crates/velesdb-core/src/velesql/graph_pattern.rs` — Pattern types
- `crates/velesdb-core/src/collection/search/query/match_exec/similarity.rs` — RETURN enhancement

---

### Phase 4: E2E Scenario Test Suite

**Goal:** Every README code example has a corresponding test that proves it works.
**Requirements:** VP-007
**Estimate:** 8-10h

**Creates a test suite in `tests/readme_scenarios.rs`** that:
1. Sets up test data matching each README scenario
2. Executes the exact query from the README
3. Validates results match the documented expected output
4. Runs as part of `cargo test --workspace`

**Scenarios to cover:**
- [x] Hero query: MATCH (doc)-[:AUTHORED_BY]->(author) WHERE similarity() + filter (3 tests)
- [x] Scenario 0: Technical deep-dive (Vector + Graph + Column) (3 tests)
- [x] Scenario 0b: NEAR_FUSED multi-vector fusion (4 fusion strategies + parse test)
- [x] Scenario 0c: All 5 distance metrics (5 tests: cosine, euclidean, dotproduct, hamming, jaccard)
- [x] Scenario 1: Medical Research (SELECT + NEAR + LIKE + date filter)
- [x] Scenario 2: E-commerce (SELECT + NEAR + BETWEEN + multi ORDER BY)
- [x] Scenario 3: Cybersecurity (SELECT + NEAR + temporal + comparison)
- [x] Business Scenario 1: E-commerce Discovery (MATCH + similarity + filter, 3 tests)
- [x] Business Scenario 2: Fraud Detection (MATCH + multi-hop + similarity)
- [x] Business Scenario 3: Healthcare (MATCH + multi-relationship + aggregation, 2 tests)
- [x] Business Scenario 4: AI Agent Memory (MATCH + temporal + ORDER BY, 4 tests)
- [x] VelesQL API: GROUP BY/HAVING, UNION, SELECT NEAR, subquery (4 tests)

**36 tests total across 7 test files. All passing.**

**Completed:** 2026-02-09 — 7 plans (04-01 through 04-07) across 3 waves

**Key Files:**
- `tests/readme_scenarios.rs` — New comprehensive test file
- `crates/velesdb-core/tests/` — Additional integration tests

---

### Phase 5: README & Documentation Truth

**Goal:** Every claim in README, website, and screenshots matches actual capability.
**Requirements:** VP-008, VP-009
**Estimate:** 4-6h

**Audit and fix:**
1. **README.md** — Mark unimplemented features, fix query examples, add caveats
2. **Website screenshots** — Verify code examples execute correctly
3. **Performance claims** — Verify numbers are reproducible
4. **Ecosystem table** — Verify each component exists and basic functionality works

**Success Criteria:**
- [ ] Every SQL query in README has been tested (Phase 4 ensures this)
- [ ] Business scenarios use queries that actually execute correctly
- [ ] "Coming Soon" labels on features that don't work yet
- [ ] Performance numbers verified against latest benchmarks
- [ ] Ecosystem table accurate (no phantom packages)
- [ ] Website screenshot code examples verified
- [ ] VELESQL_SPEC.md or equivalent updated with actual support status

**Key Files:**
- `README.md` — Documentation corrections
- `docs/VELESQL_SPEC.md` — VelesQL feature support matrix (create if missing)

---

### Phase 6: Unified Cross-Store Query Engine & Full-Text Search

**Goal:** Implement and validate unified query capabilities claimed on velesdb.com: seamless combination of vector (NEAR), graph (MATCH), and full-text (BM25) in single queries.

**Requirements:** VP-010, VP-011, VP-012  
**Estimate:** 20-25h  
**Priority:** 🚨 Critical — Site claims features requiring deeper implementation

**Problem:**
The velesdb.com website showcases three capabilities needing implementation:

1. **Cross-Store Unified Queries** — `WHERE vector NEAR $v AND MATCH (a)-[:KNOWS]->(b)` — No unified query planner exists
2. **BM25 Full-Text** — "Trigram Index 22-128x faster" — No BM25/trigram implementation exists
3. **NEAR_FUSED Execution** — Multi-vector fusion at runtime — Parsing works but full execution not validated

**Success Criteria:**
- [ ] `NEAR` + `MATCH` + column filters in single WHERE clause executes correctly
- [ ] BM25 trigram index: 25x faster than LIKE scan (2ms vs 50ms on 100K docs)
- [ ] `MATCH` operator for full-text search with BM25 scoring
- [ ] NEAR_FUSED multi-vector execution with all fusion strategies validated
- [ ] Cross-store query planner optimizes execution order

**Plans:**
- Wave 1: BM25 Foundation (06-01 → 06-03)
- Wave 2: Cross-Store Query Engine (06-04 → 06-06)
- Wave 3: NEAR_FUSED & Integration (06-07 → 06-09)

**Key Files:**
- `crates/velesdb-core/src/index/trigram/` — New full-text indexing module
- `crates/velesdb-core/src/query/unified/` — New cross-store execution
- `crates/velesdb-core/src/velesql/execution.rs` — Add unified query path

---

## Execution Order

```
Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5
                                  ↑
                    Phase 4 depends on 1-3 being done
                    
Phase 6 (Unified Query) can parallelize with Phase 5
or execute after depending on priority.
```

Phase 1 is independent and highest-impact (silent incorrect results).
Phase 2 needs user decision on subquery approach.
Phase 3 builds on Phase 1 fixes.
Phase 4 validates all previous phases.
Phase 5 is the final documentation cleanup.
Phase 6 implements advanced unified capabilities claimed on site.

---

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| Subquery implementation too complex | Delays Phase 2 | Option B (error) as fallback |
| Multi-hop MATCH breaks existing queries | Regression | Extensive test suite before changes |
| README changes anger users | Trust | "In Progress" is better than "Fake" |
| BM25 trigram index performance below claim | Site credibility | Benchmark early, adjust claim to reality |
| Cross-store query complexity | Delays Phase 6 | Start with 2-store queries, expand |
| NEAR_FUSED fusion overhead too high | Performance | Profile before optimizing claims |
| Performance regression from new WHERE eval | Slowdown | Benchmark before/after |

---
*Last updated: 2026-02-09 — Phases 1-4 complete. Phases 5-6 ready to plan. 3,222 tests passing.*
