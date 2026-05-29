# VelesQL Cheat Sheet

> **Date:** 2026-05-29  
> **VelesDB version:** 1.15.x  
> See the full specification in [`docs/VELESQL_SPEC.md`](../VELESQL_SPEC.md).

---

## DDL

```sql
-- Create a vector collection
CREATE COLLECTION docs DIMENSION 768;
CREATE COLLECTION docs DIMENSION 768 METRIC cosine;

-- Drop a collection
DROP COLLECTION docs;
```

---

## DML

```sql
-- Upsert a point (vector + optional payload)
UPSERT INTO docs (id, vector, payload)
VALUES ('a1', [0.1, 0.2, ...], '{"title": "hello"}');

-- Delete a point
DELETE FROM docs WHERE id = 'a1';
```

---

## NEAR vector search

```sql
-- Top-k nearest neighbours
NEAR docs [0.1, 0.2, ...] LIMIT 10;

-- With ef_search override
NEAR docs [0.1, 0.2, ...] LIMIT 10 EF 128;

-- Filtered NEAR
NEAR docs [0.1, 0.2, ...] LIMIT 10
WHERE payload->>'category' = 'news';
```

---

## Hybrid search — FUSION

```sql
-- Reciprocal Rank Fusion of two NEAR clauses
FUSION rrf (
  NEAR docs [0.1, 0.2, ...] LIMIT 50,
  NEAR docs [0.9, 0.8, ...] LIMIT 50
) LIMIT 10;

-- Weighted sum fusion
FUSION weighted_sum (
  NEAR docs [0.1, 0.2, ...] LIMIT 50 WEIGHT 0.7,
  NEAR docs [0.9, 0.8, ...] LIMIT 50 WEIGHT 0.3
) LIMIT 10;
```

---

## Graph — MATCH traversal

```sql
-- Find 1-hop neighbours
MATCH (a:Author)-[:WROTE]->(p:Post)
WHERE a.id = 'auth-42'
RETURN p LIMIT 20;

-- Variable-depth traversal (1 to 3 hops)
MATCH (src)-[:LINKS*1..3]->(dst)
WHERE src.id = 'node-1'
RETURN dst LIMIT 100;
```

---

## WHERE operators

| Operator | Example |
|---|---|
| `=` | `payload->>'lang' = 'en'` |
| `!=` | `payload->>'status' != 'draft'` |
| `>`, `>=`, `<`, `<=` | `payload->>'score'::float >= 0.8` |
| `IN` | `payload->>'tag' IN ('ai', 'ml')` |
| `IS NULL` / `IS NOT NULL` | `payload->>'deleted_at' IS NULL` |
| `AND`, `OR`, `NOT` | combine any of the above |

---

## Pagination

```sql
-- Offset-based
NEAR docs [...] LIMIT 10 OFFSET 20;

-- Cursor-based (preferred for large result sets)
NEAR docs [...] LIMIT 10 AFTER 'cursor-token';
```

---

## Aggregation / grouping

```sql
SELECT payload->>'category', COUNT(*) AS n
FROM docs
GROUP BY payload->>'category'
LIMIT 50
MAX_GROUPS 200;
```

---

## EXPLAIN

```sql
-- Show query plan without executing
EXPLAIN NEAR docs [0.1, 0.2, ...] LIMIT 10;
```

---

## Maintenance

```sql
-- Rebuild HNSW index
REINDEX COLLECTION docs;

-- Compact WAL / merge segments
COMPACT COLLECTION docs;
```

---

## Guard-rails (enforced automatically)

| Limit | Default |
|---|---|
| Max results per query | 10 000 |
| Query timeout | configurable (`search.query_timeout_ms`) |
| Max groups per GROUP BY | 10 000 (override with `MAX_GROUPS`) |
| Max payload size | configurable (`limits.max_payload_size`) |
