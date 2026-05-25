# VelesQL Cheat Sheet

One-page reference for the most common VelesQL statements. For the full
language reference (grammar, edge cases, conformance), see
[`docs/VELESQL_SPEC.md`](../VELESQL_SPEC.md).

> Conventions: `$name` = parameter placeholder; `[...]` = vector literal.
> Comments start with `--` (block `/* ... */` is **not** supported).

---

## Collections (DDL)

```sql
-- Create a vector collection (metric defaults to 'cosine')
CREATE COLLECTION docs (dimension = 768)
CREATE COLLECTION docs (dimension = 768, metric = 'euclidean')

-- With quantization + HNSW tuning
CREATE COLLECTION docs (dimension = 768, metric = 'cosine')
  WITH (storage = 'sq8', m = 16, ef_construction = 200)

-- Drop
DROP COLLECTION docs
```

| Metric | Aliases |
|---|---|
| `cosine` | `cos` |
| `euclidean` | `l2` |
| `dot` | `dotproduct`, `dot_product`, `inner`, `ip` |

| Storage | Memory footprint vs `full` |
|---|---|
| `full` (f32) | baseline |
| `sq8` (int8) | ÷4 |
| `binary` (1-bit) | ÷32 |

---

## Insert / Upsert / Update / Delete (DML)

```sql
-- Single row
INSERT INTO docs (id, vector, title) VALUES (1, $v, 'Getting Started')

-- Multi-row (v3.5+)
INSERT INTO docs (id, vector, title) VALUES (1, $v1, 'A'), (2, $v2, 'B')

-- Upsert (explicit overwrite semantics)
UPSERT INTO docs (id, vector, title) VALUES (1, $v, 'Updated')

-- Update by predicate
UPDATE docs SET payload.title = 'New' WHERE id = 1

-- Delete by predicate
DELETE FROM docs WHERE category = 'archive'
```

---

## Vector Search — `NEAR`

```sql
-- Top-k cosine search
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10

-- With metadata filter (ColumnStore push-down)
SELECT id, payload.title, similarity() AS score
FROM docs
WHERE vector NEAR $v AND category = 'tech' AND price > 50
LIMIT 10

-- Inline vector literal
SELECT * FROM docs WHERE vector NEAR [0.1, 0.2, 0.3] LIMIT 5
```

### Search-time options — `WITH (...)`

```sql
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
  WITH (mode = 'accurate', rerank = true)
```

| Option | Values | Notes |
|---|---|---|
| `mode` / `quality` | `fast` \| `balanced` \| `accurate` \| `high_recall` \| `perfect` | Picks `ef_search` preset |
| `ef_search` | `16..4096` | Overrides `mode` |
| `rerank` | `true` \| `false` | Two-stage SIMD rerank (4× candidates) |
| `quantization` | `f32` \| `int8` \| `dual` \| `auto` | Query-time precision |
| `oversampling` | `>= 1.0` | Used with `quantization = 'dual'` |
| `timeout_ms` | `>= 100` | Per-query timeout |

---

## Hybrid & Multi-Vector — `MATCH` text, `SPARSE_NEAR`, `NEAR_FUSED`

```sql
-- Dense vector + BM25 full-text (fusion defaults to RRF)
SELECT * FROM docs
WHERE vector NEAR $v AND content MATCH 'rag pipeline'
USING FUSION(strategy = 'rrf', k = 60)
LIMIT 20

-- Sparse vector (BM25 / SPLADE-style)
SELECT * FROM docs WHERE vector SPARSE_NEAR $sparse LIMIT 10

-- Multi-vector ensemble (text + image, etc.)
SELECT * FROM docs
WHERE vector NEAR_FUSED [$text_emb, $image_emb]
USING FUSION 'rrf' (k = 60)
LIMIT 20
```

| Fusion strategy | Parameters |
|---|---|
| `rrf` | `k` (default 60) |
| `weighted` | `weights = [w1, w2, ...]` |
| `maximum` | — (max score per item) |

---

## Filters — `WHERE` operators

```sql
-- Comparison
WHERE price > 50 AND stock <= 100 AND status != 'archived'

-- Membership
WHERE category IN ('tech', 'science')
WHERE country NOT IN ('FR', 'DE')

-- Ranges
WHERE created_at BETWEEN '2026-01-01' AND '2026-03-31'

-- Text patterns
WHERE title LIKE 'Intro%'     -- case-sensitive
WHERE title ILIKE '%rag%'     -- case-insensitive

-- Null checks
WHERE author IS NULL
WHERE author IS NOT NULL

-- Array containment (v3.7+)
WHERE tags CONTAINS 'ai'

-- Strict substring (excludes non-matches, unlike MATCH)
WHERE content CONTAINS_TEXT 'velesdb'

-- Geospatial (v3.7+)
WHERE GEO_DISTANCE(location, $point) < 5000
WHERE GEO_BBOX(location, $sw, $ne)
```

---

## Graph — `MATCH` patterns (v2.1+)

```sql
-- Node + relationship patterns
MATCH (a:Person)-[:KNOWS]->(b:Person)
RETURN a.name, b.name

-- Variable-length path
MATCH (start:Document)-[*1..3]->(end:Document)
WHERE start.topic = 'AI'
RETURN start.title, end.title

-- Vector similarity inside a graph query (GraphRAG)
MATCH (doc:Document)
WHERE similarity(doc.embedding, $query) > 0.8
RETURN doc.title
ORDER BY similarity() DESC
LIMIT 5

-- Cross-collection enrichment with @collection
MATCH (p:Product)-[:STORED_IN]->(w:Warehouse@inventory)
RETURN p.name, w.price, w.stock
LIMIT 20
```

| Pattern | Meaning |
|---|---|
| `(a)` / `(a:Label)` / `(:Label)` / `()` | Node, optional label/alias |
| `(a:Label {prop: value})` | Node with property filter |
| `-[r:TYPE]->` | Outgoing relationship, named |
| `<-[r:TYPE]-` | Incoming |
| `-[r:TYPE]-` | Undirected |
| `-[r:T1\|T2]->` | Multiple types |
| `*1..3` / `*..5` / `*2..` / `*3` / `*` | Range of hops |

### Graph mutations

```sql
INSERT EDGE INTO knowledge (source = 1, target = 2, label = 'AUTHORED_BY')
DELETE EDGE FROM knowledge WHERE source = 1 AND target = 2
INSERT NODE INTO catalog (id = 1, label = 'Product', payload = {...})
```

---

## Projection helpers

```sql
SELECT id, payload.title, payload.metadata.author FROM articles
SELECT id, similarity() AS score FROM docs WHERE vector NEAR $v
SELECT id, vector_score, bm25_score FROM docs           -- per-component scores
SELECT DISTINCT category FROM docs
SELECT category, COUNT(*) FROM docs GROUP BY category
```

| Score function | Populated when |
|---|---|
| `similarity()` | Any search path (primary score) |
| `vector_score` | `NEAR` present |
| `bm25_score` | `MATCH` text present |
| `sparse_score` | `SPARSE_NEAR` present |
| `graph_score` | `MATCH` graph pattern present |
| `fused_score` | After `USING FUSION` |

---

## Pagination & ordering

```sql
SELECT * FROM docs ORDER BY similarity() DESC LIMIT 10 OFFSET 20
```

---

## Inspect query plans

```sql
EXPLAIN SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
```

`EXPLAIN ANALYZE` runs the query and returns timings + CBO calibration.
It is exposed at the API level — call `explain_analyze_query()` (SDK) or
`POST /query/explain?analyze=true` (REST).

---

## Maintenance

```sql
FLUSH                       -- flush WAL to disk
FLUSH FULL                  -- compact + snapshot
TRAIN QUANTIZER ON docs     -- (re)train PQ/SQ codebooks
```
