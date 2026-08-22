# Core: VelesQL reference

VelesQL is the SQL-like language that queries the three `velesdb-core` engines
— vector (HNSW), graph (typed edges, `MATCH`) and columnar metadata — from a
single statement. Parse with `velesdb_core::velesql::Parser` and execute with
`Database::execute_query`.

Moved out of `crates/velesdb-core/README.md` to keep that file under the
400-line documentation budget.

---

## Vector, text and hybrid search

The distance metric is **always** the one fixed at collection creation; a query
cannot override it.

```sql
-- Vector similarity search
SELECT * FROM docs WHERE VECTOR NEAR [0.1, 0.2, 0.3, 0.4] LIMIT 5;

-- With a bound parameter (API / prepared use)
SELECT * FROM docs WHERE VECTOR NEAR $query LIMIT 10;

-- Full-text search (BM25)
SELECT * FROM docs WHERE content MATCH 'rust programming' LIMIT 10;

-- Hybrid: vector + text
SELECT * FROM docs
WHERE VECTOR NEAR $query AND content MATCH 'rust'
LIMIT 5;
```

## Filtering on metadata

Payload fields are filtered with standard SQL operators, and combine freely
with the vector predicate.

```sql
-- Equality
SELECT * FROM docs WHERE category = 'tech' LIMIT 10;

-- Comparisons
SELECT * FROM docs WHERE views > 1000 LIMIT 10;
SELECT * FROM docs WHERE price >= 50 AND price <= 200 LIMIT 10;

-- String patterns
SELECT * FROM docs WHERE title LIKE '%rust%' LIMIT 10;

-- IN list
SELECT * FROM docs WHERE category IN ('tech', 'science', 'ai') LIMIT 10;

-- BETWEEN (inclusive)
SELECT * FROM docs WHERE score BETWEEN 0.5 AND 1.0 LIMIT 10;

-- NULL checks
SELECT * FROM docs WHERE author IS NOT NULL LIMIT 10;

-- Vector + metadata filters together
SELECT * FROM docs
WHERE VECTOR NEAR [0.1, 0.2, 0.3, 0.4]
AND category = 'tech'
AND views > 100
LIMIT 5;
```

### Available filter operators

| Operator | SQL syntax | Example |
|----------|------------|---------|
| Equal | `=` | `category = 'tech'` |
| Not equal | `!=` or `<>` | `status != 'draft'` |
| Greater than | `>` | `views > 1000` |
| Greater or equal | `>=` | `price >= 50` |
| Less than | `<` | `score < 0.5` |
| Less or equal | `<=` | `rating <= 3` |
| IN | `IN (...)` | `tag IN ('a', 'b')` |
| BETWEEN | `BETWEEN ... AND ...` | `age BETWEEN 18 AND 65` |
| LIKE | `LIKE` | `name LIKE '%john%'` |
| IS NULL | `IS NULL` | `email IS NULL` |
| IS NOT NULL | `IS NOT NULL` | `phone IS NOT NULL` |
| Full-text | `MATCH` | `content MATCH 'rust'` |

## `WITH` clause: per-query options

`WITH (...)` overrides search parameters for one statement only.

```sql
-- Pick a search mode
SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 10
WITH (mode = 'accurate');

-- Raise ef_search and cap the query time
SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 10
WITH (ef_search = 512, timeout_ms = 5000);
```

| Option | Type | Description |
|--------|------|-------------|
| `mode` | string | `fast`, `balanced`, `accurate`, `perfect`, `adaptive` |
| `ef_search` | integer | HNSW `ef_search` (higher = better recall, slower) |
| `timeout_ms` | integer | Query timeout in milliseconds |
| `rerank` | boolean | Enable result reranking |

Mode semantics and their recall/latency envelope are detailed in
[Search modes](./SEARCH_MODES.md).

## `JOIN` runtime limit

`JOIN ... USING (...)` supports **one column only**. Multi-column
`USING (a, b, ...)` parses, but the executor cannot resolve it: the join path
relies on a single primary-key lookup, so the condition resolves to nothing
(`crates/velesdb-core/src/collection/search/query/join.rs`). Use an explicit
`JOIN ... ON left = right` when you need more than one key column.

## `EXPLAIN`

`EXPLAIN` returns the query plan as JSON, including the plan-cache fields — see
[Core query plan cache](./CORE_QUERY_PLAN_CACHE.md) for the full output shape
and the `cache_hit` / `plan_reuse_count` semantics.

```sql
EXPLAIN SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 10;
```

## See also

- [velesdb-core README](../../crates/velesdb-core/README.md)
- [Multi-model queries](./MULTIMODEL_QUERIES.md) — vector + graph + metadata in one statement
- [Graph patterns](./GRAPH_PATTERNS.md) — `MATCH` recipes
- [Core collections and metrics](./CORE_COLLECTIONS_AND_METRICS.md)

---

Last updated: 2026-07-25 · Applies to: velesdb-core 5.2.0
