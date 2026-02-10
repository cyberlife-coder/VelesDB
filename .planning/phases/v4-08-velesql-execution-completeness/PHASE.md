# Phase 8: VelesQL Execution Completeness

**Milestone:** v4-verify-promise
**Requirements:** VP-014 (JOIN execution), VP-015 (compound queries), VP-013 (EXPLAIN route)
**Estimate:** 15-21h
**Priority:** 🚨 **Critical** — Core value proposition broken (unified syntax parses but silently drops clauses)
**Depends on:** Phases 6-7 ✅ (NEAR_FUSED, cross-store, BM25 already wired)

---

## Goal

**Make every VelesQL syntax that parses also execute correctly.** A user should never write a valid query and get silently incomplete results.

Three features currently parse but silently fail:
1. **JOIN** — `stmt.joins` ignored by `execute_query()`
2. **UNION/INTERSECT/EXCEPT** — `query.compound` ignored by `execute_query()`
3. **`/query/explain`** — handler exists but endpoint not routed

---

## Current State

| Feature | Parser | AST | Execution Module | Wired to execute_query? |
|---------|--------|-----|-----------------|------------------------|
| **JOIN** | ✅ | `SelectStatement.joins: Vec<JoinClause>` | `join.rs::execute_join()` ✅ | ❌ **Never called** |
| **UNION/INTERSECT/EXCEPT** | ✅ | `Query.compound: Option<CompoundQuery>` | None | ❌ **Zero code** |
| **`/query/explain`** | N/A | N/A | `handlers/query.rs::explain()` ✅ | ❌ **Not routed** |
| **LEFT/RIGHT/FULL JOIN** | ✅ | `JoinType` enum | `execute_join()` = INNER only | ❌ **Partial** |

## Architecture Decision

**Problem:** `Collection::execute_query()` cannot access other collections for JOIN/compound queries.

**Solution:** Add `Database::execute_query()` that:
1. Resolves FROM collection name
2. Delegates to `Collection::execute_query()` (existing, unchanged)
3. Post-processes JOINs: resolves JOIN table → builds ColumnStore from collection payloads → calls `execute_join()`
4. Post-processes compound queries: executes second SELECT on its FROM collection → applies set operation
5. Server `/query` handler calls `Database::execute_query()` instead of `Collection::execute_query()`

See `RESEARCH.md` for full analysis.

---

## Plans (5 plans, 2 waves)

### Wave 1 (foundation — sequential):
| Plan | Title | Scope | Estimate |
|------|-------|-------|----------|
| 08-01 | Database Query Executor & ColumnStore Builder | `Database::execute_query()`, collection→ColumnStore builder, trait for testability | 4-6h |
| 08-02 | JOIN Execution Integration | Wire `stmt.joins` via Database executor, INNER + LEFT JOIN, TDD | 4-5h |
| 08-03 | Compound Query Execution | UNION/UNION ALL/INTERSECT/EXCEPT via Database executor, TDD | 3-4h |

### Wave 2 (polish — can be parallel):
| Plan | Title | Scope | Estimate |
|------|-------|-------|----------|
| 08-04 | /query/explain Route & Server Integration | Route endpoint, update /query handler, integration tests | 1-2h |
| 08-05 | E2E Tests & Documentation Update | README/VELESQL_SPEC update, remove parser-only warnings, E2E scenarios | 3-4h |

---

## Success Criteria

### VP-014: JOIN Execution
- [ ] `SELECT * FROM docs JOIN metadata ON docs.id = metadata.doc_id` returns merged results
- [ ] INNER JOIN: only matching rows returned
- [ ] LEFT JOIN: all left rows, NULL for non-matching right
- [ ] JOIN with WHERE clause filters applied correctly
- [ ] JOIN with ORDER BY + LIMIT works
- [ ] Error returned if JOIN table doesn't exist

### VP-015: Compound Query Execution
- [ ] `SELECT * FROM a UNION SELECT * FROM b` merges and deduplicates
- [ ] `UNION ALL` merges without dedup
- [ ] `INTERSECT` returns only common results
- [ ] `EXCEPT` subtracts second from first
- [ ] Cross-collection compound queries work
- [ ] Same-collection compound queries work

### VP-013: EXPLAIN Completeness
- [ ] `POST /query/explain` returns 200 with plan
- [ ] Swagger UI shows the endpoint
- [ ] EXPLAIN shows JOIN steps when present
- [ ] EXPLAIN shows compound query steps when present

### Quality Gates
- [ ] `cargo fmt --all --check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] `cargo deny check` passes
- [ ] `cargo build --release` passes
- [ ] All new code has tests (TDD)
- [ ] No file > 300 lines

---

## Key Files

**New:**
- `crates/velesdb-core/src/database_query.rs` — Database-level query executor
- `crates/velesdb-core/src/database_query_tests.rs` — Tests
- `crates/velesdb-core/src/column_store/from_collection.rs` — ColumnStore builder from collection payloads

**Modified:**
- `crates/velesdb-core/src/lib.rs` — Add `Database::execute_query()` public API
- `crates/velesdb-core/src/collection/search/query/join.rs` — Remove `#[allow(dead_code)]`, add LEFT JOIN
- `crates/velesdb-server/src/main.rs` — Route `/query/explain`
- `crates/velesdb-server/src/handlers/query.rs` — Use `Database::execute_query()` for JOIN/compound
- `docs/VELESQL_SPEC.md` — Remove parser-only warnings
- `README.md` — Remove parser-only labels

---

*Created: 2026-02-09*
*Phase: 8 of v4-verify-promise milestone*
