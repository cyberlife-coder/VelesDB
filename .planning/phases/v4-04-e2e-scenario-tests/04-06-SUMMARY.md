# Plan 04-06 Summary: Cross-Store & VelesQL API Validation

**Status:** ✅ Complete  
**Date:** 2026-02-09  
**Phase:** v4-04-e2e-scenario-tests  

## Tasks Completed

### Task 1: Scenario 0 — Technical Deep-Dive ✅

**Test A — MATCH + similarity + category filter:**
- Created collection with Document nodes (4 docs, 3 peer-reviewed) and Author nodes (4 authors)
- AUTHORED_BY edges linking documents to authors
- `start_properties` filter: `category = 'peer-reviewed'` (inline OpenCypher style)
- Similarity threshold 0.5 filters distant vectors (seed 500)
- Cross-node property projection verified (doc.title + author.name)
- Results ordered by similarity DESC

**Test B — Subquery in MATCH WHERE:**
- Non-correlated scalar subquery: `(SELECT AVG(citation_count) FROM scenario0)`
- AVG = 107.5 → filters to Prof. Johnson (120) and Prof. Expert (200)
- Subquery resolves via `resolve_subquery_value` in MATCH WHERE path
- **Documented limitation**: Correlated subqueries (`author.id` from MATCH bindings) not yet wired in MATCH WHERE path. Works in SELECT WHERE (VP-002).

### Task 2: VelesQL API Validation ✅

- **SELECT + NEAR + filter**: 4 articles, category='technology' filter, similarity DESC ordering
- **GROUP BY + HAVING**: 6 reviews, 3 categories, `HAVING COUNT(*) > 1` filters food (count=1)
- **UNION parsing**: Parser accepts UNION syntax, compound clause present. Execution not tested (multi-collection routing not implemented).

### Task 3: Manufacturing Quality Control ✅

- MATCH `(batch:Batch)-[:TESTED_WITH]->(test:QualityTest)`
- `start_properties`: material = 'steel'
- WHERE: `test.result = 'fail'`
- Returns 2 results: Tensile Strength + Impact Test (both steel batches with failures)
- Aluminum batch excluded by material filter, pass result excluded by WHERE

## Test Results

```
running 6 tests
test cross_store::test_velesql_union_parses ... ok
test cross_store::test_velesql_select_near_with_filter ... ok
test cross_store::test_velesql_group_by_having ... ok
test cross_store::test_manufacturing_qc_failed_steel_tests ... ok
test cross_store::test_scenario0_match_similarity_filter ... ok
test cross_store::test_scenario0_subquery_in_where ... ok

test result: ok. 6 passed; 0 failed; 0 ignored
```

## Files Modified

- `crates/velesdb-core/tests/readme_scenarios/cross_store.rs` — 6 tests (replaced stub)

## Success Criteria

- [x] Scenario 0 Test A: MATCH + similarity + filter + cross-node projection works
- [x] Scenario 0 Test B: Subquery in WHERE evaluates (non-correlated; limitation documented)
- [x] VelesQL basic SELECT + NEAR executes correctly
- [x] VelesQL GROUP BY/HAVING aggregation works
- [x] UNION parsing validated
- [x] Manufacturing QC MATCH example works

## Known Limitations

1. **Correlated subqueries in MATCH WHERE**: The `evaluate_where_condition` path calls `resolve_subquery_value` with `outer_row=None`, so correlated columns from MATCH bindings cannot be injected into subquery params. Non-correlated subqueries work fine.
2. **UNION execution**: Parser supports UNION syntax but core engine doesn't route execution across multiple collections yet. Parse-only validation.
