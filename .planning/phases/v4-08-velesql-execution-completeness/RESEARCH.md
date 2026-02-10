# Research: VelesQL Execution Completeness

**Date:** 2026-02-09
**Scope:** Identify all VelesQL features that parse but don't execute, design solutions

---

## Executive Summary

VelesQL's major value proposition is **unified syntax** across Vector + Graph + ColumnStore. The parser supports a rich SQL-like language, but three critical features **parse successfully then silently fail** at execution:

1. **JOIN** — parsed, execution module exists, never called
2. **UNION/INTERSECT/EXCEPT** — parsed into AST, zero execution code
3. **`/query/explain` endpoint** — handler exists, not routed in server

**Impact:** A user writing `SELECT * FROM docs JOIN metadata ON docs.id = metadata.doc_id` gets results **without the JOIN data**. This is a broken product experience.

---

## Feature Audit Matrix

### ✅ Fully Executing (parse → convert → execute → results)

| Feature | Parser | AST | Filter Conversion | Execution | Tests |
|---------|--------|-----|-------------------|-----------|-------|
| `LIKE` | ✅ | `LikeCondition` | `filter::Like` | `like_match()` | ✅ |
| `ILIKE` | ✅ | `LikeCondition(case_insensitive)` | `filter::ILike` | `like_match(true)` | ✅ |
| `BETWEEN` | ✅ | `BetweenCondition` | `filter::And(Gte, Lte)` | comparison | ✅ |
| `IN` | ✅ | `InCondition` | `filter::In` | `values_equal()` | ✅ |
| `IS NULL / IS NOT NULL` | ✅ | `IsNullCondition` | `filter::IsNull/IsNotNull` | null check | ✅ |
| `MATCH 'keyword'` (text) | ✅ | `MatchCondition` | `filter::Contains` | BM25 search | ✅ |
| `NEAR` (vector) | ✅ | `VectorSearch` | extracted | HNSW search | ✅ |
| `NEAR_FUSED` | ✅ | `VectorFusedSearch` | extracted | `multi_query_search()` | ✅ |
| `similarity()` | ✅ | `SimilarityCondition` | extracted | `filter_by_similarity()` | ✅ |
| `MATCH (graph)` | ✅ | `MatchClause` | — | `execute_match()` BFS/DFS | ✅ |
| `DISTINCT` | ✅ | `DistinctMode` | — | `apply_distinct()` | ✅ |
| `ORDER BY` | ✅ | `SelectOrderBy` | — | `apply_order_by()` | ✅ |
| `GROUP BY / HAVING` | ✅ | `GroupByClause` | — | `execute_aggregate()` | ✅ |
| Scalar subqueries | ✅ | `Subquery` | resolved pre-filter | `resolve_subqueries_in_condition()` | ✅ |
| `NOW() / INTERVAL` | ✅ | `TemporalExpr` | `to_epoch_seconds()` | numeric comparison | ✅ |
| `USING FUSION` | ✅ | `FusionClause` | — | `multi_query_search()` | ✅ |
| `WITH (ef_search=N)` | ✅ | `WithClause` | — | `search_with_ef()` | ✅ |
| Cross-store NEAR+MATCH | ✅ | — | — | VectorFirst/Parallel/GraphFirst | ✅ |
| Hybrid NEAR+BM25 | ✅ | — | — | `hybrid_search()` | ✅ |

### 🔴 Parse-Only (BROKEN — silently drops clauses)

| Feature | Parser | AST | Execution | Gap |
|---------|--------|-----|-----------|-----|
| `JOIN` | ✅ | `JoinClause` in `SelectStatement.joins` | `join.rs::execute_join()` exists | **Never called** from `execute_query()` |
| `UNION` | ✅ | `CompoundQuery` in `Query.compound` | None | **Zero execution code** |
| `INTERSECT` | ✅ | `CompoundQuery` | None | **Zero execution code** |
| `EXCEPT` | ✅ | `CompoundQuery` | None | **Zero execution code** |

### 🟡 Partial (works but not routed)

| Feature | Status | Gap |
|---------|--------|-----|
| `/query/explain` | Handler in `handlers/query.rs` | **Not routed** in `main.rs` |
| LEFT/RIGHT/FULL JOIN | `JoinType` enum parsed | `execute_join()` only does INNER JOIN |

---

## Architecture Analysis

### Current Flow

```
Server /query → parse → resolve FROM collection → collection.execute_query() → results
                                                    ↑
                                        Ignores stmt.joins
                                        Ignores query.compound
```

### Key Constraint

`Collection::execute_query()` has no access to other collections:
- **JOIN** needs a `ColumnStore` for the joined table → not available on `Collection`
- **Compound queries** need to execute a second SELECT on a different collection
- **Database** struct holds all collections via `HashMap<String, Collection>`
- **Server handler** has `state.db` (Database) but delegates directly to single Collection

### Solution: Database-Level Query Executor

```
Server /query → parse → Database::execute_query()
                              ├── resolve FROM collection
                              ├── collection.execute_query() (existing)
                              ├── POST: apply JOINs (resolve JOIN tables → ColumnStore)
                              └── POST: apply compound queries (execute second SELECT)
```

**New method: `Database::execute_query()`**
- Wraps `Collection::execute_query()` with cross-collection capabilities
- Resolves JOIN tables by name → builds ColumnStore from target collection's payloads
- Handles compound queries by executing second SELECT on its FROM collection
- Returns unified `Vec<SearchResult>`

### ColumnStore Resolution Strategy

For `JOIN metadata ON docs.id = metadata.doc_id`:
1. Look up `metadata` as a collection name in Database
2. Build a temporary ColumnStore from that collection's point payloads
3. Call existing `execute_join()` with the ColumnStore
4. Convert `JoinedResult` back to `SearchResult` via `joined_to_search_results()`

**Alternative for standalone ColumnStores:**
- Database could also hold named ColumnStores (future enhancement)
- For v1, collection-backed ColumnStore is sufficient

### Compound Query Execution

For `SELECT * FROM a UNION SELECT * FROM b`:
1. Execute first SELECT on collection `a` → results_a
2. Execute second SELECT on collection `b` → results_b
3. Apply set operation:
   - `UNION`: merge + deduplicate by `point.id`
   - `UNION ALL`: concatenate
   - `INTERSECT`: keep only IDs present in both
   - `EXCEPT`: remove IDs from results_b out of results_a

---

## Existing Infrastructure to Reuse

| Module | Location | Reuse |
|--------|----------|-------|
| `execute_join()` | `collection/search/query/join.rs` | Wire into post-processing |
| `joined_to_search_results()` | `collection/search/query/join.rs` | Convert JOIN results |
| `adaptive_batch_size()` | `collection/search/query/join.rs` | Batch lookup optimization |
| `extract_join_keys()` | `collection/search/query/join.rs` | Key extraction |
| `CompoundQuery` AST | `velesql/ast/mod.rs` | Already parsed |
| `SetOperator` enum | `velesql/ast/mod.rs` | Union/Intersect/Except/UnionAll |
| `explain()` handler | `handlers/query.rs` | Just needs routing |
| `Database::get_collection()` | `lib.rs` | Cross-collection access |
| `ColumnStore` | `column_store/mod.rs` | JOIN target storage |

---

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| ColumnStore from payloads may be slow for large collections | Medium | Lazy build, cache, limit JOIN to indexed columns |
| Compound query on same collection is trivial, cross-collection needs Database context | Low | Database executor handles both |
| LEFT/RIGHT/FULL JOIN semantics more complex than INNER | Medium | Start with INNER+LEFT (most common), extend later |
| Concurrent access to multiple collections during JOIN | Low | Collections are `Clone` (Arc-wrapped internals) |

---

## Estimates

| Plan | Scope | Estimate |
|------|-------|----------|
| 08-01 | Database-level query executor + collection-backed ColumnStore | 4-6h |
| 08-02 | JOIN execution integration (INNER + LEFT) | 4-5h |
| 08-03 | Compound query execution (UNION/INTERSECT/EXCEPT) | 3-4h |
| 08-04 | /query/explain route + server integration | 1-2h |
| 08-05 | E2E tests + documentation update | 3-4h |
| **Total** | | **15-21h** |

---

*Created: 2026-02-09*
