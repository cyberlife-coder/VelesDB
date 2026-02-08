# Roadmap — v4: Verify Promise

## Overview

**Project:** VelesDB Core
**Milestone:** v4-verify-promise
**Created:** 2026-02-08
**Phases:** 5
**Estimated effort:** ~40-50h

## Progress

```
Phase 1  ███████▓▓▓  75% — MATCH WHERE Completeness (reopened: VP-003 + VP-006)
Phase 2  ░░░░░░░░░░  0%  — Subquery Decision & Execution (3 plans ready)
Phase 3  ░░░░░░░░░░  0%  — Multi-hop MATCH & RETURN
Phase 4  ░░░░░░░░░░  0%  — E2E Scenario Test Suite
Phase 5  ░░░░░░░░░░  0%  — README & Documentation Truth
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
- [ ] ~~Temporal comparisons work in MATCH WHERE~~ → VP-003: `resolve_where_param` passes `Value::Temporal` unchanged, `evaluate_comparison` returns `false`
- [ ] ~~ORDER BY property path works in MATCH results~~ → VP-006: `order_match_results` exists but never called from `execute_match`/`execute_match_with_similarity`
- [x] No `_ => Ok(true)` catch-all remains — VectorSearch/FusedSearch pass-through only
- [x] Tests for each condition type in MATCH context (15 tests)

**Partially completed:** 2026-02-08 — Commit `dc9ac868` (VP-001 done, VP-003/VP-006 gaps found 2026-02-09)

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
- [ ] `Value::Subquery` no longer silently converts to Null
- [ ] Subqueries execute and return correct scalar values
- [ ] Correlated subqueries resolve outer row references
- [ ] Both MATCH WHERE and SELECT WHERE paths support subqueries
- [ ] Tests covering subquery execution for both paths

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
- [ ] Multi-hop patterns `(a)-[:R1]->(b)-[:R2]->(c)` correctly traverse two relationships
- [ ] Intermediate node bindings are populated (e.g., `b` is accessible in WHERE/RETURN)
- [ ] Variable-length paths produce results for each valid path length
- [ ] RETURN property projection works across all bound variables in multi-hop
- [ ] Basic aggregation (COUNT, AVG) in RETURN clause for MATCH results (or clear error)
- [ ] Tests for multi-hop traversal with 2 and 3 hops
- [ ] Tests for variable-length path `*1..3` with intermediate results

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
- [ ] Hero query: MATCH (doc)-[:AUTHORED_BY]->(author) WHERE similarity() + filter
- [ ] Scenario 0: Technical deep-dive (Vector + Graph + Column)
- [ ] Scenario 0b: NEAR_FUSED multi-vector fusion
- [ ] Scenario 0c: All 5 distance metrics
- [ ] Scenario 1: Medical Research (SELECT + NEAR + LIKE + date filter)
- [ ] Scenario 2: E-commerce (SELECT + NEAR + BETWEEN + multi ORDER BY)
- [ ] Scenario 3: Cybersecurity (SELECT + NEAR + temporal + comparison)
- [ ] Business Scenario 1: E-commerce Discovery (MATCH + similarity + filter)
- [ ] Business Scenario 2: Fraud Detection (MATCH + multi-hop + similarity)
- [ ] Business Scenario 3: Healthcare (MATCH + multi-relationship + aggregation)
- [ ] Business Scenario 4: AI Agent Memory (MATCH + temporal + ORDER BY)
- [ ] REST API examples: Create collection, upsert, search, VelesQL query

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

## Execution Order

```
Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5
                                  ↑
                    Phase 4 depends on 1-3 being done
```

Phase 1 is independent and highest-impact (silent incorrect results).
Phase 2 needs user decision on subquery approach.
Phase 3 builds on Phase 1 fixes.
Phase 4 validates all previous phases.
Phase 5 is the final documentation cleanup.

---

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| Subquery implementation too complex | Delays Phase 2 | Option B (error) as fallback |
| Multi-hop MATCH breaks existing queries | Regression | Extensive test suite before changes |
| README changes anger users | Trust | "In Progress" is better than "Fake" |
| Performance regression from new WHERE eval | Slowdown | Benchmark before/after |

---
*Last updated: 2026-02-08 — Phase 1 complete, Phase 2 planned (3 plans)*
