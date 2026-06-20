# VelesQL Cheat Sheet

> **Date:** 2026-06-03
> **VelesDB version:** 3.2.1
> See the full specification in [`docs/VELESQL_SPEC.md`](../VELESQL_SPEC.md).

> Every `sql` block below is **parsed** against the real grammar by an automated
> test (`crates/velesdb-core/tests/velesql_cheatsheet_docs.rs`) — if an example
> stops parsing, CI fails. (The test guards syntax, not runtime execution.)
> Vectors are passed as bind parameters (`$v`); inline vector literals like
> `[0.1, 0.2, 0.3]` are also accepted in `WHERE ... NEAR`.

---

## DDL

```sql
-- Create a vector collection (dimension is required; metric defaults to cosine)
CREATE COLLECTION docs (dimension = 768);
CREATE COLLECTION docs (dimension = 768, metric = 'cosine');

-- Tune storage / HNSW at creation time
CREATE COLLECTION docs (dimension = 768, metric = 'cosine') WITH (storage = 'sq8');

-- Graph and metadata-only collections
CREATE GRAPH COLLECTION kg (dimension = 768, metric = 'cosine') SCHEMALESS;
CREATE METADATA COLLECTION tags;

-- Secondary index on a payload field
CREATE INDEX ON docs (category);
DROP INDEX ON docs (category);

-- Drop a collection
DROP COLLECTION docs;
DROP COLLECTION IF EXISTS docs;
```

---

## DML

```sql
-- Insert scalar columns (multi-row supported)
INSERT INTO docs (id, title) VALUES (1, 'Hello'), (2, 'World');

-- Insert / update with a vector passed as a bind parameter ($v)
UPSERT INTO docs (id, vector, title) VALUES (1, $v, 'First');

-- Update payload columns
UPDATE docs SET title = 'Renamed' WHERE id = 1;

-- Delete (WHERE is mandatory to prevent accidental full wipes)
DELETE FROM docs WHERE id = 1;
DELETE FROM docs WHERE id IN (1, 2, 3);
```

---

## Vector search (`vector NEAR`)

Vector search is a `WHERE` predicate inside a `SELECT`. The distance metric is
fixed at collection creation, not per query.

```sql
-- Top-k nearest neighbours (bind parameter)
SELECT * FROM docs WHERE vector NEAR $q LIMIT 10;

-- Inline vector literal
SELECT * FROM docs WHERE vector NEAR [0.1, 0.2, 0.3] LIMIT 5;

-- Filtered vector search (payload fields use dot paths)
SELECT id, title FROM docs
WHERE vector NEAR $q AND category = 'news'
LIMIT 10;

-- Expose the similarity score and sort by it
SELECT title, similarity() AS score FROM docs
WHERE vector NEAR $q
ORDER BY similarity() DESC
LIMIT 10;

-- Query-time HNSW override (trade speed for recall) via WITH (after LIMIT)
SELECT * FROM docs WHERE vector NEAR $q LIMIT 10 WITH (ef_search = 512);
```

---

## Sparse & hybrid search

```sql
-- Sparse-only search (inline {index: weight} literal, weights are floats)
SELECT * FROM docs WHERE vector SPARSE_NEAR {1: 0.5, 2: 0.3} LIMIT 5;

-- Sparse with a named index
SELECT * FROM docs WHERE vector SPARSE_NEAR $sv USING 'splade' LIMIT 5;

-- Dense + sparse in one query
SELECT * FROM docs WHERE vector NEAR $q AND vector SPARSE_NEAR $sv LIMIT 5;

-- Multi-vector fusion (RRF over several query vectors)
SELECT * FROM docs
WHERE vector NEAR_FUSED [$v1, $v2] USING FUSION 'rrf' (k = 60)
LIMIT 10;

-- Combine vector search with full-text MATCH
SELECT * FROM docs WHERE vector NEAR $q AND content MATCH 'database' LIMIT 10;

-- Hybrid dense + text with an explicit fusion strategy.
-- USING FUSION(...) is a trailing clause: it comes AFTER LIMIT.
SELECT * FROM docs
WHERE vector NEAR $q AND content MATCH 'database'
LIMIT 10 USING FUSION(strategy = 'rrf', k = 60);

-- Weighted fusion (parser reads vector_weight / graph_weight, not a weights[] array)
SELECT * FROM docs
WHERE vector NEAR $q AND content MATCH 'database'
LIMIT 10 USING FUSION(strategy = 'weighted', vector_weight = 0.7, graph_weight = 0.3);

-- Inline NEAR_FUSED with a bare strategy string (no params)
SELECT * FROM products WHERE vector NEAR_FUSED [$a, $b] USING FUSION 'weighted' LIMIT 20;
```

---

## Graph — MATCH traversal

```sql
-- 1-hop neighbours
MATCH (a:Author)-[:WROTE]->(p:Post)
WHERE a.id = 'auth-42'
RETURN p LIMIT 20;

-- Variable-depth traversal (1 to 3 hops)
MATCH (src)-[:LINKS*1..3]->(dst)
WHERE src.id = 'node-1'
RETURN dst LIMIT 100;

-- Graph predicate inside a SELECT
SELECT * FROM docs
WHERE category = 'tech' AND MATCH (d:Doc)-[:REL]->(x)
LIMIT 10;

-- With a FROM alias, the MATCH anchor reuses it — or, when no pattern alias
-- matches a declared alias, the leftmost node binds implicitly to the FROM
-- rows (validation rule V011)
SELECT * FROM docs AS d
WHERE category = 'tech' AND MATCH (d)-[:REL]->(x)
LIMIT 10;
```

---

## WHERE operators

```sql
SELECT * FROM docs WHERE lang = 'en' LIMIT 10;
SELECT * FROM docs WHERE status != 'draft' LIMIT 10;
SELECT * FROM docs WHERE score >= 0.8 LIMIT 10;
SELECT * FROM docs WHERE tag IN ('ai', 'ml') LIMIT 10;
SELECT * FROM docs WHERE year BETWEEN 2020 AND 2024 LIMIT 10;
SELECT * FROM docs WHERE title LIKE '%search%' LIMIT 10;
SELECT * FROM docs WHERE deleted_at IS NULL LIMIT 10;
SELECT * FROM docs WHERE tags CONTAINS 'ml' LIMIT 10;
SELECT * FROM docs WHERE tags CONTAINS ANY ('ai', 'ml') LIMIT 10;
SELECT * FROM docs WHERE body MATCH 'machine learning' LIMIT 10;
SELECT * FROM docs WHERE (lang = 'en' OR lang = 'fr') AND NOT archived = true LIMIT 10;
```

Payload sub-fields use dot paths: `payload.author.name = 'Ada'`.

---

## Aggregation, grouping & windows

```sql
-- GROUP BY with an aggregate and a HAVING filter
SELECT category, COUNT(*) AS n FROM docs
GROUP BY category
HAVING COUNT(*) > 5
LIMIT 50;

-- Window ranking function
SELECT id, category, RANK() OVER (PARTITION BY category ORDER BY score DESC) AS rk
FROM items;
```

---

## Joins & set operations

```sql
-- INNER JOIN across collections (AS is optional: `docs d` == `docs AS d`).
-- The joined (right) table must be matched on its primary key `id`.
SELECT d.name, t.tag FROM docs AS d JOIN tags AS t ON d.tag_id = t.id;
SELECT d.name, t.tag FROM docs d JOIN tags t ON d.tag_id = t.id;

-- Set operations
SELECT * FROM a UNION SELECT * FROM b;
SELECT * FROM a INTERSECT SELECT * FROM b;
```

---

## Pagination

```sql
SELECT * FROM docs WHERE vector NEAR $q LIMIT 10 OFFSET 20;
```

> There is no cursor/`AFTER` clause in VelesQL; use `LIMIT ... OFFSET ...`.

---

## Introspection & maintenance

```sql
SHOW COLLECTIONS;
DESCRIBE docs;
DESCRIBE COLLECTION docs;

-- Query plan without executing
EXPLAIN SELECT * FROM docs WHERE vector NEAR $q LIMIT 10;

-- Compute optimizer statistics
ANALYZE docs;
ANALYZE COLLECTION docs;

-- Maintenance
TRUNCATE COLLECTION docs;
ALTER COLLECTION docs SET (auto_reindex = true);
FLUSH;
FLUSH FULL docs;
```

---

## Guard-rails (enforced automatically)

| Limit | Default |
|---|---|
| LIMIT when omitted (any SELECT) | 10 — `MATCH ... RETURN` and UNION/INTERSECT/EXCEPT have no implicit limit (bounded by the 100 000-row ceiling below) |
| Max results per query | 100 000 |
| Query timeout | configurable (`search.query_timeout_ms`) |
| Max groups per GROUP BY | 10 000 |
| Max payload size | configurable (`limits.max_payload_size`) |

> Reserved words: `vector` and `score` are language keywords, not free payload
> field names. Table aliases accept both forms: `FROM docs AS d` and
> `FROM docs d`. A bare alias may not be a clause keyword (`WHERE`, `LIMIT`,
> `ORDER`, ...); quote it with backticks to use one anyway.
