# Phase 8 Verification Checklist

## Pre-Execution Verification

- [ ] Plans 08-01 through 08-05 reviewed and approved
- [ ] No conflicts with ongoing work in other phases
- [ ] Phase 6 and Phase 7 completed (NEAR_FUSED, cross-store, BM25 wired)

---

## Plan 08-01: Database Query Executor & ColumnStore Builder

### Tests
- [ ] `test_column_store_from_empty_collection`
- [ ] `test_column_store_from_collection_with_payloads`
- [ ] `test_column_store_primary_key_is_point_id`
- [ ] `test_column_store_max_rows_limit`
- [ ] `test_column_store_mixed_types_inferred`
- [ ] `test_database_execute_query_basic_select`
- [ ] `test_database_execute_query_collection_not_found`
- [ ] `test_database_execute_query_with_params`
- [ ] `test_database_execute_query_aggregation`

### Validation
```powershell
cargo test --lib column_store::from_collection
cargo test --lib database_query
cargo clippy -- -D warnings
```

---

## Plan 08-02: JOIN Execution Integration

### Tests
- [ ] `test_left_join_keeps_all_left_rows`
- [ ] `test_left_join_merges_matching_rows`
- [ ] `test_inner_join_filters_non_matching`
- [ ] `test_right_join_returns_error`
- [ ] `test_full_join_returns_error`
- [ ] `test_database_join_two_collections`
- [ ] `test_database_join_collection_not_found`
- [ ] `test_database_left_join`
- [ ] `test_database_join_with_where_filter`
- [ ] `test_database_join_with_order_by`
- [ ] `test_database_multiple_joins`
- [ ] `test_database_join_with_vector_search`

### Validation
```powershell
cargo test --lib join
cargo test --lib database_query
cargo clippy -- -D warnings
```

### Functional Verification
```sql
-- This query must return merged results (not silently drop JOIN):
SELECT * FROM docs JOIN metadata ON docs.id = metadata.doc_id LIMIT 10
```

---

## Plan 08-03: Compound Query Execution

### Tests
- [ ] `test_union_deduplicates`
- [ ] `test_union_merges_different_ids`
- [ ] `test_union_all_keeps_duplicates`
- [ ] `test_intersect_keeps_common`
- [ ] `test_intersect_disjoint_returns_empty`
- [ ] `test_except_removes_right_from_left`
- [ ] `test_except_with_no_overlap`
- [ ] `test_empty_left_set`
- [ ] `test_empty_right_set`
- [ ] `test_database_union_two_collections`
- [ ] `test_database_union_all`
- [ ] `test_database_intersect`
- [ ] `test_database_except`
- [ ] `test_database_union_same_collection`
- [ ] `test_database_compound_collection_not_found`
- [ ] `test_database_compound_with_order_by`

### Validation
```powershell
cargo test --lib compound
cargo test --lib database_query
cargo clippy -- -D warnings
```

### Functional Verification
```sql
-- These queries must return combined results:
SELECT * FROM docs_a UNION SELECT * FROM docs_b
SELECT * FROM docs WHERE tag = 'a' INTERSECT SELECT * FROM docs WHERE tag = 'b'
```

---

## Plan 08-04: /query/explain Route & Server Integration

### Tests
- [ ] `POST /query/explain` returns 200
- [ ] Swagger UI shows endpoint
- [ ] `/query` handler uses `Database::execute_query()`
- [ ] Existing server tests pass (no regression)

### Validation
```powershell
cargo test -p velesdb-server
cargo clippy -p velesdb-server -- -D warnings
```

---

## Plan 08-05: E2E Tests & Documentation Update

### Tests
- [ ] E2E JOIN scenarios (5+)
- [ ] E2E compound query scenarios (5+)
- [ ] E2E EXPLAIN scenarios (4+)

### Documentation
- [ ] `VELESQL_SPEC.md` — no "parser only" warnings for JOIN/Set Ops
- [ ] `README.md` — no "parser only" labels for JOIN/Set Ops
- [ ] `README.md` — `/query/explain` in API table
- [ ] `GAPS.md` — resolved gaps marked

### Quality Gates (Final)
```powershell
.\scripts\local-ci.ps1
# Or manually:
cargo fmt --all --check
cargo clippy -- -D warnings
cargo test --workspace
cargo deny check
cargo build --release
```

---

## Post-Phase Metrics

| Metric | Before | Target |
|--------|--------|--------|
| Parse-only features | 4 (JOIN, UNION, INTERSECT, EXCEPT) | 0 |
| Routed endpoints | /query only | /query + /query/explain |
| JOIN types supported | 0 (dead code) | 2 (INNER, LEFT) |
| Set operations supported | 0 | 4 (UNION, UNION ALL, INTERSECT, EXCEPT) |
| New tests | — | 45+ |

---
*Created: 2026-02-09*
