# VelesQL Language Specification

> SQL-like query language for vector + graph + column-store search in VelesDB.

**Version**: 3.7.0 | **Last Updated**: 2026-04-06

---

## Overview

VelesQL is a SQL-inspired query language designed for hybrid search workloads.
It combines familiar SQL syntax with vector-specific operations (`NEAR`,
`SPARSE_NEAR`, `NEAR_FUSED`), graph pattern matching (`MATCH`), full-text search
(`MATCH` BM25), and collection management (`CREATE`, `DROP`).

All keywords are **case-insensitive**: `SELECT`, `select`, and `Select` are
equivalent. Identifiers (collection names, column names) are case-sensitive.

### Feature Support Status

| Feature | Status | Version |
|---------|--------|---------|
| SELECT, FROM, WHERE | Stable | 1.0 |
| NEAR vector search | Stable | 1.0 |
| similarity() function | Stable | 1.3 |
| LIMIT, OFFSET | Stable | 1.0 |
| WITH clause | Stable | 1.0 |
| ORDER BY | Stable | 2.0 |
| GROUP BY, HAVING | Stable | 2.0 |
| JOIN (INNER ... ON) | Stable | 2.0 |
| JOIN (LEFT, RIGHT, FULL) | Parsed, explicit runtime error | 2.0 |
| JOIN USING | Experimental (single-column) | 2.0 |
| Set Operations (UNION, INTERSECT, EXCEPT) | Stable | 2.0 |
| USING FUSION | Stable | 2.0 |
| NOW() / INTERVAL temporal | Stable | 2.1 |
| MATCH graph traversal | Stable | 2.1 |
| SPARSE_NEAR sparse vector search | Stable | 2.2 |
| NEAR_FUSED multi-vector fusion | Stable | 2.2 |
| TRAIN QUANTIZER command | Stable | 2.2 |
| ORDER BY arithmetic scoring | Stable | 3.0 |
| LET score bindings | Stable | 3.2 |
| Agent Memory VelesQL queries | Stable | 3.2 |
| INSERT INTO statement | Stable | 3.2 |
| UPDATE statement | Stable | 3.2 |
| CREATE COLLECTION | Stable | 3.3 |
| DROP COLLECTION | Stable | 3.3 |
| INSERT EDGE graph mutation | Stable | 3.3 |
| DELETE FROM statement | Stable | 3.3 |
| DELETE EDGE graph mutation | Stable | 3.3 |
| DISTINCT modifier | Stable | 3.3 |
| ILIKE case-insensitive pattern | Stable | 3.3 |
| FROM / JOIN aliases | Stable (INNER JOIN) | 2.0 |
| Scalar subqueries | Stable | 3.2 |
| SQL comments (`--`) | Stable | 1.0 |
| Identifier quoting (backtick, double-quote) | Stable | 1.3 |
| SHOW COLLECTIONS | Stable | 3.4 |
| DESCRIBE COLLECTION | Stable | 3.4 |
| EXPLAIN query plan | Stable | 3.4 |
| CREATE INDEX / DROP INDEX | Stable | 3.5 |
| ANALYZE | Stable | 3.5 |
| TRUNCATE | Stable | 3.5 |
| ALTER COLLECTION | Stable | 3.5 |
| Multi-row INSERT | Stable | 3.5 |
| UPSERT INTO statement | Stable | 3.5 |
| WITH (quality='...') alias | Stable | 3.5 |
| SELECT EDGES graph query | Stable | 3.5 |
| INSERT NODE graph mutation | Stable | 3.5 |
| FLUSH / FLUSH FULL | Stable | 3.6 |
| CONTAINS array filter | Stable | 3.7 |
| GEO_DISTANCE / GEO_BBOX geospatial | Stable | 3.7 |
| GROUP BY MAX(score) / AVG(score) | Stable | 3.7 |
| FIRST(column) projection | Stable | 3.7 |
| FUSE BY fusion clause | Planned | -- |

### REST Contract Notes

- `/query` supports top-level `MATCH`, but request body must include `collection`.
- `/collections/{name}/match` is the collection-scoped graph endpoint.
- Canonical payload contract: `docs/reference/VELESQL_CONTRACT.md`.
- Conformance test matrix: `docs/reference/VELESQL_CONFORMANCE_MATRIX.md`.
- Recommended developer syntax for mixed filters:
  `SELECT ... FROM <collection> WHERE ... AND MATCH (...)`.

---

## Comments

VelesQL supports single-line comments with `--`. Everything after `--` to the end
of the line is ignored by the parser.

```sql
-- This is a comment
SELECT * FROM docs LIMIT 10  -- inline comment after a statement
```

Block comments (`/* ... */`) are **not** supported.

---

## SELECT Statement

The SELECT statement is the primary way to query data from collections.

### Full Syntax

```sql
[LET <name> = <expr> ...]
SELECT [DISTINCT] <columns>
FROM <collection> [AS <alias>]
[JOIN <collection2> [AS <alias>] ON <condition> | USING (<column>)]
[WHERE <conditions>]
[GROUP BY <columns>]
[HAVING <aggregate_condition>]
[ORDER BY <expressions>]
[LIMIT <n>]
[OFFSET <n>]
[WITH (<options>)]
[USING FUSION (<strategy>)]
```

---

## SELECT Clause

### Select All Columns

```sql
SELECT * FROM documents
```

### Select Specific Columns

```sql
SELECT id, score FROM documents
SELECT id, payload.title, payload.category FROM documents
```

### Nested Payload Fields

Access nested JSON fields using dot notation at any depth:

```sql
SELECT payload.metadata.author FROM articles
SELECT payload.stats.views, payload.stats.likes FROM posts
```

### Column Aliases

Rename output columns with `AS`:

```sql
SELECT id, payload.title AS title FROM docs
SELECT id AS doc_id, category AS cat FROM products
```

### DISTINCT Modifier

Eliminate duplicate rows from the result set. DISTINCT applies to ALL selected
columns -- two rows are considered duplicates only when every selected column
has the same value.

```sql
-- Unique categories
SELECT DISTINCT category FROM products

-- Unique authors from nested payload
SELECT DISTINCT payload.author FROM docs

-- DISTINCT with WHERE filter
SELECT DISTINCT category FROM products WHERE price > 50

-- DISTINCT on multiple columns (unique combinations)
SELECT DISTINCT category, status FROM products
```

When combined with vector search, DISTINCT deduplicates based on payload values
after the search results are returned:

```sql
SELECT DISTINCT payload.source FROM docs WHERE vector NEAR $v LIMIT 20
```

### Qualified Wildcard

Select all columns from a specific alias in a JOIN:

```sql
SELECT ctx.* FROM docs AS ctx WHERE vector NEAR $v LIMIT 10
```

### Aggregate Functions in SELECT

```sql
SELECT COUNT(*) FROM docs
SELECT category, COUNT(*), AVG(price), MAX(rating) FROM products GROUP BY category
SELECT SUM(quantity) AS total FROM orders
```

| Function | Description | Argument |
|----------|-------------|----------|
| `COUNT(*)` | Count all rows | Wildcard |
| `COUNT(col)` | Count non-null values in column | Column name |
| `SUM(col)` | Sum of numeric values | Column name |
| `AVG(col)` | Average of numeric values | Column name |
| `MIN(col)` | Minimum value | Column name |
| `MAX(col)` | Maximum value | Column name |
| `MAX(score)` | Max similarity score across group (v3.7+) | `score` pseudo-column |
| `AVG(score)` | Mean similarity score across group (v3.7+) | `score` pseudo-column |
| `FIRST(col)` | Value from highest-scoring row in group (v3.7+) | Column name |

### Similarity Score in SELECT

Use the zero-argument `similarity()` to project the pre-computed search score:

```sql
SELECT similarity() FROM docs WHERE vector NEAR $v LIMIT 10
SELECT similarity() AS score FROM docs WHERE vector NEAR $v LIMIT 10
SELECT id, payload.title, similarity() AS relevance
FROM docs WHERE vector NEAR $v ORDER BY similarity() DESC LIMIT 5
```

---

## FROM Clause

Specify the source collection. An optional alias enables shorter column references
and is required for self-joins.

```sql
-- Simple
SELECT * FROM my_collection

-- With alias
SELECT * FROM documents AS d

-- Alias used in column references
SELECT d.title, d.category FROM documents AS d WHERE d.price > 50
```

---

## JOIN Clause (v2.0+)

Combine data from multiple collections.

### Syntax

```sql
SELECT <columns>
FROM <table1> [AS <alias1>]
[INNER | LEFT | RIGHT | FULL] JOIN <table2> [AS <alias2>]
  ON <alias1>.<col> = <alias2>.<col>
```

### Join Types

| Type | Keyword | Description | Runtime |
|------|---------|-------------|---------|
| Inner | `JOIN` or `INNER JOIN` | Only matching rows from both sides | Fully executed |
| Left | `LEFT JOIN` or `LEFT OUTER JOIN` | All left + matching right | Parsed, runtime error |
| Right | `RIGHT JOIN` or `RIGHT OUTER JOIN` | All right + matching left | Parsed, runtime error |
| Full | `FULL JOIN` or `FULL OUTER JOIN` | All from both tables | Parsed, runtime error |

### ON Condition

Specify the join equality condition:

```sql
SELECT o.id, c.name
FROM orders AS o
JOIN customers AS c ON o.customer_id = c.id
```

### USING Clause

Shorthand when both tables share the same column name:

```sql
SELECT o.id, c.name
FROM orders AS o
JOIN customers AS c USING (customer_id)
```

> **Limitation**: USING currently supports a single column only.

### Self-Join

Join a collection with itself using aliases:

```sql
SELECT e.name AS employee, m.name AS manager
FROM employees AS e
JOIN employees AS m ON e.manager_id = m.id
```

### Multiple Joins

Chain multiple JOINs in a single query:

```sql
SELECT e.name, m.name AS manager, d.name AS director
FROM employees AS e
JOIN employees AS m ON e.manager_id = m.id
JOIN employees AS d ON m.manager_id = d.id
```

### JOIN with Vector Search

```sql
SELECT p.title, c.name AS category_name
FROM products AS p
JOIN categories AS c ON p.category_id = c.id
WHERE similarity(p.embedding, $query) > 0.7
LIMIT 20
```

### JOIN Optimizations (v1.12+)

The query engine automatically applies two optimizations to cross-collection JOINs:

**Filter Pushdown**: WHERE conditions that reference the joined table (using qualified column names like `inventory.price > 100`) are automatically pushed down and evaluated before loading points into the ColumnStore. This reduces memory usage and I/O for large joined collections.

```sql
-- inventory.stock > 0 is pushed down to filter inventory before JOIN
SELECT * FROM products
JOIN inventory ON products.id = inventory.id
WHERE inventory.stock > 0
```

**Lookup Join**: When the JOIN condition references the primary key (`id`) on both sides and no pushdown filters apply, the engine uses direct `collection.get()` lookups instead of building a full ColumnStore, achieving O(K) instead of O(N) retrieval.

```sql
-- Uses optimized lookup path (both sides reference id)
SELECT * FROM products
JOIN inventory ON products.id = inventory.id
```

Both optimizations are transparent and produce identical results to the non-optimized path.

---

## WHERE Clause

The WHERE clause filters rows using conditions. Multiple conditions are combined
with `AND`, `OR`, and `NOT`.

### Comparison Operators

| Operator | Description | Example |
|----------|-------------|---------|
| `=` | Equal | `category = 'tech'` |
| `!=` or `<>` | Not equal | `status != 'deleted'` |
| `>` | Greater than | `price > 100` |
| `>=` | Greater or equal | `score >= 0.8` |
| `<` | Less than | `count < 50` |
| `<=` | Less or equal | `rating <= 5` |

```sql
SELECT * FROM products WHERE price > 100 AND category = 'electronics'
SELECT * FROM users WHERE age >= 18 AND age <= 65
SELECT * FROM docs WHERE status <> 'deleted'
```

### Logical Operators (AND, OR, NOT)

| Priority | Operator | Description |
|----------|----------|-------------|
| 1 (highest) | `NOT` | Negation |
| 2 | `AND` | Conjunction |
| 3 (lowest) | `OR` | Disjunction |

```sql
-- AND
SELECT * FROM docs WHERE category = 'tech' AND price > 50

-- OR
SELECT * FROM docs WHERE category = 'tech' OR category = 'science'

-- Parentheses to override precedence
SELECT * FROM docs WHERE (category = 'tech' OR category = 'ai') AND price > 100
```

#### NOT Expression

The `NOT` operator negates any condition, including comparisons, `IN`, `BETWEEN`,
and parenthesized groups.

```sql
-- Negate a comparison
SELECT * FROM docs WHERE NOT (price > 100)

-- Negate an IN list
SELECT * FROM docs WHERE NOT (category IN ('draft', 'archived'))

-- Negate a compound condition
SELECT * FROM docs WHERE NOT (status = 'deleted' AND archived = true)

-- NOT with vector search filter
SELECT * FROM docs
WHERE vector NEAR $v AND NOT (category = 'spam')
LIMIT 10
```

### IN / NOT IN

Test membership in a list of values:

```sql
-- String values
SELECT * FROM docs WHERE category IN ('tech', 'science', 'ai')

-- Integer values
SELECT * FROM docs WHERE id IN (1, 2, 3, 4, 5)

-- NOT IN
SELECT * FROM docs WHERE category NOT IN ('draft', 'deleted')
SELECT * FROM docs WHERE id NOT IN (1, 2, 3)
```

**Index acceleration**: When a secondary index exists on the filtered column,
`IN` uses bitmap pre-filtering to restrict HNSW traversal to matching points
only. The engine builds a `RoaringBitmap` by unioning per-value B-tree lookups
(time complexity: O(N × log K) where N = list size, K = index cardinality).
`NOT IN` on indexed fields computes the universe bitmap minus the IN bitmap
(set subtraction). Queries on non-indexed fields fall back to post-filtering
transparently. Results are identical in both paths.

### BETWEEN

Test if a value falls within a range (inclusive):

```sql
SELECT * FROM docs WHERE price BETWEEN 10 AND 100
SELECT * FROM docs WHERE date BETWEEN '2024-01-01' AND '2024-12-31'
SELECT * FROM products WHERE rating BETWEEN 3.5 AND 5.0
```

### LIKE (Pattern Matching)

Case-sensitive pattern matching with wildcards:

| Wildcard | Meaning |
|----------|---------|
| `%` | Any sequence of characters (including empty) |
| `_` | Exactly one character |

```sql
-- Suffix match
SELECT * FROM docs WHERE title LIKE '%database%'

-- Prefix match
SELECT * FROM docs WHERE name LIKE 'vec%'

-- Single character wildcard
SELECT * FROM docs WHERE code LIKE 'v_les'

-- Combined with other filters
SELECT * FROM docs WHERE name LIKE 'John%' AND active = true
```

### ILIKE (Case-Insensitive Pattern Matching)

`ILIKE` works identically to `LIKE` but performs case-insensitive matching.
The same `%` and `_` wildcards apply.

```sql
-- Matches 'Database', 'DATABASE', 'database', etc.
SELECT * FROM docs WHERE title ILIKE '%database%'

-- Case-insensitive prefix match
SELECT * FROM docs WHERE name ILIKE 'vec%'

-- Single-character wildcard (case-insensitive)
SELECT * FROM docs WHERE code ILIKE 'v_les'

-- Combined with other filters
SELECT * FROM docs
WHERE category ILIKE '%science%' AND price > 50
LIMIT 20
```

| Operator | Case-Sensitive | Example Match for `'hello'` |
|----------|---------------|------------------------------|
| `LIKE` | Yes | `LIKE 'hello'` matches `hello` only |
| `ILIKE` | No | `ILIKE 'hello'` matches `Hello`, `HELLO`, `hello` |

Unicode behavior: case folding follows Rust's default Unicode lowercase rules.
Characters without a defined case mapping (e.g., CJK ideographs) are compared as-is.

### IS NULL / IS NOT NULL

Test for null values:

```sql
SELECT * FROM docs WHERE category IS NULL
SELECT * FROM docs WHERE category IS NOT NULL
SELECT * FROM users WHERE email IS NOT NULL AND verified = true
```

### CONTAINS (Array Filter, v3.7+)

Test whether an array field contains specific values. Three modes are supported:

| Syntax | Description |
|--------|-------------|
| `column CONTAINS value` | Array contains the single value |
| `column CONTAINS ANY (v1, v2, ...)` | Array contains at least one of the values |
| `column CONTAINS ALL (v1, v2, ...)` | Array contains every listed value |

```sql
-- Single value containment
SELECT * FROM hotels WHERE amenities CONTAINS 'pool' LIMIT 10

-- At least one match (OR semantics)
SELECT * FROM hotels WHERE amenities CONTAINS ANY ('spa', 'bar') LIMIT 10

-- All values must be present (AND semantics)
SELECT * FROM hotels WHERE amenities CONTAINS ALL ('pool', 'gym') LIMIT 10

-- Combined with scalar filters
SELECT * FROM hotels
WHERE amenities CONTAINS 'pool' AND rating > 4.5
LIMIT 10

-- Combined with vector search
SELECT * FROM hotels
WHERE amenities CONTAINS 'spa' OR vector NEAR $v
LIMIT 10

-- Integer arrays
SELECT * FROM items WHERE scores CONTAINS 10 LIMIT 10

-- NOT negation
SELECT * FROM hotels WHERE NOT (amenities CONTAINS 'pool') LIMIT 10

-- Multiple array fields
SELECT * FROM hotels
WHERE amenities CONTAINS 'gym' AND tags CONTAINS 'luxury'
LIMIT 10
```

Behavior on edge cases:
- Empty array (`[]`): CONTAINS returns no match
- Null/missing field: row excluded from results
- Non-array field: CONTAINS returns no match (use `LIKE` for substring matching)
- Non-existent field: returns empty result set

### Full-Text Search (MATCH)

The `MATCH` operator performs BM25 full-text search against the collection's
text index.

**MATCH alone** -- returns BM25 text search results ranked by text relevance:

```sql
SELECT * FROM docs WHERE content MATCH 'vector database' LIMIT 10
```

**MATCH + NEAR (hybrid search)** -- triggers Reciprocal Rank Fusion (RRF)
between vector similarity and BM25 text relevance. In this mode, `MATCH` acts
as a **score boost, not a strict filter**. Results that do not contain the
keyword may still appear with lower fused scores:

```sql
-- Hybrid: vector similarity fused with BM25 text relevance via RRF
SELECT * FROM docs WHERE vector NEAR $v AND content MATCH 'database' LIMIT 10
```

**Multi-condition hybrid search**:

```sql
SELECT * FROM docs
WHERE vector NEAR $v AND title MATCH 'vector database' AND category = 'tech'
LIMIT 10
```

> **Known Limitations (v1.9.0)**:
> - **No strict text filter**: `MATCH` in hybrid mode boosts but does not filter.
>   A dedicated text filter operator is planned (see issue #446).
> - **Column parameter ignored**: The column name (e.g., `content`) is parsed but
>   the execution engine searches all indexed text fields regardless.

### Vector Search (NEAR)

Use `NEAR` for approximate nearest neighbor search on dense vectors:

```sql
-- With parameter placeholder
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10

-- With literal vector
SELECT * FROM docs WHERE vector NEAR [0.1, 0.2, 0.3, 0.4] LIMIT 5

-- With metadata filter
SELECT * FROM docs
WHERE vector NEAR $v AND category = 'tech' AND price > 50
LIMIT 10

-- With similarity score projection and ordering
SELECT id, title, similarity() AS score
FROM docs WHERE vector NEAR $query
ORDER BY similarity() DESC
LIMIT 5

-- With search quality tuning
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
WITH (mode = 'accurate', ef_search = 256)
```

### Sparse Vector Search (SPARSE_NEAR, v2.2+)

Use `SPARSE_NEAR` for sparse vector similarity search (SPLADE, BM42, or custom
sparse embeddings).

```sql
-- With parameter placeholder
SELECT * FROM docs WHERE vector SPARSE_NEAR $sv LIMIT 10

-- With inline sparse vector literal (dimension_index: weight)
SELECT * FROM docs
WHERE vector SPARSE_NEAR {0: 0.9, 14: 0.7, 256: 0.5, 1024: 0.3}
LIMIT 10

-- With named sparse index
SELECT * FROM docs
WHERE vector SPARSE_NEAR $sparse USING 'bm42_index'
LIMIT 10

-- Combined dense + sparse (hybrid)
SELECT * FROM docs
WHERE vector NEAR $dense AND vector SPARSE_NEAR $sparse
LIMIT 10
```

Sparse vectors use `{index: weight}` format where `index` is a non-negative
integer (dimension) and `weight` is a float value. Scoring uses inner product
with MaxScore DAAT for efficient top-K retrieval.

### Multi-Vector Fusion (NEAR_FUSED, v2.2+)

`NEAR_FUSED` combines multiple embedding vectors into a single similarity search
using a fusion strategy. Useful for multi-modal search (text + image embeddings)
or ensemble approaches.

```sql
-- Two-vector fusion with RRF
SELECT * FROM products
WHERE vector NEAR_FUSED [$text_emb, $image_emb]
USING FUSION 'rrf' (k=60)
LIMIT 20

-- Three-vector ensemble
SELECT * FROM docs
WHERE vector NEAR_FUSED [$query_v1, $query_v2, $query_v3]
USING FUSION 'maximum'
LIMIT 10

-- Without explicit fusion (defaults to RRF)
SELECT * FROM docs
WHERE vector NEAR_FUSED [$v1, $v2]
LIMIT 5
```

The vector array `[$v1, $v2, ...]` accepts parameters (`$name`) or vector
literals (`[0.1, 0.2, ...]`).

| Strategy | Best For | Parameters |
|----------|----------|------------|
| `rrf` | General-purpose ensemble (default) | `k` (default: 60) |
| `rsf` | Normalized score blending | `dense_weight`, `sparse_weight` |
| `weighted` | Explicit priority tuning | `weight_1`, `weight_2`, ... |
| `maximum` | Conservative high-precision | (none) |

### Similarity Function (v1.3+)

The `similarity()` function enables threshold-based vector filtering -- filter
results by similarity score rather than just finding nearest neighbors.

#### Syntax

```sql
similarity(field, vector_expr) <operator> threshold
```

| Parameter | Description |
|-----------|-------------|
| `field` | The vector field name (e.g., `vector`, `embedding`) |
| `vector_expr` | A parameter (`$v`) or literal vector (`[0.1, 0.2, ...]`) |
| `operator` | Comparison: `>`, `>=`, `<`, `<=`, `=` |
| `threshold` | Similarity score (0.0 to 1.0 for cosine/dot) |

#### Examples

```sql
-- Find documents with similarity > 0.8
SELECT * FROM docs WHERE similarity(vector, $query) > 0.8

-- High precision filtering (>= 0.9)
SELECT * FROM docs WHERE similarity(embedding, $v) >= 0.9 LIMIT 10

-- Exclude very similar documents (deduplication)
SELECT * FROM docs WHERE similarity(vector, $ref) < 0.95

-- Combined with metadata filters
SELECT * FROM docs
WHERE similarity(vector, $q) > 0.7 AND category = 'technology'
LIMIT 20
```

#### NEAR vs similarity()

| Feature | `NEAR` | `similarity()` |
|---------|--------|----------------|
| Purpose | Find K nearest neighbors | Filter by score threshold |
| Returns | Top-K results | All matching results |
| Control | `LIMIT N` | Threshold value |
| Best for | "Find similar" | "Filter by quality" |

```sql
-- NEAR: "Give me 10 most similar docs"
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10

-- similarity(): "Give me docs with similarity > 0.8"
SELECT * FROM docs WHERE similarity(vector, $v) > 0.8
```

### Temporal Functions (v2.1+)

VelesQL supports temporal expressions for date/time filtering using `NOW()`
and `INTERVAL`.

#### NOW() Function

Returns the current Unix timestamp (seconds since epoch):

```sql
SELECT * FROM events WHERE timestamp > NOW()
SELECT * FROM items WHERE created_at > NOW()
```

#### INTERVAL Expression

Defines a time duration with the syntax `INTERVAL '<magnitude> <unit>'`:

| Unit | Aliases | Seconds |
|------|---------|---------|
| seconds | s, sec, second | 1 |
| minutes | m, min, minute | 60 |
| hours | h, hour | 3,600 |
| days | d, day | 86,400 |
| weeks | w, week | 604,800 |
| months | month | ~2,592,000 (30 days) |

#### Temporal Arithmetic

Combine `NOW()` with `INTERVAL` using `+` or `-`:

```sql
-- Last 7 days
SELECT * FROM logs WHERE created_at > NOW() - INTERVAL '7 days'

-- Last hour
SELECT * FROM events WHERE timestamp > NOW() - INTERVAL '1 hour'

-- Next week (future events)
SELECT * FROM tasks WHERE due_date < NOW() + INTERVAL '7 days'

-- Shorthand units
SELECT * FROM metrics WHERE ts > NOW() - INTERVAL '30 min'

-- Filter items older than 30 days
SELECT * FROM logs WHERE created_at < NOW() - INTERVAL '30 days'
```

#### Common Temporal Patterns

| Use Case | Query |
|----------|-------|
| Last 24 hours | `WHERE ts > NOW() - INTERVAL '24 hours'` |
| This week | `WHERE ts > NOW() - INTERVAL '7 days'` |
| Last month | `WHERE ts > NOW() - INTERVAL '1 month'` |
| Recent activity | `WHERE last_seen > NOW() - INTERVAL '5 minutes'` |

> **Note:** `NOW()` returns a Unix timestamp in seconds (timezone-agnostic).
> Month intervals are approximated as 2,592,000 seconds (30 days).

### Scalar Subqueries (v3.2+)

Use a scalar subquery in WHERE to compare against a computed value from another
(or the same) collection. The subquery must return exactly one row and one column.

```sql
-- Filter where amount exceeds the average
SELECT * FROM orders
WHERE amount > (SELECT AVG(amount) FROM orders)

-- Compare against a value from another collection
SELECT * FROM products
WHERE price < (SELECT MAX(budget) FROM departments WHERE name = 'engineering')
LIMIT 20
```

A subquery is enclosed in parentheses and contains a full SELECT statement
(with optional WHERE, GROUP BY, HAVING, LIMIT).

### Graph Match Predicate in WHERE

Embed a graph pattern inside a WHERE clause using the `MATCH` keyword:

```sql
SELECT * FROM docs
WHERE category = 'tech' AND MATCH (d:Doc)-[:REL]->(x)
LIMIT 10
```

This filters rows that participate in the specified graph pattern, combined with
any other scalar conditions.

### Vector Search with Filters

Combine vector search with any number of metadata filters:

```sql
SELECT * FROM docs
WHERE vector NEAR $v
  AND category = 'tech'
  AND price > 50
  AND status IS NOT NULL
  AND tags IN ('ai', 'ml')
LIMIT 10
```

---

## GROUP BY Clause (v2.0+)

Group results by one or more columns and apply aggregate functions.

```sql
-- Count by category
SELECT category, COUNT(*) FROM products GROUP BY category

-- Multiple aggregates
SELECT category, COUNT(*), AVG(price), MAX(rating)
FROM products
GROUP BY category

-- Nested payload fields
SELECT payload.author, payload.metadata.language, COUNT(*)
FROM articles
GROUP BY payload.author, payload.metadata.language

-- With vector similarity threshold
SELECT category, COUNT(*)
FROM documents
WHERE similarity(embedding, $query) > 0.6
GROUP BY category
ORDER BY COUNT(*) DESC
LIMIT 10
```

### Vector-Search GROUP BY (Parent-Document Retrieval, v3.7+)

When combined with a vector NEAR search, GROUP BY enables parent-document
retrieval from chunked collections. This groups search results by a parent
field and aggregates similarity scores across chunks.

Use `MAX(score)` for ColBERT-style MaxSim (max score across chunks),
`AVG(score)` for mean similarity, and `FIRST(column)` to surface the
excerpt from the highest-scoring chunk.

```sql
-- Parent-document retrieval with MAX_SIM scoring
SELECT parent_id, MAX(score) AS relevance, FIRST(text) AS excerpt
FROM chunks
WHERE vector NEAR $embedding
GROUP BY parent_id
ORDER BY relevance DESC
LIMIT 20

-- AVG_SIM scoring (mean across chunks)
SELECT document_id, AVG(score) AS avg_sim
FROM paragraphs
WHERE vector NEAR $query
GROUP BY document_id
ORDER BY avg_sim DESC
LIMIT 10

-- Combined with metadata filter
SELECT parent_id, MAX(score) AS relevance, FIRST(text) AS excerpt
FROM chunks
WHERE vector NEAR $v AND category = 'science'
GROUP BY parent_id
ORDER BY relevance DESC
LIMIT 10
```

| Function | Description | Requires |
|----------|-------------|----------|
| `MAX(score)` | Maximum similarity score across chunks in group | `NEAR` + `GROUP BY` |
| `AVG(score)` | Mean similarity score across chunks in group | `NEAR` + `GROUP BY` |
| `FIRST(col)` | Value of `col` from the highest-scoring chunk | `GROUP BY` |

Behavior:
- Chunks missing the GROUP BY field are silently skipped
- `FIRST(col)` returns `null` if the best chunk lacks the specified column
- Multiple `FIRST` projections all come from the same highest-scoring chunk
- `FIRST(*)` is not supported (parse error)
- `MAX(score)` / `AVG(score)` without `NEAR` returns an error
- `FIRST(col)` without `GROUP BY` returns an error

---

## HAVING Clause (v2.0+)

Filter groups after aggregation. HAVING conditions compare aggregate functions
against values using `AND`/`OR`.

```sql
-- Single condition
SELECT category, COUNT(*) FROM docs
GROUP BY category
HAVING COUNT(*) > 10

-- Multiple conditions
SELECT category, COUNT(*) FROM items
GROUP BY category
HAVING COUNT(*) > 10 AND AVG(price) < 100

-- OR logic
SELECT region, SUM(amount) FROM sales
GROUP BY region
HAVING SUM(amount) > 1000 OR COUNT(*) > 50

-- Full pipeline: GROUP BY + HAVING + ORDER BY
SELECT category, COUNT(*) AS total, AVG(price) AS avg_price
FROM products
GROUP BY category
HAVING COUNT(*) > 5
ORDER BY AVG(price) DESC
```

---

## ORDER BY Clause (v2.0+)

Sort results by one or more expressions, each with an optional direction.

### Direction

| Keyword | Description |
|---------|-------------|
| `ASC` | Ascending (default) |
| `DESC` | Descending |

### Basic Sorting

```sql
SELECT * FROM docs ORDER BY created_at DESC
SELECT * FROM docs ORDER BY category ASC, price DESC
```

### Order by Similarity

```sql
-- Highest similarity first
SELECT * FROM docs
WHERE vector NEAR $v
ORDER BY similarity() DESC
LIMIT 10

-- Two-argument similarity
SELECT * FROM docs
WHERE similarity(embedding, $query) > 0.5
ORDER BY similarity(embedding, $query) DESC
LIMIT 10

-- Multi-column with similarity
SELECT * FROM docs
WHERE vector NEAR $v
ORDER BY similarity() DESC, created_at DESC
LIMIT 20
```

### Order by Aggregate

```sql
SELECT category, COUNT(*) AS cnt
FROM products
GROUP BY category
ORDER BY COUNT(*) DESC
LIMIT 10
```

### Arithmetic Scoring (v3.0+)

Combine multiple score components with arithmetic expressions in ORDER BY:

```sql
-- Weighted hybrid scoring: 70% vector + 30% text relevance
SELECT * FROM docs
WHERE vector NEAR $v AND content MATCH 'machine learning'
ORDER BY 0.7 * vector_score + 0.3 * bm25_score DESC
LIMIT 10

-- Average of two scores
SELECT * FROM docs
WHERE vector NEAR $v
ORDER BY (vector_score + graph_score) / 2 DESC
LIMIT 10

-- Boost similarity
SELECT * FROM docs
WHERE vector NEAR $v
ORDER BY similarity() * 2 + 1 ASC
LIMIT 10

-- Hybrid: vector + BM25 + graph
SELECT * FROM docs
WHERE vector NEAR $v AND content MATCH 'AI'
ORDER BY 0.5 * similarity() + 0.5 * bm25_score DESC
LIMIT 10
```

Arithmetic expressions support `+`, `-`, `*`, `/`, parentheses, numeric
literals, score variables, and `similarity()`.

#### Score Variables

| Variable | Source | When Populated |
|----------|--------|----------------|
| `vector_score` | HNSW dense search | `NEAR` clause present |
| `bm25_score` | BM25 full-text search | `MATCH` text clause present |
| `sparse_score` | Sparse vector search | `SPARSE_NEAR` clause present |
| `graph_score` | Graph traversal | `MATCH` graph pattern present |
| `fused_score` | After fusion pipeline | `USING FUSION` applied |
| `similarity()` | Primary search score | Any search clause |

Component scores are populated independently by each search path. In hybrid
queries (NEAR + MATCH text), `vector_score` and `bm25_score` have different
values reflecting each component's individual contribution before fusion.

| Query Type | `vector_score` | `bm25_score` | `sparse_score` | `graph_score` |
|------------|----------------|--------------|-----------------|---------------|
| NEAR only | Populated | 0 | 0 | 0 |
| MATCH text only | 0 | Populated | 0 | 0 |
| NEAR + MATCH | Populated | Populated | 0 | 0 |
| SPARSE_NEAR | 0 | 0 | Populated | 0 |
| MATCH (graph) | 0 | 0 | 0 | Populated |

When a score variable is not populated, it defaults to 0.
`similarity()` always returns the primary search score regardless of type.
`fused_score` is populated after `USING FUSION` is applied.

---

## LIMIT and OFFSET

```sql
-- Limit results
SELECT * FROM docs LIMIT 10

-- Pagination: skip first 20, return next 10
SELECT * FROM docs LIMIT 10 OFFSET 20
```

Always specify `LIMIT` for vector search queries to bound the result set.

---

## LET Clause (Score Bindings, v3.2+)

Define named score bindings evaluated once and reusable in SELECT and ORDER BY.
LET clauses appear **before** the SELECT statement.

### Syntax

```sql
LET <name> = <arithmetic_expression>
```

### Rules

- LET clauses appear **before** the SELECT statement.
- Each binding has a name and an arithmetic expression.
- Bindings can reference earlier bindings (forward references are invalid).
- Available in ORDER BY and SELECT projection.
- **NOT** available in WHERE (WHERE runs before scores exist).
- LET names take **highest priority** in variable resolution (overrides component scores).
- Case-insensitive keyword (`LET`, `let`, `Let` all work).

### Expression Support

LET expressions support the same arithmetic as ORDER BY:

```sql
LET boost = 1.5
LET weighted = 0.7 * vector_score + 0.3 * bm25_score
LET final_score = weighted * boost
LET sim = similarity()
```

Available variables: `vector_score`, `bm25_score`, `graph_score`, `sparse_score`,
`fused_score`, `similarity()`, numeric literals, and earlier LET bindings.

### Examples

```sql
-- RAG scoring: blend vector similarity with text relevance
LET relevance = 0.7 * vector_score + 0.3 * bm25_score
SELECT *, relevance AS score
FROM documents
WHERE vector NEAR $query AND content MATCH 'machine learning'
ORDER BY relevance DESC
LIMIT 10

-- Chained bindings: later bindings can reference earlier ones
LET base = 0.6 * vector_score + 0.4 * bm25_score
LET boosted = base * 1.5
SELECT * FROM docs
WHERE vector NEAR $query AND content MATCH 'AI'
ORDER BY boosted DESC
LIMIT 5

-- LET with MATCH graph query
LET x = similarity()
MATCH (a)-[r]->(b)
RETURN a
LIMIT 5
```

---

## WITH Clause (Search Options)

Control search behavior with per-query configuration overrides.

```sql
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
WITH (mode = 'accurate', ef_search = 512, timeout_ms = 5000)
```

### Available Options

| Option | Type | Values | Description |
|--------|------|--------|-------------|
| `mode` | string | `fast`, `balanced`, `accurate`, `perfect`, `autotune` | Search quality preset (maps to ef_search: 64/128/512/4096/auto) |
| `quality` | string | same as `mode` | Alias for `mode` (v3.5+). If both are set, `mode` takes precedence. |
| `ef_search` | integer | 16--4096 | HNSW ef_search parameter (overrides `mode`) |
| `timeout_ms` | integer | >= 100 | Per-query timeout in milliseconds |
| `rerank` | boolean | `true`/`false` | Two-stage SIMD reranking (retrieves 4x candidates, re-ranks with exact distance) |
| `quantization` | string | `f32`, `int8`, `dual`, `auto` | Quantization mode for search |
| `oversampling` | float | >= 1.0 | Oversampling ratio for dual-precision mode |

### Examples

```sql
-- Fast search for autocomplete
SELECT * FROM suggestions WHERE vector NEAR $v LIMIT 5 WITH (mode = 'fast')

-- High accuracy for production
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (mode = 'accurate')

-- Custom ef_search
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (ef_search = 512)

-- Combined options
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
WITH (mode = 'balanced', ef_search = 256, rerank = true)

-- Dual quantization with oversampling
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
WITH (quantization = 'dual', oversampling = 2)

-- quality alias for mode (v3.5+)
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (quality = 'accurate')
```

---

## USING FUSION -- Hybrid Search (v2.0+)

Combine multiple search strategies with result fusion.

### Syntax

```sql
SELECT * FROM docs
WHERE vector NEAR $v AND content MATCH 'query'
USING FUSION(strategy = 'rrf', k = 60)
```

### Fusion Strategies

| Strategy | Description | Parameters | Use Case |
|----------|-------------|------------|----------|
| `rrf` | Reciprocal Rank Fusion | `k` (default: 60) | Balanced ranking (default) |
| `weighted` | Weighted combination | `weights = [w1, w2]` | Custom importance |
| `maximum` | Take highest score | (none) | Best match wins |
| `rsf` | Reciprocal Score Fusion | `dense_weight`, `sparse_weight` | Dense + sparse blending |

### Examples

```sql
-- Default RRF fusion
SELECT * FROM docs
WHERE vector NEAR $v AND content MATCH 'neural networks'
USING FUSION(strategy = 'rrf', k = 60)
LIMIT 10

-- Weighted fusion (70% vector, 30% text)
SELECT * FROM docs
WHERE vector NEAR $semantic AND content MATCH $keywords
USING FUSION(strategy = 'weighted', weights = [0.7, 0.3])
LIMIT 20

-- Dense + sparse hybrid with RSF
SELECT * FROM docs
WHERE vector NEAR $dense AND vector SPARSE_NEAR $sparse
USING FUSION(strategy = 'rsf', dense_weight = 0.7, sparse_weight = 0.3)
LIMIT 10

-- Maximum score fusion
SELECT * FROM docs
WHERE similarity(embedding, $q1) > 0.5
USING FUSION(strategy = 'maximum')
LIMIT 10
```

### FUSE BY (Planned Syntax)

> **PLANNED**: `FUSE BY` is not yet implemented in the grammar.
> Use `USING FUSION(...)` instead for all hybrid search queries.

---

## Set Operations (v2.0+)

Combine results from multiple SELECT queries. N-ary chaining is supported.

> **Precedence note:** VelesQL evaluates set operators strictly **left-to-right**,
> unlike standard SQL where `INTERSECT` binds tighter than `UNION`. Use
> parenthesized subqueries if different evaluation order is needed.

### UNION

Combine results, removing duplicates:

```sql
SELECT id, title FROM articles WHERE category = 'tech'
UNION
SELECT id, title FROM articles WHERE category = 'science'
```

### UNION ALL

Combine results, keeping duplicates:

```sql
SELECT * FROM table1 WHERE similarity(v, $q) > 0.8
UNION ALL
SELECT * FROM table2 WHERE similarity(v, $q) > 0.8
```

### INTERSECT

Only rows present in both queries:

```sql
SELECT item_id FROM user_likes WHERE user_id = 1
INTERSECT
SELECT item_id FROM user_likes WHERE user_id = 2
```

### EXCEPT

Rows in first query but not in second:

```sql
SELECT id FROM all_items
EXCEPT
SELECT id FROM purchased_items
```

### Three-Way Set Operations

```sql
-- Evaluated left-to-right: (A UNION B) UNION C
SELECT id FROM recent_docs WHERE created_at > 1000
UNION
SELECT id FROM popular_docs WHERE views > 100
UNION
SELECT id FROM featured_docs WHERE featured = TRUE

-- Mixed operators: (A UNION B) INTERSECT C
SELECT * FROM a UNION SELECT * FROM b INTERSECT SELECT * FROM c
```

---

## MATCH Statement (Graph Queries, v2.1+)

Execute graph pattern matching queries with optional vector similarity filtering.

### Syntax

```sql
MATCH <pattern>
[WHERE <conditions>]
RETURN <projection>
[ORDER BY <expression>]
[LIMIT <n>]
```

### Pattern Syntax

#### Node Patterns

A node is enclosed in parentheses with optional alias, labels, and properties:

```sql
(alias)                          -- Any node
(alias:Label)                    -- Node with label
(alias:Label1:Label2)            -- Node with multiple labels
(alias:Label {prop: value})      -- Node with property filter
(:Label)                         -- Anonymous node with label
()                               -- Anonymous node (any)
(alias:Label@collection)         -- Node resolved from a specific collection (cross-collection)
```

#### Cross-Collection Annotation (`@collection`)

The `@collection` suffix on a node pattern specifies which collection the node's data
should be resolved from. This enables cross-collection MATCH queries where the graph
traversal runs on one collection (the primary/edge collection) and node payloads are
enriched from other collections.

```sql
-- Products in 'catalog' graph, enriched with pricing from 'inventory' collection
MATCH (p:Product)-[:STORED_IN]->(w:Warehouse@inventory)
RETURN p.name, w.price, w.stock
LIMIT 20
```

When using `@collection`:
- The graph traversal (edges) runs on the primary collection specified via `_collection` param or `FROM` clause
- Node payloads are looked up from the annotated collection after traversal
- Enriched fields are prefixed with the node alias (e.g., `w.price`, `w.stock`)
- If the annotated collection doesn't exist, the enrichment is silently skipped

#### Relationship Patterns

Relationships connect nodes with direction and optional type/range:

```sql
-[r:TYPE]->                      -- Outgoing, named
<-[r:TYPE]-                      -- Incoming, named
-[r:TYPE]-                       -- Undirected, named
-[:TYPE]->                       -- Outgoing, anonymous
-[r:TYPE1|TYPE2]->               -- Multiple relationship types
-[*1..3]->                       -- Variable-length (1 to 3 hops)
-[r:TYPE *2..5]->                -- Named with type and range
-[*]->                           -- Any length
```

#### Range Specification

| Syntax | Meaning |
|--------|---------|
| `*1..3` | Between 1 and 3 hops |
| `*..5` | Up to 5 hops |
| `*2..` | At least 2 hops |
| `*3` | Exactly 3 hops |
| `*` | Any number of hops |

### RETURN Clause

Project fields from matched nodes and relationships:

```sql
RETURN a.name, b.title             -- Property access
RETURN a.name AS author            -- With alias
RETURN *                           -- All bound variables
RETURN similarity()                -- Pre-computed similarity score
RETURN a.name, similarity() AS score
```

### Full Examples

```sql
-- Simple graph traversal
MATCH (a:Person)-[:KNOWS]->(b:Person)
RETURN a.name, b.name

-- With similarity filter (RAG use case)
MATCH (doc:Document)
WHERE similarity(doc.embedding, $query) > 0.8
RETURN doc.title, doc.content
ORDER BY similarity() DESC
LIMIT 5

-- Multi-hop traversal
MATCH (a:Person)-[:KNOWS]->(b:Person)-[:WORKS_AT]->(c:Company)
WHERE a.department = 'engineering'
RETURN a.name, b.name, c.name
LIMIT 20

-- Variable-length paths
MATCH (start:Document)-[*1..3]->(end:Document)
WHERE start.topic = 'AI'
RETURN start.title, end.title

-- Bidirectional relationship
MATCH (a:Person)-[:COLLABORATES]-(b:Person)
RETURN a.name, b.name

-- Multiple relationship types
MATCH (p:Person)-[r:KNOWS|WORKS_WITH]->(q:Person)
RETURN p.name, q.name
LIMIT 10

-- Combined graph + vector + column (AIOps)
MATCH (incident:Ticket)-[:IMPACTS]->(service:Microservice)
WHERE similarity(incident.log_embedding, $error_vec) > 0.85
  AND incident.status = 'RESOLVED'
  AND service.criticality = 'HIGH'
RETURN incident.solution, service.name
ORDER BY similarity() DESC
LIMIT 3
```

### Scope and Requirements

**Collection Scope**: MATCH patterns operate within the current collection's
internal graph store. They do NOT traverse across different collections.

**Label Requirement**: Points must have a `_labels` array in their payload:

```json
{ "_labels": ["Product"], "name": "Headphones", "price": 99.99 }
```

**Edge Requirement**: Edges must exist in the collection's edge store. Create
them via `collection.add_edge()` (Rust/Python) or the REST API.

### Execution Strategies

The query planner automatically chooses the optimal strategy:

| Strategy | When Used | Description |
|----------|-----------|-------------|
| **GraphFirst** | No similarity condition | Traverse graph, then filter |
| **VectorFirst** | Similarity on start node | Vector search, then validate graph |
| **Parallel** | Large collection, high threshold | Execute both in parallel |

### REST API

```
POST /collections/{name}/match
Content-Type: application/json

{
  "query": "MATCH (a:Person)-[:KNOWS]->(b) WHERE similarity(a.vec, $v) > 0.8 RETURN a.name",
  "params": { "v": [0.1, 0.2, ...] }
}
```

---

## Introspection Statements (v3.4+)

### SHOW COLLECTIONS

Lists all collections in the database with their names and types.

```sql
-- List all collections
SHOW COLLECTIONS

-- With trailing semicolon
SHOW COLLECTIONS;
```

Returns one row per collection, each containing:

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Collection name |
| `type` | string | Collection type: `vector`, `graph`, or `metadata` |

### DESCRIBE COLLECTION

Returns metadata about a specific collection. The `COLLECTION` keyword is optional.

```sql
-- With COLLECTION keyword
DESCRIBE COLLECTION docs

-- Without COLLECTION keyword (equivalent)
DESCRIBE docs
```

Returns a single row with:

| Field | Type | Present When |
|-------|------|-------------|
| `name` | string | Always |
| `type` | string | Always (`vector`, `graph`, or `metadata`) |
| `dimension` | integer | Vector collections |
| `metric` | string | Vector collections |
| `point_count` | integer | All types |

Returns an error if the collection does not exist.

### EXPLAIN

Returns the query execution plan for a SELECT query without executing it.

```sql
-- Explain a simple query
EXPLAIN SELECT * FROM docs LIMIT 10

-- Explain a complex query
EXPLAIN SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
```

Returns a single row with:

| Field | Type | Description |
|-------|------|-------------|
| `plan` | object | Structured query plan (JSON) |
| `tree` | string | Human-readable plan tree |

The `tree` field now surfaces additional context when the query uses `WITH`, `LET`, or `FUSION`
clauses:

- **`WITH` options** — `ef_search`, `mode`, `rerank`, `timeout_ms` are read from the `WITH` clause
  and displayed as-is (no longer hardcoded to 100 for `ef_search`).
- **`LET` bindings** — each named binding is listed under a `LET bindings:` node.
- **`FUSION` details** — strategy name, `k`, and per-source weights appear under a `FUSION:` node.

Example enriched output:

```
── VectorSearch (docs) ef_search=512
   ├── WITH options:
   │   ├── ef_search: 512
   │   ├── mode: accurate
   │   └── rerank: true
   ├── LET bindings:
   │   └── boost = vector_score * 1.5
   ├── FUSION: rrf (k=60, vector_weight=0.7, bm25_weight=0.3)
   ├── Filter (PreFilter)
   └── Limit 10
```

---

## DDL Statements (v3.3+) and Admin Statements (v3.5+)

### CREATE COLLECTION

Create vector, graph, or metadata-only collections.

#### Vector Collection

```sql
-- Minimal: dimension only (metric defaults to cosine)
CREATE COLLECTION docs (dimension = 768)

-- Explicit metric
CREATE COLLECTION docs (dimension = 768, metric = 'euclidean')

-- With storage quantization and HNSW parameters
CREATE COLLECTION docs (dimension = 768, metric = 'cosine')
  WITH (storage = 'sq8', m = 16, ef_construction = 200)

-- Dot product metric
CREATE COLLECTION embeddings (dimension = 384, metric = 'dotproduct')
```

#### Graph Collection

```sql
-- Schemaless graph with embeddings
CREATE GRAPH COLLECTION knowledge (dimension = 768, metric = 'cosine') SCHEMALESS

-- Schemaless pure graph (no embeddings)
CREATE GRAPH COLLECTION pure_graph

-- Typed graph with schema definition
CREATE GRAPH COLLECTION ontology (dimension = 768, metric = 'cosine') WITH SCHEMA (
  NODE Person (name: STRING, age: INTEGER),
  NODE Document (title: STRING, year: INTEGER),
  EDGE AUTHORED_BY FROM Person TO Document,
  EDGE CITED_BY FROM Document TO Document
)
```

#### Metadata Collection

```sql
-- No vectors, metadata-only storage
CREATE METADATA COLLECTION tags
```

#### CREATE Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `dimension` | integer | Yes (vector/graph) | -- | Embedding dimension |
| `metric` | string | No | `cosine` | Distance metric: `cosine`, `euclidean`, `dotproduct` |

#### WITH Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `storage` | string | `full` | Storage mode: `full`, `sq8`, `binary` |
| `m` | integer | 16 | HNSW M parameter (max links per node) |
| `ef_construction` | integer | 200 | HNSW build-time expansion factor |

#### Schema Type Names

| Type | Description |
|------|-------------|
| `STRING` | Text value |
| `INTEGER` | Integer value |
| `FLOAT` | Floating-point value |
| `BOOLEAN` | True/false value |
| `VECTOR` | Vector field (special) |

### DROP COLLECTION

```sql
-- Error if collection not found
DROP COLLECTION documents

-- No error if absent
DROP COLLECTION IF EXISTS documents
```

### Index Management (v3.5+)

Create or drop secondary metadata indexes on collection payload fields.
Secondary indexes use a BTree structure for O(log n) equality lookups on
metadata fields, accelerating WHERE-clause filters.

#### CREATE INDEX

```sql
-- Create a secondary index on a metadata field
CREATE INDEX ON docs (category)

-- Case-insensitive keywords
create index on docs (status)
```

Index creation is **idempotent** -- creating an index that already exists is a
no-op.

#### DROP INDEX

```sql
-- Remove a secondary metadata index
DROP INDEX ON docs (category)
```

Dropping a non-existent index succeeds silently (no error).

#### Notes

- Secondary indexes only apply to **metadata payload fields** (not vector data).
- Indexes accelerate equality filters in WHERE clauses (e.g., `WHERE category = 'tech'`).
- The indexed field must already contain data for the index to be populated on
  subsequent upserts. Existing data is not retroactively indexed.
- Indexes are in-memory only; they are not persisted to disk in the current
  implementation.

### ANALYZE (v3.5+)

Computes cost-based optimizer (CBO) statistics for a collection. The statistics
are cached in memory and persisted to disk (`collection.stats.json`).

```sql
-- Compute statistics for query optimizer
ANALYZE docs
ANALYZE COLLECTION docs   -- optional COLLECTION keyword
```

Returns a JSON payload with collection statistics including `total_points`,
`row_count`, `deleted_count`, `avg_row_size_bytes`, `payload_size_bytes`,
and per-field cardinality information.

### TRUNCATE (v3.5+)

Deletes all rows from a collection without dropping the collection itself.
Unlike `DELETE FROM` which requires a `WHERE` clause, `TRUNCATE` removes
all data unconditionally.

```sql
-- Delete all rows
TRUNCATE docs
TRUNCATE COLLECTION docs   -- optional COLLECTION keyword
```

Returns a payload with `{"deleted_count": N}`. Truncating an empty collection
succeeds with `deleted_count: 0`. The collection structure (config, indexes)
is preserved after truncation.

### ALTER COLLECTION (v3.5+)

Modifies collection settings at runtime. Uses the same key-value option
syntax as `CREATE COLLECTION`.

```sql
-- Enable auto-reindex
ALTER COLLECTION docs SET (auto_reindex = true)

-- Disable auto-reindex
ALTER COLLECTION docs SET (auto_reindex = false)
```

**Supported options:**

| Option | Type | Description |
|--------|------|-------------|
| `auto_reindex` | boolean | Enable/disable automatic HNSW parameter tuning |

Unknown options are rejected with an error message listing supported options.

### FLUSH (v3.6+)

Persists collection data to disk. Supports fast flush (WAL only) and full flush
(includes index serialization). Can target all collections or a specific one.

```sql
-- Fast flush all collections (WAL only)
FLUSH

-- Full flush all collections (includes index serialization)
FLUSH FULL

-- Fast flush a specific collection
FLUSH docs

-- Full flush a specific collection
FLUSH FULL docs
```

Returns a payload with `{"status": "flushed", "full": true/false}`.

- **Fast flush** (`FLUSH`): Writes WAL data to disk. Fast but requires WAL replay
  on next startup if the process crashes before a full flush.
- **Full flush** (`FLUSH FULL`): Writes WAL data and serializes vector indexes.
  Slower but ensures fast startup without WAL replay. Recommended before
  graceful shutdown.

Flushing a nonexistent collection returns a `CollectionNotFound` error.
Flushing an empty database (no collections) succeeds with no error.

> **Note:** `full` (case-insensitive) is a reserved modifier for FLUSH. To flush a
> collection named `full`, use `FLUSH FULL full` or quote the identifier:
> `` FLUSH `full` ``.

---

## DML Statements (INSERT, UPDATE, DELETE)

### INSERT INTO (v3.2+, multi-row v3.5+)

Insert one or more rows into a collection. Column names are listed in parentheses,
followed by corresponding values. Multiple rows are separated by commas (v3.5+).

```sql
-- Insert a single row
INSERT INTO docs (id, title, category) VALUES (1, 'Getting Started', 'tutorial')

-- Insert with numeric values
INSERT INTO products (id, price, stock) VALUES (42, 29.99, 100)

-- Insert with boolean values
INSERT INTO flags (id, active, verified) VALUES (1, TRUE, FALSE)

-- Insert with NULL
INSERT INTO events (id, description, metadata) VALUES (10, 'system boot', NULL)

-- Insert with parameter-bound vector
INSERT INTO docs (id, vector, category) VALUES (1, $vector, 'test')

-- Multi-row insert (v3.5+)
INSERT INTO docs (id, vector, title) VALUES (1, $v1, 'Hello'), (2, $v2, 'World'), (3, $v3, 'Foo')
```

Column count must match value count **in every row**. Types are inferred from
literal values. Multi-row INSERT executes as a single batch `upsert()` call.

### UPSERT INTO (v3.5+)

Insert or update rows. Identical syntax to INSERT; semantically equivalent since
VelesDB's INSERT is always idempotent (existing IDs are overwritten). UPSERT
makes this intent explicit at the SQL level.

```sql
-- Single-row upsert
UPSERT INTO docs (id, vector, title) VALUES (1, $v, 'Updated')

-- Multi-row upsert
UPSERT INTO docs (id, vector, title) VALUES (1, $v1, 'A'), (2, $v2, 'B')
```

### UPDATE (v3.2+)

Update one or more fields in existing rows.

```sql
-- Update a single field
UPDATE docs SET status = 'archived' WHERE id = 42

-- Update multiple fields
UPDATE products SET price = 19.99, stock = 50 WHERE category = 'sale'

-- Update with boolean
UPDATE products SET featured = TRUE WHERE id = 1

-- Update without WHERE (updates all rows -- use with caution)
UPDATE products SET featured = FALSE
```

The WHERE clause is optional but strongly recommended.

### DELETE FROM (v3.3+)

Delete rows from a collection. The WHERE clause is **mandatory** to prevent
accidental full-collection deletion.

```sql
-- Delete by single ID
DELETE FROM docs WHERE id = 42

-- Delete by multiple IDs
DELETE FROM docs WHERE id IN (1, 2, 3)

-- Delete with string comparison
DELETE FROM logs WHERE level = 'debug'

-- Delete with compound condition
DELETE FROM events WHERE status = 'expired' AND created_at < 1000
```

> VelesQL rejects `DELETE FROM <collection>` without a WHERE predicate as a safety
> measure.

### INSERT EDGE (v3.3+)

Insert a labeled edge between two nodes in a graph collection.

```sql
-- Basic edge insertion
INSERT EDGE INTO knowledge (source = 1, target = 2, label = 'AUTHORED_BY')

-- With explicit edge ID
INSERT EDGE INTO knowledge (id = 10, source = 1, target = 2, label = 'KNOWS')

-- With edge properties
INSERT EDGE INTO knowledge (source = 1, target = 2, label = 'AUTHORED_BY')
  WITH PROPERTIES (year = 2026, confidence = 0.95)

-- With ID and properties
INSERT EDGE INTO kg (id = 20, source = 2, target = 3, label = 'KNOWS')
  WITH PROPERTIES (weight = 0.9)
```

#### Edge Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `source` | integer | Yes | Source node ID |
| `target` | integer | Yes | Target node ID |
| `label` | string | Yes | Edge relationship type |
| `id` | integer | No | Explicit edge ID (auto-generated if omitted) |

WITH PROPERTIES accepts key-value pairs (strings, integers, or floats).

### DELETE EDGE (v3.3+)

Remove an edge by its ID from a graph collection.

```sql
DELETE EDGE 123 FROM knowledge
DELETE EDGE 10 FROM kg
```

### SELECT EDGES (v3.5+)

Query edges from a graph collection with optional filtering by source node,
target node, or edge label.

```sql
-- All edges
SELECT EDGES FROM kg LIMIT 100

-- Outgoing edges from a node
SELECT EDGES FROM kg WHERE source = 1

-- Incoming edges to a node
SELECT EDGES FROM kg WHERE target = 2

-- Edges by label
SELECT EDGES FROM kg WHERE label = 'KNOWS'

-- Combined: outgoing with label
SELECT EDGES FROM kg WHERE source = 1 AND label = 'KNOWS'
```

Each result contains a JSON payload with `edge_id`, `source`, `target`, `label`,
and `properties` fields.

> **Limitation:** WHERE supports at most two conditions combined with AND. Three-condition
> queries like `WHERE source = 1 AND label = 'X' AND target = 2` are not supported.
> Use two-condition AND and filter results client-side if needed.

### INSERT NODE (v3.5+)

Insert or update a node's payload in a graph collection.

```sql
-- With JSON payload
INSERT NODE INTO kg (id = 42, payload = '{"name": "Alice", "_labels": ["Person"]}')

-- With inline fields (collected into a JSON object)
INSERT NODE INTO kg (id = 10, name = 'Bob', age = 30)
```

If a node with the given `id` already exists, the payload is replaced (upsert
semantics).

---

## TRAIN QUANTIZER Command (v2.2+)

Explicitly train a Product Quantizer on a collection's vector data.

### Syntax

```sql
TRAIN QUANTIZER ON <collection> WITH (<parameters>)
```

### Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `m` | integer | 8 | Number of subspaces |
| `k` | integer | 256 | Codebook size per subspace |
| `type` | string | -- | Quantizer type: `pq`, `opq`, `rabitq` |
| `oversampling` | integer | -- | Training oversampling ratio |
| `sample` | integer | -- | Number of vectors to sample |
| `force` | boolean | false | Force retrain if exists |

### Examples

```sql
-- Train with standard parameters
TRAIN QUANTIZER ON my_embeddings WITH (m = 8, k = 256)

-- Higher compression (more subspaces)
TRAIN QUANTIZER ON large_collection WITH (m = 16, k = 256)

-- Finer codebooks
TRAIN QUANTIZER ON precision_collection WITH (m = 8, k = 512)

-- With quantizer type
TRAIN QUANTIZER ON vectors WITH (m = 16, k = 256, type = opq)

-- Force retrain with oversampling
TRAIN QUANTIZER ON my_coll WITH (m = 8, k = 256, oversampling = 4, force = true)

-- Semicolons optional
TRAIN QUANTIZER ON my_collection WITH (m = 8, k = 256);
```

### Notes

- Training is explicit -- it does not happen automatically.
- The collection must contain enough vectors (recommended: at least `k`).
- Re-training overwrites the existing quantizer.
- OPQ can be enabled via the `type` parameter.

---

## Agent Memory VelesQL Queries (v3.2+)

The Agent Memory SDK creates three internal collections queryable via VelesQL:

### Semantic Memory (`_semantic_memory`)

```sql
SELECT * FROM _semantic_memory
WHERE vector NEAR $query_embedding
LIMIT 10 WITH (mode = 'accurate')
```

Payload fields: `content` (string).

### Episodic Memory (`_episodic_memory`)

```sql
-- Recent events (last 24h)
SELECT * FROM _episodic_memory
WHERE timestamp > 1711234567
ORDER BY timestamp DESC
LIMIT 20

-- Similarity search on events
SELECT * FROM _episodic_memory
WHERE vector NEAR $query_embedding
LIMIT 5
```

Payload fields: `description` (string), `timestamp` (integer, Unix epoch).

### Procedural Memory (`_procedural_memory`)

```sql
SELECT * FROM _procedural_memory
WHERE confidence > 0.7
ORDER BY confidence DESC
LIMIT 10
```

Payload fields: `name` (string), `steps` (array), `confidence` (float),
`usage_count` (integer), `created_at` (integer), `last_used_at` (integer).

### Convenience API (Rust/Python)

```rust
let results = agent_memory.query_semantic(
    "SELECT * FROM _semantic_memory WHERE vector NEAR $v LIMIT 5",
    &params,
)?;
```

All three subsystems support: vector NEAR, payload filters, ORDER BY,
LIMIT/OFFSET, WITH options, LET bindings, and USING FUSION.

VelesQL queryability of agent memory collections is verified by BDD integration
tests (`tests/bdd/agent_memory.rs`) that exercise the full
`Database::execute_query` pipeline, and by unit tests in
`src/agent/velesql_bridge_tests.rs` that test the `Collection::execute_query_str`
path.

---

## Value Types

### Strings

Single-quoted. To include a single quote, double it (`''`):

```sql
'hello world'
'O''Brien'                -- produces: O'Brien
'It''s a ''test'''        -- produces: It's a 'test'
''                        -- empty string
```

Unicode characters are fully supported:

```sql
'日本語テキスト'
'🚀 Launch'
```

> **Warning**: Backslash escaping (`\'`) is **not** supported. Always use `''`.

### Numbers

```sql
42            -- integer
3.14          -- float
-100          -- negative integer
-0.5          -- negative float
```

### Booleans

```sql
TRUE
FALSE
```

Case-insensitive: `true`, `True`, `TRUE` are all equivalent.

### Vectors

```sql
[0.1, 0.2, 0.3, 0.4]           -- literal vector (dense)
$query_vector                    -- parameter reference
```

### Sparse Vectors

```sql
{0: 0.9, 14: 0.7, 256: 0.5}   -- literal sparse vector
$sparse_query                    -- parameter reference
```

### NULL

```sql
NULL
```

### Temporal Values

```sql
NOW()                            -- current Unix timestamp
INTERVAL '7 days'               -- time duration
NOW() - INTERVAL '24 hours'     -- temporal arithmetic
```

---

## Parameters

Use `$name` syntax for parameterized queries. Parameters are resolved at runtime
from the query context.

```sql
SELECT * FROM docs WHERE vector NEAR $query_vector AND category = $cat LIMIT 10
```

Parameters can be used for:
- Vectors: `$v`, `$query_embedding`
- Sparse vectors: `$sparse`, `$sv`
- Scalar values: `$category`, `$min_price`

---

## Identifier Quoting

### Reserved Keywords

The following keywords cannot be used as identifiers without quoting:

| Category | Keywords |
|----------|----------|
| Query structure | `SELECT`, `FROM`, `WHERE`, `AS` |
| Logical operators | `AND`, `OR`, `NOT` |
| Comparison operators | `IN`, `BETWEEN`, `LIKE`, `ILIKE`, `MATCH` |
| NULL handling | `IS`, `NULL` |
| Boolean literals | `TRUE`, `FALSE` |
| Pagination | `LIMIT`, `OFFSET` |
| Sorting | `ORDER`, `BY`, `ASC`, `DESC` |
| Aggregation | `GROUP`, `HAVING` |
| Vector operations | `NEAR`, `NEAR_FUSED`, `SPARSE_NEAR`, `SIMILARITY` |
| Extensions | `FUSE`, `TRAIN`, `QUANTIZER`, `WITH` |
| DDL | `CREATE`, `DROP`, `COLLECTION`, `GRAPH`, `METADATA` |
| Graph/DML | `EDGE`, `SCHEMALESS`, `SCHEMA`, `NODE`, `PROPERTIES` |
| Conditional | `IF`, `EXISTS` |
| DML | `INSERT`, `INTO`, `UPDATE`, `SET`, `DELETE`, `VALUES` |
| Set operations | `UNION`, `ALL`, `INTERSECT`, `EXCEPT` |
| JOINs | `JOIN`, `INNER`, `LEFT`, `RIGHT`, `FULL`, `OUTER`, `ON`, `USING` |
| Bindings | `LET`, `RETURN`, `MATCH` |
| Temporal | `NOW`, `INTERVAL` |
| Misc | `DISTINCT`, `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`, `FIRST`, `CONTAINS` |

### Quoting Styles

To use reserved keywords or special characters as identifiers, quote them:

**Backtick quoting** (MySQL-style):

```sql
SELECT `select`, `from`, `order` FROM docs
```

**Double-quote quoting** (SQL standard):

```sql
SELECT "select", "from", "order" FROM docs
```

**Mixed styles** in the same query:

```sql
SELECT `select`, "order" FROM my_table WHERE `limit` > 10
```

### Escaping Quotes Inside Identifiers

Double the quote character to include it:

```sql
-- Column named: col"name
SELECT "col""name" FROM docs
```

### Examples

```sql
-- Reserved keyword as collection name
SELECT * FROM `order`

-- Reserved keywords in WHERE
SELECT * FROM docs WHERE `select` = 'value' AND `from` LIKE '%pattern%'

-- Reserved keywords in ORDER BY
SELECT * FROM docs ORDER BY `order` ASC

-- Reserved keywords in GROUP BY
SELECT `group`, COUNT(*) FROM docs GROUP BY `group`

-- Reserved keyword as alias
SELECT id AS `select` FROM docs
```

---

## Default Values Reference

| Feature | Default | Notes |
|---------|---------|-------|
| LIMIT | No limit (all results) | Always specify LIMIT for vector search |
| OFFSET | 0 | |
| ORDER BY direction | ASC | Explicit DESC recommended for similarity |
| metric (CREATE) | cosine | |
| storage (CREATE) | full | full = no quantization |
| m (HNSW) | 16 | |
| ef_construction | 200 | |
| WITH mode | balanced | |
| USING FUSION strategy | rrf | |
| USING FUSION k | 60 | |
| ef_search | Depends on mode | fast=32, balanced=64, accurate=256, perfect=512 |

---

## Error Handling

VelesQL returns structured errors:

| Error Type | Description |
|------------|-------------|
| `SyntaxError` | Invalid query syntax |
| `SemanticError` | Valid syntax but invalid semantics |
| `CollectionNotFound` | Referenced collection doesn't exist |
| `ColumnNotFound` | Referenced column doesn't exist |
| `TypeMismatch` | Incompatible types in comparison |
| `Timeout` | Query exceeded timeout_ms |

---

## Quick Reference

### Statement Types

| Statement | Purpose | Example |
|-----------|---------|---------|
| `SELECT` | Query data | `SELECT * FROM docs WHERE vector NEAR $v LIMIT 10` |
| `MATCH` | Graph traversal | `MATCH (a)-[:KNOWS]->(b) RETURN a.name` |
| `INSERT INTO` | Add rows (multi-row) | `INSERT INTO docs (id, title) VALUES (1, 'A'), (2, 'B')` |
| `UPSERT INTO` | Insert or update rows | `UPSERT INTO docs (id, title) VALUES (1, 'Updated')` |
| `UPDATE` | Modify rows | `UPDATE docs SET status = 'done' WHERE id = 1` |
| `DELETE FROM` | Remove rows | `DELETE FROM docs WHERE id = 42` |
| `INSERT EDGE` | Add graph edge | `INSERT EDGE INTO kg (source=1, target=2, label='REL')` |
| `DELETE EDGE` | Remove graph edge | `DELETE EDGE 123 FROM kg` |
| `SELECT EDGES` | Query graph edges | `SELECT EDGES FROM kg WHERE source = 1 LIMIT 10` |
| `INSERT NODE` | Add/update node payload | `INSERT NODE INTO kg (id = 42, payload = '{"name": "Alice"}')` |
| `CREATE COLLECTION` | Create collection | `CREATE COLLECTION docs (dimension=768)` |
| `DROP COLLECTION` | Delete collection | `DROP COLLECTION IF EXISTS docs` |
| `CREATE INDEX` | Add metadata index | `CREATE INDEX ON docs (category)` |
| `DROP INDEX` | Remove metadata index | `DROP INDEX ON docs (category)` |
| `TRAIN QUANTIZER` | Train compression | `TRAIN QUANTIZER ON docs WITH (m=8, k=256)` |
| `SHOW COLLECTIONS` | List collections | `SHOW COLLECTIONS` |
| `DESCRIBE` | Collection metadata | `DESCRIBE COLLECTION docs` |
| `EXPLAIN` | Query plan | `EXPLAIN SELECT * FROM docs LIMIT 10` |

### WHERE Operators

| Operator | Syntax | Example |
|----------|--------|---------|
| NEAR | `vector NEAR $v` | `WHERE vector NEAR $query` |
| SPARSE_NEAR | `vector SPARSE_NEAR $v` | `WHERE vector SPARSE_NEAR $sparse` |
| NEAR_FUSED | `vector NEAR_FUSED [$v1,$v2]` | `WHERE vector NEAR_FUSED [$a, $b]` |
| similarity() | `similarity(field, $v) op N` | `WHERE similarity(emb, $v) > 0.8` |
| MATCH (text) | `column MATCH 'text'` | `WHERE content MATCH 'database'` |
| `=` `!=` `>` `>=` `<` `<=` | `column op value` | `WHERE price > 100` |
| IN | `column IN (values)` | `WHERE id IN (1, 2, 3)` |
| GEO_DISTANCE | `GEO_DISTANCE(col, lat, lng) op meters` | `WHERE GEO_DISTANCE(location, 48.8566, 2.3522) < 500` |
| GEO_BBOX | `GEO_BBOX(col, lat_min, lng_min, lat_max, lng_max)` | `WHERE GEO_BBOX(location, 48.8, 2.3, 48.9, 2.4)` |

### Geospatial Functions

#### GEO_DISTANCE

Computes the Haversine great-circle distance in meters between a GeoPoint column value and a reference coordinate pair.

```sql
SELECT * FROM places WHERE GEO_DISTANCE(location, 48.8566, 2.3522) < 500;
SELECT * FROM places WHERE GEO_DISTANCE(location, 48, 2) >= 1000;
```

- Coordinates accept both float (`48.8566`) and integer (`48`) literals.
- Returns distance in meters (SI unit).
- Null GeoPoint values are excluded from results.
- Non-GeoPoint or non-existent columns return empty results (no error).
- Combinable with AND, OR, NOT, and other WHERE operators.

#### GEO_BBOX

Tests whether a GeoPoint column value falls within a latitude/longitude bounding box (inclusive).

```sql
SELECT * FROM places WHERE GEO_BBOX(location, 48.8, 2.3, 48.9, 2.4);
```

- All four coordinate parameters accept float and integer literals.
- Boundary is inclusive: points exactly on the edge are included.
- Inverted coordinates (`lat_min > lat_max`) return empty results.
- Null GeoPoint values are excluded from results.
| NOT IN | `column NOT IN (values)` | `WHERE status NOT IN ('deleted')` |
| BETWEEN | `column BETWEEN a AND b` | `WHERE price BETWEEN 10 AND 100` |
| LIKE | `column LIKE 'pattern'` | `WHERE name LIKE 'John%'` |
| ILIKE | `column ILIKE 'pattern'` | `WHERE name ILIKE '%john%'` |
| IS NULL | `column IS NULL` | `WHERE deleted_at IS NULL` |
| IS NOT NULL | `column IS NOT NULL` | `WHERE email IS NOT NULL` |
| AND / OR / NOT | logical combinators | `WHERE a > 1 AND (b = 2 OR c = 3)` |

### Clauses

| Clause | Purpose | Example |
|--------|---------|---------|
| `FROM ... AS` | Source with alias | `FROM docs AS d` |
| `WHERE` | Filter conditions | `WHERE status = 'active'` |
| `ORDER BY ... ASC/DESC` | Sort results | `ORDER BY similarity() DESC` |
| `LIMIT N` | Cap result count | `LIMIT 10` |
| `OFFSET N` | Skip first N results | `OFFSET 20` |
| `GROUP BY` | Aggregate grouping | `GROUP BY category` |
| `HAVING` | Filter groups | `HAVING COUNT(*) > 5` |
| `JOIN ... ON` | Combine collections | `JOIN users ON docs.author_id = users.id` |
| `WITH (...)` | Search options | `WITH (mode = 'accurate')` |
| `USING FUSION(...)` | Hybrid strategy | `USING FUSION(strategy = 'rrf')` |
| `LET x = expr` | Score binding | `LET score = 0.7 * vector_score` |
| `DISTINCT` | Deduplicate | `SELECT DISTINCT category FROM docs` |

### Aggregate Functions

| Function | Description | Example |
|----------|-------------|---------|
| `COUNT(*)` | Count rows | `SELECT COUNT(*) FROM docs` |
| `COUNT(col)` | Count non-null | `SELECT COUNT(email) FROM users` |
| `SUM(col)` | Sum values | `SELECT SUM(price) FROM orders` |
| `AVG(col)` | Average value | `SELECT AVG(rating) FROM reviews` |
| `MIN(col)` | Minimum value | `SELECT MIN(created_at) FROM logs` |
| `MAX(col)` | Maximum value | `SELECT MAX(score) FROM results` |
| `MAX(score)` | Max similarity in group | `SELECT MAX(score) AS rel FROM chunks ... GROUP BY parent_id` |
| `AVG(score)` | Mean similarity in group | `SELECT AVG(score) AS avg FROM chunks ... GROUP BY parent_id` |
| `FIRST(col)` | Value from best chunk | `SELECT FIRST(text) AS excerpt FROM chunks ... GROUP BY parent_id` |

### Value Types

| Type | Syntax | Example |
|------|--------|---------|
| String | `'single quotes'` | `'hello world'` |
| Integer | digits | `42`, `-7` |
| Float | digits.digits | `3.14`, `-0.5` |
| Boolean | `TRUE` / `FALSE` | `TRUE` |
| NULL | `NULL` | `NULL` |
| Vector | `[floats]` | `[0.1, 0.2, 0.3]` |
| Sparse Vector | `{idx: val}` | `{12: 0.8, 45: 0.3}` |
| Parameter | `$name` | `$query_vector` |
| Temporal | `NOW()`, `INTERVAL` | `NOW() - INTERVAL '7 days'` |

### Score Variables (for ORDER BY and LET)

| Variable | Source | When Populated |
|----------|--------|----------------|
| `vector_score` | HNSW dense search | NEAR clause present |
| `bm25_score` | Full-text BM25 | MATCH text clause present |
| `sparse_score` | Sparse vector search | SPARSE_NEAR clause present |
| `graph_score` | Graph traversal | MATCH graph pattern present |
| `fused_score` | After fusion | USING FUSION applied |
| `similarity()` | Primary search score | Any search clause |

---

## Complete Examples

### Vector Similarity Search

```sql
-- Basic: find 10 nearest documents
SELECT * FROM documents WHERE vector NEAR $query LIMIT 10

-- With similarity score in results
SELECT id, title, similarity() AS score
FROM documents WHERE vector NEAR $query
ORDER BY similarity() DESC LIMIT 5

-- With metadata filter
SELECT * FROM articles
WHERE vector NEAR $query AND category = 'science' AND year >= 2024
LIMIT 10

-- With search quality tuning
SELECT * FROM embeddings WHERE vector NEAR $query
ORDER BY similarity() DESC LIMIT 20
WITH (mode = 'accurate', ef_search = 256)
```

### Full-Text Search

```sql
-- Keyword search with BM25 scoring
SELECT * FROM articles WHERE content MATCH 'machine learning' LIMIT 10

-- Case-insensitive pattern matching
SELECT * FROM users WHERE name ILIKE '%john%'

-- Pattern with wildcards
SELECT * FROM products WHERE sku LIKE 'SKU-2024-%'
```

### Hybrid Search (Vector + Text)

```sql
-- Dense vector + BM25 text with RRF fusion
SELECT * FROM docs
WHERE vector NEAR $query AND content MATCH 'neural networks'
USING FUSION(strategy = 'rrf', k = 60)
LIMIT 10

-- With custom scoring weights
LET score = 0.7 * vector_score + 0.3 * bm25_score
SELECT *, score AS relevance
FROM documents
WHERE vector NEAR $query AND content MATCH 'transformer'
ORDER BY score DESC
LIMIT 10

-- Dense + sparse vector fusion
SELECT * FROM docs
WHERE vector NEAR $dense_query AND vector SPARSE_NEAR $sparse_query
USING FUSION(strategy = 'rrf', k = 60)
LIMIT 10

-- Multi-vector fusion (text + image embeddings)
SELECT * FROM products
WHERE vector NEAR_FUSED [$text_emb, $image_emb]
USING FUSION(strategy = 'weighted')
LIMIT 20
```

### Filtering and Aggregation

```sql
-- Complex WHERE conditions
SELECT * FROM products
WHERE category IN ('electronics', 'computers')
  AND price BETWEEN 100 AND 500
  AND brand IS NOT NULL
  AND name NOT IN ('discontinued')
LIMIT 50

-- Aggregation with grouping
SELECT category, COUNT(*) AS total, AVG(price) AS avg_price
FROM products
GROUP BY category
HAVING COUNT(*) > 5
ORDER BY avg_price DESC

-- Nested payload field access
SELECT payload.author, payload.metadata.language, COUNT(*)
FROM articles
GROUP BY payload.author, payload.metadata.language

-- Temporal filtering
SELECT * FROM events
WHERE timestamp > NOW() - INTERVAL '7 days'
ORDER BY timestamp DESC
LIMIT 100

-- Deduplicated results
SELECT DISTINCT category FROM products WHERE stock > 0
```

### Graph Queries

```sql
-- Basic traversal
MATCH (person:Person)-[:AUTHORED]->(doc:Document)
WHERE similarity(doc.embedding, $query) > 0.7
RETURN person.name, doc.title, similarity() AS score
ORDER BY similarity() DESC
LIMIT 10

-- Multi-hop traversal
MATCH (a:Person)-[:KNOWS]->(b:Person)-[:WORKS_AT]->(c:Company)
WHERE a.department = 'engineering'
RETURN a.name, b.name, c.name
LIMIT 20

-- Variable-length paths (1 to 3 hops)
MATCH (start:Document)-[*1..3]->(end:Document)
WHERE start.topic = 'AI'
RETURN start.title, end.title

-- Graph mutations
INSERT NODE INTO knowledge (id = 1, payload = '{"name": "Alice", "_labels": ["Author"]}');
INSERT EDGE INTO knowledge (source = 1, target = 2, label = 'AUTHORED_BY')
  WITH PROPERTIES (confidence = 0.95, year = 2026);

-- Query edges
SELECT EDGES FROM knowledge WHERE source = 1;
SELECT EDGES FROM knowledge WHERE label = 'AUTHORED_BY' LIMIT 10;

DELETE EDGE 42 FROM knowledge;
```

### Collection Management (DDL)

```sql
-- Create vector collection
CREATE COLLECTION documents (dimension = 768)

-- Create with explicit parameters
CREATE COLLECTION embeddings (dimension = 384, metric = 'euclidean')
  WITH (storage = 'sq8', m = 16, ef_construction = 200)

-- Create graph collection
CREATE GRAPH COLLECTION knowledge (dimension = 768, metric = 'cosine') SCHEMALESS

-- Create graph with typed schema
CREATE GRAPH COLLECTION ontology (dimension = 768) WITH SCHEMA (
  NODE Person (name: STRING, age: INTEGER),
  NODE Organization (name: STRING),
  EDGE WORKS_AT FROM Person TO Organization
)

-- Create metadata-only collection
CREATE METADATA COLLECTION tags

-- Drop collection safely
DROP COLLECTION IF EXISTS old_data

-- Create a secondary index on a metadata field
CREATE INDEX ON documents (category)

-- Drop a secondary index
DROP INDEX ON documents (category)
```

### Data Manipulation (DML)

```sql
-- Insert a row
INSERT INTO documents (id, title, category) VALUES (1, 'Getting Started', 'tutorial')

-- Update fields
UPDATE documents SET status = 'published', updated = TRUE WHERE id = 1

-- Delete with mandatory WHERE
DELETE FROM documents WHERE id IN (10, 11, 12)
DELETE FROM logs WHERE level = 'debug' AND timestamp < 1000
```

### Join Queries

```sql
-- Inner join
SELECT o.id, c.name, o.total
FROM orders AS o
JOIN customers AS c ON o.customer_id = c.id
WHERE o.total > 100

-- Self-join
SELECT a.title AS original, b.title AS related
FROM articles AS a
JOIN articles AS b ON a.parent_id = b.id
WHERE a.category = 'tech'

-- Join with USING shorthand
SELECT * FROM orders AS o
JOIN products AS p USING (product_id)
```

### Set Operations

```sql
-- Combine results from two queries
SELECT id FROM recommended_docs WHERE user_id = 1
UNION
SELECT id FROM trending_docs WHERE score > 0.8

-- Find common items
SELECT item_id FROM user_1_likes
INTERSECT
SELECT item_id FROM user_2_likes

-- Exclude items
SELECT id FROM all_docs
EXCEPT
SELECT id FROM archived_docs
```

### Advanced Scoring with LET

```sql
-- Weighted hybrid scoring
LET relevance = 0.6 * vector_score + 0.4 * bm25_score
SELECT id, title, relevance AS score
FROM articles
WHERE vector NEAR $query AND content MATCH 'deep learning'
ORDER BY relevance DESC
LIMIT 10

-- Chained bindings
LET base = 0.7 * vector_score + 0.3 * bm25_score
LET boosted = base * 1.5
SELECT * FROM docs
WHERE vector NEAR $v AND content MATCH 'AI'
ORDER BY boosted DESC
LIMIT 5
```

### Training

```sql
-- Train product quantizer
TRAIN QUANTIZER ON embeddings WITH (m = 8, k = 256)

-- Train with higher compression
TRAIN QUANTIZER ON large_collection WITH (m = 16, k = 256)
```

### Multi-Row INSERT and UPSERT

```sql
-- Multi-row insert (batch)
INSERT INTO products (id, title, price) VALUES
  (1, 'Laptop', 999.99),
  (2, 'Mouse', 29.99),
  (3, 'Keyboard', 79.99)

-- Upsert: insert or update if exists
UPSERT INTO products (id, title, price) VALUES (1, 'Laptop Pro', 1299.99)

-- Multi-row upsert
UPSERT INTO products (id, title, price) VALUES
  (1, 'Laptop Pro', 1299.99),
  (4, 'Monitor', 499.99)
```

### Graph Edge and Node Operations

```sql
-- Query all edges in a graph collection
SELECT EDGES FROM knowledge LIMIT 100

-- Query outgoing edges from a specific node
SELECT EDGES FROM knowledge WHERE source = 1

-- Query edges by relationship type
SELECT EDGES FROM knowledge WHERE label = 'AUTHORED_BY'

-- Insert a node with payload into a graph collection
INSERT NODE INTO knowledge (id = 42, payload = '{"name": "Alice", "_labels": ["Person"]}')
```

### Introspection

```sql
-- List all collections with their types
SHOW COLLECTIONS

-- Get collection metadata (dimension, metric, point count, type)
DESCRIBE COLLECTION documents
DESCRIBE documents

-- View query execution plan without running
EXPLAIN SELECT * FROM docs WHERE vector NEAR $v AND category = 'tech' LIMIT 10
```

### Administration

```sql
-- Create a secondary index for fast metadata lookups
CREATE INDEX ON documents (category)

-- Drop an index
DROP INDEX ON documents (category)

-- Compute CBO statistics for query optimizer
ANALYZE documents

-- Delete all data from a collection (preserves the collection)
TRUNCATE documents

-- Modify collection settings
ALTER COLLECTION documents SET (auto_reindex = true)

-- Flush data to disk (WAL only)
FLUSH

-- Full flush (includes index serialization)
FLUSH FULL

-- Flush a specific collection
FLUSH documents
```

---

## EBNF Grammar (v3.6)

```ebnf
(* ═══════════════════════════════════════════════════════ *)
(* Entry point                                            *)
(* ═══════════════════════════════════════════════════════ *)

query             = let_clause* (show_collections_stmt | describe_stmt
                    | explain_stmt | flush_stmt
                    | analyze_stmt | truncate_stmt | alter_collection_stmt
                    | select_edges_stmt
                    | match_query | compound_query | train_stmt
                    | create_index_stmt | create_collection_stmt
                    | drop_index_stmt | drop_collection_stmt
                    | insert_node_stmt | insert_edge_stmt
                    | delete_edge_stmt | delete_stmt
                    | upsert_stmt | insert_stmt | update_stmt) [";"] ;

(* ═══════════════════════════════════════════════════════ *)
(* Introspection statements (v3.4)                        *)
(* ═══════════════════════════════════════════════════════ *)

show_collections_stmt = "SHOW" "COLLECTIONS" ;
describe_stmt         = "DESCRIBE" ["COLLECTION"] identifier ;
explain_stmt          = "EXPLAIN" compound_query ;

(* ═══════════════════════════════════════════════════════ *)
(* LET bindings (v3.2)                                    *)
(* ═══════════════════════════════════════════════════════ *)

let_clause        = "LET" identifier "=" order_by_arithmetic ;

(* ═══════════════════════════════════════════════════════ *)
(* MATCH graph queries (v2.1)                             *)
(* ═══════════════════════════════════════════════════════ *)

match_query       = "MATCH" graph_pattern
                    [where_clause] return_clause
                    [order_by_clause] [limit_clause] ;

graph_pattern     = node_pattern (relationship_pattern node_pattern)* ;
node_pattern      = "(" [node_spec] ")" ;
node_spec         = [node_alias] [node_labels] [node_properties] ;
node_alias        = identifier ;
node_labels       = ":" label_name (":" label_name)* ;
node_properties   = "{" property ("," property)* "}" ;
property          = identifier ":" property_value ;
property_value    = string | float | integer | boolean | null | parameter ;

relationship_pattern = rel_incoming | rel_outgoing | rel_undirected ;
rel_incoming      = "<-" [rel_spec] "-" ;
rel_outgoing      = "-" [rel_spec] "->" ;
rel_undirected    = "-" [rel_spec] "-" ;
rel_spec          = "[" [rel_details] "]" ;
rel_details       = [rel_alias] [rel_types] [rel_range] [node_properties] ;
rel_alias         = identifier ;
rel_types         = ":" rel_type_name ("|" rel_type_name)* ;
rel_range         = "*" [range_spec] ;
range_spec        = range_bound ".." [range_bound]
                  | ".." range_bound
                  | integer ;

return_clause     = "RETURN" return_item ("," return_item)* ;
return_item       = return_expr ["AS" identifier] ;
return_expr       = "similarity" "(" ")" | property_access | identifier | "*" ;
property_access   = identifier "." identifier ;

(* ═══════════════════════════════════════════════════════ *)
(* SELECT + compound queries                              *)
(* ═══════════════════════════════════════════════════════ *)

compound_query    = select_stmt (set_operator select_stmt)* ;
set_operator      = "UNION" "ALL" | "UNION" | "INTERSECT" | "EXCEPT" ;

select_stmt       = "SELECT" [distinct_modifier] select_list
                    "FROM" from_clause join_clause*
                    [where_clause]
                    [group_by_clause] [having_clause]
                    [order_by_clause]
                    [limit_clause] [offset_clause]
                    [with_clause]
                    [using_fusion_clause] ;

distinct_modifier = "DISTINCT" ;

(* SELECT list *)
select_list       = "*" | select_item ("," select_item)* ;
select_item       = "similarity" "(" ")" ["AS" identifier]
                  | aggregate_function ["AS" identifier]
                  | qualified_wildcard
                  | column ["AS" identifier] ;
qualified_wildcard = identifier "." "*" ;
column            = identifier ("." identifier)* ;

(* FROM clause *)
from_clause       = identifier ["AS" identifier] ;

(* JOIN clause (v2.0) *)
join_clause       = [join_type] "JOIN" identifier ["AS" identifier]
                    (on_clause | using_clause) ;
join_type         = "LEFT" ["OUTER"]
                  | "RIGHT" ["OUTER"]
                  | "FULL" ["OUTER"]
                  | "INNER" ;
on_clause         = "ON" column_ref "=" column_ref ;
using_clause      = "USING" "(" identifier ("," identifier)* ")" ;
column_ref        = identifier "." identifier ;

(* ═══════════════════════════════════════════════════════ *)
(* WHERE clause                                           *)
(* ═══════════════════════════════════════════════════════ *)

where_clause      = "WHERE" or_expr ;
or_expr           = and_expr ("OR" and_expr)* ;
and_expr          = primary_expr ("AND" primary_expr)* ;
primary_expr      = "(" or_expr ")"
                  | not_expr
                  | graph_match_expr
                  | similarity_expr
                  | vector_fused_search
                  | sparse_vector_search
                  | vector_search
                  | match_expr
                  | in_expr
                  | between_expr
                  | like_expr
                  | is_null_expr
                  | compare_expr ;

not_expr          = "NOT" primary_expr ;
graph_match_expr  = "MATCH" graph_pattern ;

(* Vector operations *)
vector_search     = "vector" "NEAR" vector_expr ;
vector_fused_search = "vector" "NEAR_FUSED" vector_array [fusion_clause_inline] ;
sparse_vector_search = "vector" "SPARSE_NEAR" sparse_vector_expr ["USING" string] ;
similarity_expr   = "similarity" "(" similarity_field "," vector_expr ")"
                    compare_op numeric_threshold ;

vector_expr       = vector_literal | parameter ;
vector_literal    = "[" (float | integer) ("," (float | integer))* "]" ;
vector_array      = "[" vector_expr ("," vector_expr)* "]" ;
sparse_vector_expr = sparse_literal | parameter ;
sparse_literal    = "{" sparse_entry ("," sparse_entry)* "}" ;
sparse_entry      = integer ":" float ;

fusion_clause_inline = "USING" "FUSION" string ["(" fusion_param_list ")"] ;
fusion_param_list = fusion_param ("," fusion_param)* ;
fusion_param      = identifier "=" (float | integer) ;

(* Comparisons *)
compare_expr      = where_column compare_op value ;
compare_op        = ">=" | "<=" | "<>" | "!=" | "=" | ">" | "<" ;
where_column      = identifier ("." identifier)* ;

(* Special conditions *)
in_expr           = where_column ["NOT"] "IN" "(" value ("," value)* ")" ;
between_expr      = where_column "BETWEEN" value "AND" value ;
like_expr         = where_column ("ILIKE" | "LIKE") string ;
is_null_expr      = where_column "IS" ["NOT"] "NULL" ;
match_expr        = where_column "MATCH" string ;

(* ═══════════════════════════════════════════════════════ *)
(* GROUP BY, HAVING (v2.0)                                *)
(* ═══════════════════════════════════════════════════════ *)

group_by_clause   = "GROUP" "BY" column ("," column)* ;

having_clause     = "HAVING" having_condition (("AND" | "OR") having_condition)* ;
having_condition  = aggregate_function compare_op value ;

aggregate_function = ("COUNT" | "SUM" | "AVG" | "MIN" | "MAX")
                     "(" ("*" | column) ")" ;

(* ═══════════════════════════════════════════════════════ *)
(* ORDER BY (v2.0 + arithmetic v3.0)                      *)
(* ═══════════════════════════════════════════════════════ *)

order_by_clause   = "ORDER" "BY" order_by_item ("," order_by_item)* ;
order_by_item     = order_by_expr ["ASC" | "DESC"] ;
order_by_expr     = aggregate_function
                  | property_access
                  | order_by_arithmetic ;

order_by_arithmetic = oa_additive ;
oa_additive       = oa_multiplicative (("+" | "-") oa_multiplicative)* ;
oa_multiplicative = oa_atom (("*" | "/") oa_atom)* ;
oa_atom           = float | integer
                  | order_by_similarity
                  | order_by_similarity_bare
                  | "(" oa_additive ")"
                  | identifier ;

order_by_similarity      = "similarity" "(" similarity_field "," vector_expr ")" ;
order_by_similarity_bare = "similarity" "(" ")" ;

similarity_field  = (letter | "_") (letter | digit | "_" | ".")* ;

(* ═══════════════════════════════════════════════════════ *)
(* LIMIT, OFFSET, WITH, USING FUSION                      *)
(* ═══════════════════════════════════════════════════════ *)

limit_clause      = "LIMIT" integer ;
offset_clause     = "OFFSET" integer ;

with_clause       = "WITH" "(" with_option ("," with_option)* ")" ;
with_option       = identifier "=" with_value ;
with_value        = string | float | integer | boolean | identifier ;

using_fusion_clause = "USING" "FUSION" ["(" fusion_option_list ")"] ;
fusion_option_list = fusion_option ("," fusion_option)* ;
fusion_option     = identifier "=" (string | float | integer) ;

(* ═══════════════════════════════════════════════════════ *)
(* INSERT statement (v3.2)                                *)
(* ═══════════════════════════════════════════════════════ *)

insert_stmt       = "INSERT" "INTO" identifier
                    "(" identifier ("," identifier)* ")"
                    "VALUES" values_row ("," values_row)* ;
values_row        = "(" value ("," value)* ")" ;

(* ═══════════════════════════════════════════════════════ *)
(* UPDATE statement (v3.2)                                *)
(* ═══════════════════════════════════════════════════════ *)

update_stmt       = "UPDATE" identifier "SET"
                    assignment ("," assignment)*
                    [where_clause] ;
assignment        = identifier "=" value ;

(* ═══════════════════════════════════════════════════════ *)
(* DELETE statement (v3.3)                                *)
(* ═══════════════════════════════════════════════════════ *)

delete_stmt       = "DELETE" "FROM" identifier where_clause ;

(* ═══════════════════════════════════════════════════════ *)
(* Graph mutation statements (v3.3)                       *)
(* ═══════════════════════════════════════════════════════ *)

insert_edge_stmt  = "INSERT" "EDGE" "INTO" identifier
                    "(" edge_field ("," edge_field)* ")"
                    ["WITH" "PROPERTIES" "(" create_option_list ")"] ;
edge_field        = identifier "=" value ;

delete_edge_stmt  = "DELETE" "EDGE" value "FROM" identifier ;

(* ═══════════════════════════════════════════════════════ *)
(* DDL: CREATE / DROP COLLECTION, INDEX (v3.3 / v3.5)     *)
(* ═══════════════════════════════════════════════════════ *)

create_index_stmt = "CREATE" "INDEX" "ON" identifier "(" identifier ")" ;
drop_index_stmt   = "DROP" "INDEX" "ON" identifier "(" identifier ")" ;

create_collection_stmt = "CREATE" ["GRAPH" | "METADATA"] "COLLECTION" identifier
                         [create_body] ;
create_body       = "(" create_option_list ")" [create_suffix] ;
create_option_list = create_option ("," create_option)* ;
create_option     = identifier "=" create_option_value ;
create_option_value = string | float | integer | boolean | identifier ;

create_suffix     = "SCHEMALESS"
                  | "WITH" "SCHEMA" "(" schema_def_list ")"
                  | with_clause ;
schema_def_list   = schema_def ("," schema_def)* ;
schema_def        = node_type_def | edge_type_def ;
node_type_def     = "NODE" identifier "(" property_def_list ")" ;
edge_type_def     = "EDGE" identifier "FROM" identifier "TO" identifier ;
property_def_list = property_def ("," property_def)* ;
property_def      = identifier ":" type_name ;
type_name         = "STRING" | "INTEGER" | "FLOAT" | "BOOLEAN" | "VECTOR" ;

drop_collection_stmt = "DROP" "COLLECTION" ["IF" "EXISTS"] identifier ;

(* ═══════════════════════════════════════════════════════ *)
(* TRAIN QUANTIZER (v2.2)                                 *)
(* ═══════════════════════════════════════════════════════ *)

train_stmt        = "TRAIN" "QUANTIZER" "ON" identifier with_clause ;

(* ═══════════════════════════════════════════════════════ *)
(* Admin statements (v3.5-3.6)                            *)
(* ═══════════════════════════════════════════════════════ *)

analyze_stmt      = "ANALYZE" ["COLLECTION"] identifier ;
truncate_stmt     = "TRUNCATE" ["COLLECTION"] identifier ;
alter_collection_stmt = "ALTER" "COLLECTION" identifier "SET"
                        "(" create_option_list ")" ;
flush_stmt        = "FLUSH" ["FULL"] [identifier] ;

(* ═══════════════════════════════════════════════════════ *)
(* Index management (v3.5)                                *)
(* ═══════════════════════════════════════════════════════ *)

create_index_stmt = "CREATE" "INDEX" "ON" identifier "(" identifier ")" ;
drop_index_stmt   = "DROP" "INDEX" "ON" identifier "(" identifier ")" ;

(* ═══════════════════════════════════════════════════════ *)
(* UPSERT (v3.5)                                          *)
(* ═══════════════════════════════════════════════════════ *)

upsert_stmt       = "UPSERT" "INTO" identifier
                    "(" identifier ("," identifier)* ")"
                    "VALUES" values_row ("," values_row)* ;

(* ═══════════════════════════════════════════════════════ *)
(* Graph edge/node queries (v3.5)                         *)
(* ═══════════════════════════════════════════════════════ *)

select_edges_stmt = "SELECT" "EDGES" "FROM" identifier
                    [where_clause] [limit_clause] ;
insert_node_stmt  = "INSERT" "NODE" "INTO" identifier
                    "(" edge_field ("," edge_field)* ")" ;

(* ═══════════════════════════════════════════════════════ *)
(* Subqueries (v3.2)                                      *)
(* ═══════════════════════════════════════════════════════ *)

subquery_expr     = "(" "SELECT" select_list "FROM" identifier
                    [where_clause] [group_by_clause] [having_clause]
                    [limit_clause] ")" ;

(* ═══════════════════════════════════════════════════════ *)
(* Values and literals                                    *)
(* ═══════════════════════════════════════════════════════ *)

value             = subquery_expr | temporal_expr | float | integer
                  | string | boolean | null | parameter ;

temporal_expr     = temporal_arithmetic | now_function | interval_expr ;
temporal_arithmetic = (now_function | interval_expr) ("+" | "-")
                      (now_function | interval_expr) ;
now_function      = "NOW" "(" ")" ;
interval_expr     = "INTERVAL" string ;

string            = "'" (char | "''")* "'" ;
integer           = ["-"] digit+ ;
float             = ["-"] digit+ "." digit+ ;
boolean           = "TRUE" | "FALSE" ;
null              = "NULL" ;
parameter         = "$" identifier ;

identifier        = quoted_identifier | regular_identifier ;
regular_identifier = (letter | "_") (letter | digit | "_")* ;
quoted_identifier = "`" backtick_inner "`" | '"' doublequote_inner '"' ;
backtick_inner    = (!"`" ANY)+ ;
doublequote_inner = (doublequote_escape | !('"') ANY)* ;
doublequote_escape = '""' ;

(* ═══════════════════════════════════════════════════════ *)
(* Whitespace and comments                                *)
(* ═══════════════════════════════════════════════════════ *)

WHITESPACE        = " " | "\t" | "\r" | "\n" ;
COMMENT           = "--" (!"\n" ANY)* ;
```

---

## License

VelesDB Core License 1.0
