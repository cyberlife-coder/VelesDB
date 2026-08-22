# VelesQL cookbook (CLI & REPL)

Copy-pasteable VelesQL snippets that run as written in the
[`velesdb` REPL](CLI_REPL_REFERENCE.md) and via
`velesdb query execute ./data "<query>"`.

This is a task-oriented cookbook. The normative grammar, version history and
error semantics live in [VELESQL_SPEC.md](../VELESQL_SPEC.md); a one-page
summary lives in
[VELESQL_CHEATSHEET.md](../reference/VELESQL_CHEATSHEET.md).

> **Bind parameters (`$v`, `$query`) do not work in the REPL** — there is no
> mechanism to pass external values there. Snippets below that show `$v` are
> written for the REST API (`POST /query` with `params`) or the SDKs; in the
> REPL, substitute a literal vector such as `[0.1, 0.2, 0.3]`.

---

## Table of contents

- [Vector search](#vector-search)
- [Multi-vector fusion (NEAR_FUSED)](#multi-vector-fusion-near_fused)
- [Hybrid search (USING FUSION)](#hybrid-search-using-fusion)
- [Graph MATCH queries](#graph-match-queries)
- [Metadata-only collections](#metadata-only-collections)
- [Metadata filters](#metadata-filters)
- [Similarity threshold](#similarity-threshold)
- [Sparse vector search](#sparse-vector-search)
- [Temporal queries](#temporal-queries)
- [DISTINCT](#distinct)
- [Aggregations](#aggregations)
- [ORDER BY](#order-by)
- [OFFSET (pagination)](#offset-pagination)
- [Set operations](#set-operations)
- [JOIN](#join)
- [Subqueries](#subqueries-parsed-not-yet-executable)
- [EXPLAIN](#explain)
- [TRAIN QUANTIZER](#train-quantizer)
- [WITH clause (per-query options)](#with-clause-per-query-options)
- [Escaped identifiers](#escaped-identifiers)

---

## Vector search

```sql
-- Basic vector similarity search
SELECT * FROM documents
WHERE vector NEAR [0.15, 0.25, 0.35, 0.45]
LIMIT 10;

-- Vector search with metadata filter
SELECT * FROM documents
WHERE vector NEAR [0.1, 0.2, 0.3, 0.4]
AND category = 'tech'
AND views > 100
LIMIT 5;
```

> The distance metric is defined at collection creation time and applies to all
> searches on that collection. All five metrics are supported: Cosine,
> Euclidean, DotProduct, Hamming, Jaccard.

## Multi-vector fusion (NEAR_FUSED)

```sql
-- RRF fusion with multiple query vectors
SELECT * FROM documents
WHERE vector NEAR_FUSED [$v1, $v2, $v3] USING FUSION 'rrf' (k = 60)
LIMIT 10;

-- Weighted fusion
SELECT * FROM documents
WHERE vector NEAR_FUSED [$query1, $query2] USING FUSION 'weighted'
LIMIT 10;

-- Maximum fusion
SELECT * FROM documents
WHERE vector NEAR_FUSED [$v1, $v2] USING FUSION 'maximum'
LIMIT 10;

-- Default (RRF) -- the USING FUSION clause is optional
SELECT * FROM documents
WHERE vector NEAR_FUSED [$v1, $v2]
LIMIT 10;
```

## Hybrid search (USING FUSION)

The `USING FUSION` clause at query level combines results from multiple search
strategies (vector + text, dense + sparse):

```sql
-- Dense vector + BM25 full-text combined with RRF
SELECT * FROM documents
WHERE vector NEAR $v AND content MATCH 'rust programming'
LIMIT 10 USING FUSION(strategy = 'rrf', k = 60);

-- Dense + sparse vector fusion with RSF
SELECT * FROM documents
WHERE vector NEAR $dense AND vector SPARSE_NEAR $sparse
LIMIT 10 USING FUSION(strategy = 'rsf', dense_w = 0.7, sparse_w = 0.3);

-- Weighted fusion
SELECT * FROM docs
WHERE vector NEAR $v
LIMIT 10 USING FUSION(strategy = 'weighted', vector_weight = 0.7, graph_weight = 0.3);

-- Maximum fusion (take best score)
SELECT * FROM docs
WHERE vector NEAR $v
LIMIT 10 USING FUSION(strategy = 'maximum');

-- Default USING FUSION (defaults to RRF)
SELECT * FROM docs
WHERE vector NEAR $v
LIMIT 10 USING FUSION;
```

## Graph MATCH queries

```sql
-- Find authors of documents similar to a query
MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)
WHERE similarity(doc.embedding, $question) > 0.8
RETURN author.name, doc.title
ORDER BY similarity() DESC
LIMIT 5;

-- Multi-hop traversal with depth range
MATCH (user:User)-[:FOLLOWS*1..3]->(target:User)
WHERE user.name = 'Alice'
RETURN target.name, target.bio
LIMIT 20;

-- Undirected relationship (both directions)
MATCH (a:Person)-[:KNOWS]-(b:Person)
WHERE a.city = 'Paris'
RETURN a.name, b.name
LIMIT 10;

-- Incoming relationships
MATCH (doc:Document)<-[:AUTHORED_BY]-(author:Person)
RETURN doc.title, author.name
LIMIT 10;

-- Node property filtering in pattern
MATCH (doc:Document {status: 'published'})-[:HAS_TAG]->(tag:Tag)
RETURN doc.title, tag.name
LIMIT 20;

-- Combined: graph + vector + metadata in WHERE
SELECT * FROM articles
WHERE category = 'tech' AND MATCH (d:Doc)-[:HAS_TAG]->(tag)
LIMIT 10;
```

> **Cross-collection MATCH:** use `\use <collection>` to set the primary
> collection (the one with graph edges) before running a MATCH query. Nodes
> annotated with `@collection` have their payloads enriched from the named
> collection after traversal:
>
> ```
> velesdb> \use catalog_graph
> velesdb> MATCH (p:Product)-[:STORED_IN]->(inv:Inventory@inventory) RETURN p.name, inv.price LIMIT 20;
> ```

More patterns: [GRAPH_PATTERNS.md](GRAPH_PATTERNS.md).

## Metadata-only collections

Metadata collections store structured data without vectors. They support full
VelesQL `SELECT` queries:

```sql
-- Browse all items in a metadata collection
SELECT * FROM my_metadata LIMIT 10;

-- Filter by field
SELECT * FROM my_metadata WHERE status = 'active' LIMIT 20;

-- Count items
SELECT * FROM my_metadata WHERE price > 100 LIMIT 100;
```

> **Tip:** `.sample my_metadata`, `.browse my_metadata` and `.export
> my_metadata` all work for Metadata collections in the REPL (no "vector"
> column is shown).

## Metadata filters

```sql
-- Equality, comparison, pattern matching
SELECT * FROM docs WHERE category = 'tech' LIMIT 10;
SELECT * FROM docs WHERE price >= 50 AND price <= 200 LIMIT 10;
SELECT * FROM docs WHERE title LIKE '%rust%' LIMIT 10;

-- Case-insensitive pattern matching
SELECT * FROM docs WHERE title ILIKE '%Rust%' LIMIT 10;

-- IN, BETWEEN, NULL checks
SELECT * FROM docs WHERE status IN ('published', 'featured') LIMIT 10;
SELECT * FROM docs WHERE score BETWEEN 0.5 AND 1.0 LIMIT 10;
SELECT * FROM docs WHERE author IS NOT NULL LIMIT 10;

-- NOT and OR operators
SELECT * FROM docs WHERE NOT category = 'draft' LIMIT 10;
SELECT * FROM docs WHERE category = 'tech' OR category = 'science' LIMIT 10;

-- Nested field access (dot notation)
SELECT metadata.source, profile.type FROM docs WHERE metadata.lang = 'en' LIMIT 10;

-- Full-text search (BM25)
SELECT * FROM docs WHERE content MATCH 'rust programming' LIMIT 10;
```

## Similarity threshold

```sql
-- Return all documents above a similarity threshold (not just top-K)
SELECT * FROM docs
WHERE similarity(vector, $query) > 0.8
LIMIT 20;

-- Combine similarity threshold with metadata filters
SELECT * FROM docs
WHERE similarity(embedding, $ref) >= 0.9 AND category = 'tech'
LIMIT 10;
```

## Sparse vector search

```sql
-- Sparse vector similarity search
SELECT * FROM docs
WHERE vector SPARSE_NEAR $sparse_vector
LIMIT 10;

-- Sparse search with inline literal
SELECT * FROM docs
WHERE vector SPARSE_NEAR {12: 0.8, 45: 0.3, 891: 0.1}
LIMIT 10;

-- Sparse search on a named index
SELECT * FROM docs
WHERE vector SPARSE_NEAR $sv USING 'my_sparse_index'
LIMIT 10;
```

## Temporal queries

```sql
-- Filter by current time
SELECT * FROM events WHERE timestamp > NOW() LIMIT 10;

-- Last 7 days
SELECT * FROM logs WHERE created_at > NOW() - INTERVAL '7 days' LIMIT 50;

-- Last hour
SELECT * FROM alerts WHERE fired_at > NOW() - INTERVAL '1 hour' LIMIT 20;

-- Next week (scheduling)
SELECT * FROM tasks WHERE due_date < NOW() + INTERVAL '7 days' LIMIT 20;

-- Shorthand units: s, m, h, d, w, month
SELECT * FROM metrics WHERE ts > NOW() - INTERVAL '30 min' LIMIT 100;
```

## DISTINCT

```sql
-- Deduplicate results
SELECT DISTINCT category FROM documents LIMIT 50;

SELECT DISTINCT status, priority FROM tasks LIMIT 20;
```

## Aggregations

```sql
SELECT category, COUNT(*) as cnt
FROM documents
GROUP BY category
HAVING cnt > 5
ORDER BY cnt DESC
LIMIT 10;

-- Multiple aggregates
SELECT category, COUNT(*) as cnt, AVG(price) as avg_price, MIN(price), MAX(price)
FROM products
GROUP BY category
LIMIT 20;

-- SUM aggregate
SELECT region, SUM(quantity) as total
FROM orders
GROUP BY region
ORDER BY total DESC
LIMIT 10;

-- GROUP BY nested fields
SELECT metadata.source, COUNT(*) as cnt
FROM documents
GROUP BY metadata.source
LIMIT 20;
```

## ORDER BY

```sql
-- Multiple sort keys
SELECT * FROM docs ORDER BY category ASC, created_at DESC LIMIT 20;

-- Order by similarity score
SELECT * FROM docs
WHERE vector NEAR $v
ORDER BY similarity(vector, $v) DESC
LIMIT 10;
```

## OFFSET (pagination)

```sql
-- Skip first 20 results, return next 10
SELECT * FROM docs LIMIT 10 OFFSET 20;
```

## Set operations

```sql
-- UNION: combine results from two queries
SELECT id, title FROM news WHERE category = 'tech'
UNION
SELECT id, title FROM blog WHERE category = 'tech';

-- UNION ALL: include duplicates
SELECT id FROM collection_a
UNION ALL
SELECT id FROM collection_b;

-- INTERSECT: rows in both queries
SELECT id FROM favorites INTERSECT SELECT id FROM published;

-- EXCEPT: rows in first but not second
SELECT id FROM all_items EXCEPT SELECT id FROM archived;
```

## JOIN

```sql
-- INNER JOIN (default)
SELECT o.id, c.name
FROM orders AS o
JOIN customers AS c ON o.customer_id = c.id
LIMIT 20;

-- LEFT JOIN
SELECT d.title, a.name
FROM documents AS d
LEFT JOIN authors AS a ON d.author_id = a.id
LIMIT 20;

-- JOIN with vector search
SELECT o.id, c.name
FROM orders AS o
JOIN customers AS c ON o.customer_id = c.id
WHERE similarity(o.embedding, $q) > 0.7
LIMIT 20;
```

> **Note:** `LEFT JOIN` and `RIGHT JOIN` are parsed but raise runtime errors.
> `INNER JOIN` is fully supported.

## Subqueries (parsed, not yet executable)

> **Note:** subqueries are recognized by the VelesQL parser but raise runtime
> errors during execution. This syntax is reserved for future support.

```sql
-- IN subquery (parsed, execution not yet supported)
SELECT * FROM docs WHERE id IN (SELECT doc_id FROM comments) LIMIT 10;

-- Scalar subquery comparison (parsed, execution not yet supported)
SELECT * FROM products WHERE price > (SELECT AVG(price) FROM products) LIMIT 20;
```

## EXPLAIN

```sql
-- Show the query execution plan
EXPLAIN SELECT * FROM documents
WHERE vector NEAR [0.1, 0.2, 0.3, 0.4]
AND category = 'tech'
LIMIT 10;
```

In the REPL, `.explain <query>` and `.explain-analyze <query>` do the same,
the latter adding actual row counts and per-node timings.

## TRAIN QUANTIZER

```sql
-- Train a product quantizer on a collection
TRAIN QUANTIZER ON documents WITH (m = 8, k = 256);

-- With oversampling and force retrain
TRAIN QUANTIZER ON large_docs WITH (m = 16, k = 256, oversampling = 4, force = true);
```

| Parameter | Description | Typical values |
|-----------|-------------|----------------|
| `m` | Number of sub-spaces to divide the vector into. Higher = better recall, slower training. | 4, 8, 16, 32 |
| `k` | Number of centroids per sub-space. Almost always 256 (one byte per sub-quantizer). | 256 |
| `oversampling` | Oversampling factor for training data | 2, 4 |
| `force` | Force retraining even if a quantizer exists | `true`, `false` |

See also [QUANTIZATION.md](QUANTIZATION.md).

## WITH clause (per-query options)

```sql
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
WITH (mode = 'accurate');

SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
WITH (ef_search = 512, timeout_ms = 5000, rerank = true);

-- Quantization hints for dual-precision search
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
WITH (quantization = 'dual', oversampling = 4);
```

| Option | Type | Description |
|--------|------|-------------|
| `mode` | string | `fast`, `balanced`, `accurate`, `perfect`, `adaptive` |
| `ef_search` | integer | HNSW ef_search (16–4096) |
| `timeout_ms` | integer | Query timeout in milliseconds |
| `rerank` | boolean | Enable reranking after quantized search |
| `quantization` | string | Quantization precision: `f32`, `int8`, `dual`, `auto` |
| `oversampling` | integer | Oversampling ratio for dual-precision mode (>= 1) |

See also [SEARCH_MODES.md](SEARCH_MODES.md).

## Escaped identifiers

Use backticks or double quotes to use reserved words as column names:

```sql
-- Backtick style
SELECT `select`, `from`, `order` FROM docs LIMIT 10;

-- Double-quote style (SQL standard)
SELECT "select", "from", "order" FROM docs LIMIT 10;
```

---

Last updated: 2026-07-25 · Applies to: velesdb-core 5.2.0
