# VelesQL Parser-Executor Conformance Matrix

This document lists every VelesQL feature with its parser and executor status.
A feature can be **Parsed** (the grammar + AST accept it) without being
**Executed** (the query engine acts on it at runtime).

> Last updated: 2026-03-15 (v1.6.0)

## Fully Supported (Parsed AND Executed)

| Feature | Parser source | Executor source | Notes |
|---------|--------------|-----------------|-------|
| `SELECT *` / named columns | `grammar.pest:select_list` | `query_engine.rs` | Full support |
| `SELECT DISTINCT` | `grammar.pest:distinct_kw` | `query_engine.rs` | EPIC-052 US-001 |
| `FROM <collection>` | `grammar.pest:from_clause` | `query_engine.rs` | Single collection |
| `FROM <collection> AS <alias>` | `grammar.pest:from_clause` | `query_engine.rs` | BUG-8 fix |
| `WHERE <condition>` | `grammar.pest:where_clause` | `search/query/` | Equality, comparison, logical |
| `WHERE vector NEAR $v` | `grammar.pest:near_condition` | `search/query/planner.rs` | kNN via HNSW |
| `WHERE vector SPARSE_NEAR $sv` | `grammar.pest:sparse_near` | `search/query/hybrid_sparse.rs` | SPLADE/BM42 |
| `WHERE content MATCH 'term'` | `grammar.pest:match_condition` | `search/query/planner.rs` | BM25 full-text |
| `ORDER BY field [ASC\|DESC]` | `ast/select.rs:SelectOrderBy` | `query_engine.rs` | Field + similarity() |
| `ORDER BY similarity()` | `ast/select.rs:SimilarityOrderBy` | `query_engine.rs` | EPIC-051 |
| `LIMIT n` | `grammar.pest:limit_clause` | `query_engine.rs` | |
| `OFFSET n` | `grammar.pest:offset_clause` | `query_engine.rs` | |
| `INNER JOIN` | `ast/join.rs:JoinType::Inner` | `search/query/join.rs` | EPIC-031 US-004 |
| `LEFT JOIN` | `ast/join.rs:JoinType::Left` | `search/query/join.rs:192-226` | EPIC-031 US-004 |
| `RIGHT JOIN` | `ast/join.rs:JoinType::Right` | `search/query/join.rs` | EPIC-031 US-004 |
| `FULL JOIN` | `ast/join.rs:JoinType::Full` | `search/query/join.rs` | EPIC-031 US-004 |
| `GROUP BY` | `ast/aggregation.rs:GroupByClause` | `velesql/aggregator.rs` | |
| `HAVING` | `ast/aggregation.rs:HavingClause` | `velesql/aggregator.rs` | |
| Aggregate functions | `ast/aggregation.rs` | `velesql/aggregator.rs` | COUNT, SUM, AVG, MIN, MAX |
| `USING FUSION(strategy=...)` | `ast/fusion.rs:FusionClause` | `search/query/hybrid_sparse.rs` | RRF, RSF |
| `WITH (key=value)` hints | `ast/with_clause.rs:WithClause` | `query_engine.rs` | ef_search, mode, quantization |
| `TRAIN QUANTIZER ON <coll>` | `ast/train.rs` | `database/training.rs` | PQ training |
| `MATCH (a)-[r]->(b)` | `ast/mod.rs:MatchClause` | `search/query/match_exec.rs` | Graph traversal |
| `similarity()` function | `grammar.pest:similarity_fn` | `search/query/` | In WHERE + ORDER BY |
| `IN (list)` | `grammar.pest:in_condition` | `search/query/where_eval.rs` | Value list matching |
| `BETWEEN x AND y` | `grammar.pest:between_condition` | `search/query/where_eval.rs` | Range filtering |
| `LIKE` / `ILIKE` | `grammar.pest:like_condition` | `search/query/where_eval.rs` | Pattern matching |
| `IS NULL` / `IS NOT NULL` | `grammar.pest:is_null_condition` | `search/query/where_eval.rs` | Null checks |
| `NOW()` | `grammar.pest:temporal_expr` | `search/query/where_eval.rs` | EPIC-038; current Unix timestamp |
| `INTERVAL` arithmetic | `ast/values.rs:IntervalValue` | `search/query/where_eval.rs` | EPIC-038; seconds, minutes, hours, days, weeks, months |
| `INSERT INTO` | `ast/dml.rs:InsertStatement` | `database/query_engine.rs` | DML |
| `UPDATE ... SET` | `ast/dml.rs:UpdateStatement` | `database/query_engine.rs` | DML |
| `DELETE FROM` | `ast/dml.rs:DeleteStatement` | `database/query_engine.rs` | DML |
| `UNION` / `UNION ALL` | `grammar.pest:set_operator`, `ast/mod.rs:CompoundQuery` | `search/query/set_operations.rs`, `database/query_engine.rs` | EPIC-040 US-006; dedup by point ID (UNION) or keep all (UNION ALL) |
| `INTERSECT` | `grammar.pest:set_operator`, `ast/mod.rs:SetOperator::Intersect` | `search/query/set_operations.rs`, `database/query_engine.rs` | EPIC-040 US-006; keep only common point IDs |
| `EXCEPT` | `grammar.pest:set_operator`, `ast/mod.rs:SetOperator::Except` | `search/query/set_operations.rs`, `database/query_engine.rs` | EPIC-040 US-006; remove right-side IDs from left |

## Parsed but NOT Executed

These features have grammar rules and AST representations but no runtime
execution path. Queries using them will parse successfully but produce
incorrect or empty results at execution time.

| Feature | Parser source | Status | Notes |
|---------|--------------|--------|-------|
| Scalar subqueries | `grammar.pest:subquery_expr`, `ast/values.rs:Subquery` | Parsed only | EPIC-039; no executor support |

## Not Parsed (Not in Grammar)

These SQL features are **not** in the VelesQL grammar and will produce
parse errors.

| Feature | Notes |
|---------|-------|
| Window functions (`OVER`, `PARTITION BY`, `ROW_NUMBER()`) | Not in grammar or AST |
| CTEs (`WITH name AS (SELECT ...)`) | VelesQL `WITH` is for query hints, not CTEs |
| Subqueries in `FROM` clause | Only collection names allowed in `FROM` |
| `CASE WHEN ... THEN ... END` | Not in grammar |
| `EXISTS` / `IN (subquery)` | Not in grammar |
| `CREATE TABLE` / DDL | Collections managed via API, not SQL DDL |
| Stored procedures / functions | Not applicable |

## Conformance Test Coverage

- **Parser conformance cases**: 140 cases in `conformance/velesql_parser_cases.json`
- **Cross-crate tests**: `velesql_parser_conformance` integration test runs in each crate
- **Join tests**: `search/query/join_tests.rs` (374-435)
- **Set operation parse tests**: `velesql/set_operations_tests.rs`
- **Set operation execution tests**: `collection/set_operations_execution_tests.rs`, `search/query/set_operations.rs` (unit tests)
- **Aggregation tests**: `velesql/aggregation_tests.rs`
- **DML tests**: `velesql/dml_tests.rs`
- **MATCH tests**: `search/query/match_exec_tests.rs`
