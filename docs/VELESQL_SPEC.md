# VelesQL Language Specification

> SQL-like query language for vector search in VelesDB.

**Version**: 2.1.0 | **Last Updated**: 2026-02-09

## Overview

VelesQL is a SQL-inspired query language designed specifically for vector similarity search. It combines familiar SQL syntax with vector-specific operations like `NEAR` for semantic search.

### Feature Support Status

| Feature | Status | Version |
|---------|--------|---------|
| SELECT, FROM, WHERE | ✅ Stable | 1.0 |
| NEAR vector search | ✅ Stable | 1.0 |
| similarity() function | ✅ Stable | 1.3 |
| LIMIT, OFFSET | ✅ Stable | 1.0 |
| WITH clause | ✅ Stable | 1.0 |
| LIKE / ILIKE (case-insensitive) | ✅ Stable | 1.3 |
| IN, BETWEEN, IS NULL | ✅ Stable | 1.3 |
| Quoted identifiers (\`col\`, "col") | ✅ Stable | 1.3 |
| ORDER BY (columns, similarity, aggregates) | ✅ Stable | 2.0 |
| GROUP BY, HAVING | ✅ Stable | 2.0 |
| DISTINCT | ✅ Stable | 2.0 |
| USING FUSION (rrf, weighted, maximum) | ✅ Stable | 2.0 |
| Table aliases (FROM t AS alias) | ✅ Stable | 2.0 |
| NOW() / INTERVAL temporal | ✅ Stable | 2.1 |
| Subqueries (scalar) | ✅ Stable | 2.1 |
| NEAR_FUSED multi-vector fusion | ✅ Stable | 2.1 |
| MATCH graph traversal | ✅ Stable | 2.1 |
| Multi-hop MATCH patterns | ✅ Stable | 2.1 |
| RETURN aggregation in MATCH | ✅ Stable | 2.1 |
| ORDER BY property in MATCH | ✅ Stable | 2.1 |
| JOIN (INNER, LEFT) | ✅ Stable | 2.2 |
| JOIN (RIGHT, FULL) | 🧪 Parser only (falls back to INNER) | 2.0 |
| Set Operations (UNION, INTERSECT, EXCEPT) | ✅ Stable | 2.2 |
| EXPLAIN query plan | ✅ Stable | 2.2 |

## Basic Syntax

```sql
SELECT <columns>
FROM <collection>
[WHERE <conditions>]
[LIMIT <n>]
[OFFSET <n>]
[WITH (<options>)]
```

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

Access nested JSON fields using dot notation:

```sql
SELECT payload.metadata.author FROM articles
```

## FROM Clause

Specify the collection name:

```sql
SELECT * FROM my_collection
```

## WHERE Clause

### Vector Similarity Search

Use `NEAR` for approximate nearest neighbor search:

```sql
-- With parameter placeholder
SELECT * FROM docs WHERE vector NEAR $v

-- With literal vector
SELECT * FROM docs WHERE vector NEAR [0.1, 0.2, 0.3, 0.4]
```

### Comparison Operators

| Operator | Description | Example |
|----------|-------------|---------|
| `=` | Equal | `category = 'tech'` |
| `!=` or `<>` | Not equal | `status != 'deleted'` |
| `>` | Greater than | `price > 100` |
| `>=` | Greater or equal | `score >= 0.8` |
| `<` | Less than | `count < 50` |
| `<=` | Less or equal | `rating <= 5` |

### String Matching

```sql
-- LIKE with wildcards (case-sensitive)
SELECT * FROM docs WHERE title LIKE '%database%'
SELECT * FROM docs WHERE name LIKE 'vec%'

-- ILIKE for case-insensitive matching (v1.3+)
SELECT * FROM docs WHERE title ILIKE '%Database%'
SELECT * FROM docs WHERE author ILIKE 'john%'

-- MATCH for full-text search (BM25 scoring)
SELECT * FROM docs WHERE content MATCH 'vector database'
```

#### LIKE vs ILIKE

| Operator | Case | Example |
|----------|------|---------|
| `LIKE` | Sensitive | `title LIKE '%DB%'` matches "VelesDB" but not "velesdb" |
| `ILIKE` | Insensitive | `title ILIKE '%db%'` matches both "VelesDB" and "velesdb" |

Wildcards: `%` matches zero or more characters, `_` matches exactly one character.

### NULL Checks

```sql
SELECT * FROM docs WHERE category IS NULL
SELECT * FROM docs WHERE category IS NOT NULL
```

### IN Operator

```sql
SELECT * FROM docs WHERE category IN ('tech', 'science', 'ai')
SELECT * FROM docs WHERE id IN (1, 2, 3, 4, 5)
```

### BETWEEN Operator

```sql
SELECT * FROM docs WHERE price BETWEEN 10 AND 100
SELECT * FROM docs WHERE date BETWEEN '2024-01-01' AND '2024-12-31'
```

### Temporal Functions (v2.1+)

VelesQL supports temporal expressions for date/time filtering using `NOW()` and `INTERVAL`.

#### NOW() Function

Returns the current Unix timestamp (seconds since epoch):

```sql
-- Events after now
SELECT * FROM events WHERE timestamp > NOW()

-- Recently created items
SELECT * FROM items WHERE created_at > NOW()
```

#### INTERVAL Expression

Defines a time duration:

```sql
INTERVAL '<magnitude> <unit>'
```

**Supported Units:**

| Unit | Aliases | Seconds |
|------|---------|---------|
| seconds | s, sec, second | 1 |
| minutes | m, min, minute | 60 |
| hours | h, hour | 3,600 |
| days | d, day | 86,400 |
| weeks | w, week | 604,800 |
| months | month | ~2,592,000 |

#### Temporal Arithmetic

Combine `NOW()` with `INTERVAL` for relative time queries:

```sql
-- Last 7 days
SELECT * FROM logs WHERE created_at > NOW() - INTERVAL '7 days'

-- Last hour
SELECT * FROM events WHERE timestamp > NOW() - INTERVAL '1 hour'

-- Next week (future events)
SELECT * FROM tasks WHERE due_date < NOW() + INTERVAL '7 days'

-- Shorthand units
SELECT * FROM metrics WHERE ts > NOW() - INTERVAL '30 min'
```

#### Common Patterns

| Use Case | Query |
|----------|-------|
| Last 24 hours | `WHERE ts > NOW() - INTERVAL '24 hours'` |
| This week | `WHERE ts > NOW() - INTERVAL '7 days'` |
| Last month | `WHERE ts > NOW() - INTERVAL '1 month'` |
| Recent activity | `WHERE last_seen > NOW() - INTERVAL '5 minutes'` |

### Logical Operators

```sql
-- AND
SELECT * FROM docs WHERE category = 'tech' AND price > 50

-- OR
SELECT * FROM docs WHERE category = 'tech' OR category = 'science'

-- Combined
SELECT * FROM docs WHERE (category = 'tech' OR category = 'ai') AND price > 100
```

### Vector Search with Filters

Combine vector search with metadata filters:

```sql
SELECT * FROM docs 
WHERE vector NEAR $v AND category = 'tech' AND price > 50
LIMIT 10
```

### Similarity Function (v1.3+)

The `similarity()` function enables **threshold-based vector filtering** - filter results by similarity score rather than just finding nearest neighbors.

#### Syntax

```sql
similarity(field, vector_expr) <operator> threshold
```

#### Parameters

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
WHERE similarity(vector, $q) > 0.7 
  AND category = 'technology'
  AND published = true
LIMIT 20
```

#### Use Cases

| Use Case | Query Pattern |
|----------|---------------|
| **Semantic Search** | `similarity(v, $q) > 0.75` |
| **Deduplication** | `similarity(v, $ref) < 0.9` |
| **Quality Filter** | `similarity(v, $ideal) >= 0.85` |
| **RAG Retrieval** | `similarity(embedding, $query) > 0.7 AND source = 'docs'` |

#### Difference: NEAR vs similarity()

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

## ORDER BY Clause (v2.0+)

Sort results by one or more columns or expressions.

### Basic Syntax

```sql
SELECT * FROM docs ORDER BY created_at DESC
SELECT * FROM docs ORDER BY category ASC, price DESC
```

### Order by Similarity

Sort by vector similarity score:

```sql
-- Order by similarity (highest first)
SELECT * FROM docs 
WHERE similarity(embedding, $query) > 0.5
ORDER BY similarity(embedding, $query) DESC
LIMIT 10

-- Multi-column with similarity
SELECT * FROM docs 
WHERE vector NEAR $v
ORDER BY similarity(vector, $v) DESC, created_at DESC
LIMIT 20
```

### Direction

| Direction | Description |
|-----------|-------------|
| `ASC` | Ascending (default) |
| `DESC` | Descending |

## GROUP BY and HAVING (v2.0+)

Aggregate results by groups.

### Basic Syntax

```sql
SELECT category, COUNT(*) FROM docs GROUP BY category
SELECT category, AVG(price) FROM products GROUP BY category HAVING COUNT(*) > 5
```

### Aggregate Functions

| Function | Description | Example |
|----------|-------------|---------|
| `COUNT(*)` | Count rows | `COUNT(*)` |
| `COUNT(field)` | Count non-null values | `COUNT(price)` |
| `SUM(field)` | Sum of values | `SUM(quantity)` |
| `AVG(field)` | Average value | `AVG(rating)` |
| `MIN(field)` | Minimum value | `MIN(price)` |
| `MAX(field)` | Maximum value | `MAX(score)` |

### Examples

```sql
-- Count by category
SELECT category, COUNT(*) FROM products GROUP BY category

-- Average with filter
SELECT category, AVG(price) 
FROM products 
WHERE similarity(embedding, $query) > 0.6
GROUP BY category
ORDER BY AVG(price) DESC

-- HAVING clause
SELECT category, COUNT(*) 
FROM docs 
GROUP BY category 
HAVING COUNT(*) > 10
ORDER BY COUNT(*) DESC

-- Multiple aggregates
SELECT category, COUNT(*), AVG(price), MAX(rating)
FROM products
GROUP BY category
HAVING AVG(price) > 50
```

## JOIN Clause (v2.0+)

> ✅ **Executed** — INNER JOIN and LEFT JOIN are fully supported via `Database::execute_query()`. RIGHT and FULL JOIN are parsed but fall back to INNER JOIN with a warning. JOINs resolve the target table as a Collection, build a ColumnStore for O(1) PK lookups, and merge column data into the result payload.

Combine data from multiple collections.

### Syntax

```sql
SELECT * FROM table1
[INNER|LEFT|RIGHT|FULL] JOIN table2 ON condition
```

### Join Types

| Type | Description |
|------|-------------|
| `JOIN` / `INNER JOIN` | Only matching rows |
| `LEFT JOIN` | All from left + matching right |
| `RIGHT JOIN` | All from right + matching left |
| `FULL JOIN` | All from both tables |

### Examples

```sql
-- Inner join
SELECT orders.id, customers.name
FROM orders
JOIN customers ON orders.customer_id = customers.id

-- Left join with filter
SELECT p.title, c.name AS category_name
FROM products p
LEFT JOIN categories c ON p.category_id = c.id
WHERE similarity(p.embedding, $query) > 0.7
LIMIT 20

-- Using clause (alternative to ON)
SELECT * FROM orders JOIN customers USING (customer_id)

-- Multiple joins
SELECT o.id, c.name, p.title
FROM orders o
JOIN customers c ON o.customer_id = c.id
JOIN products p ON o.product_id = p.id
```

## Set Operations (v2.0+)

> ✅ **Executed** — Set operations are fully supported via `Database::execute_query()`. UNION deduplicates by point ID, UNION ALL keeps all rows, INTERSECT keeps only common IDs, and EXCEPT removes the second set's IDs from the first.

Combine results from multiple queries.

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

Only rows in both queries:

```sql
SELECT id FROM liked_items WHERE user_id = 1
INTERSECT
SELECT id FROM liked_items WHERE user_id = 2
```

### EXCEPT

Rows in first query but not second:

```sql
SELECT id FROM all_items
EXCEPT
SELECT id FROM purchased_items
```

## USING FUSION - Hybrid Search (v2.0+)

Combine multiple search strategies with result fusion.

### Syntax

```sql
SELECT * FROM docs
WHERE vector NEAR $v AND text MATCH 'query'
USING FUSION(strategy, k, weights)
```

### Fusion Strategies

| Strategy | Description | Use Case |
|----------|-------------|----------|
| `rrf` | Reciprocal Rank Fusion | Balanced ranking (default) |
| `weighted` | Weighted combination | Custom importance |
| `maximum` | Take highest score | Best match wins |

### Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `strategy` | string | `rrf` | Fusion algorithm |
| `k` | integer | 60 | RRF constant |
| `weights` | array | `[0.5, 0.5]` | Strategy weights |

### Examples

```sql
-- Default RRF fusion
SELECT * FROM docs
WHERE vector NEAR $v
USING FUSION(rrf)
LIMIT 10

-- Weighted fusion (70% vector, 30% text)
SELECT * FROM docs
WHERE vector NEAR $semantic AND content MATCH $keywords
USING FUSION(weighted, weights = [0.7, 0.3])
LIMIT 20

-- Maximum score fusion
SELECT * FROM docs
WHERE similarity(embedding, $q1) > 0.5
USING FUSION(maximum)
LIMIT 10
```

## Scalar Subqueries (v2.1+)

Use scalar subqueries in WHERE clauses to compare values against aggregated results from the same or another collection.

### Syntax

```sql
WHERE column <operator> (SELECT aggregate(col) FROM collection [WHERE ...])
```

### Examples

```sql
-- Find products priced above average
SELECT * FROM products
WHERE price > (SELECT AVG(price) FROM products)
LIMIT 20

-- Find items more expensive than category max
SELECT * FROM products
WHERE price > (SELECT MAX(price) FROM products WHERE category = 'budget')

-- Combine with vector search
SELECT * FROM products
WHERE similarity(embedding, $query) > 0.7
  AND price < (SELECT AVG(price) FROM products WHERE category = $cat)
LIMIT 10
```

### Supported Aggregations in Subqueries

| Function | Returns |
|----------|---------|
| `MAX(col)` | Maximum value |
| `MIN(col)` | Minimum value |
| `AVG(col)` | Average value |
| `SUM(col)` | Sum of values |
| `COUNT(*)` | Row count |

### Limitations

- Only **scalar** subqueries (returning a single value) are supported
- Subqueries execute against the **same collection** as the outer query
- Correlated subqueries with `$parameter` references are supported
- Nested subqueries (subquery inside subquery) are not supported

## NEAR_FUSED Multi-Vector Search (v2.1+)

Search with multiple query vectors simultaneously and fuse results using configurable strategies.

### Syntax

```sql
SELECT * FROM collection
WHERE vector NEAR_FUSED [$v1, $v2, ...] [USING FUSION 'strategy' [(params)]]
LIMIT n
```

### Fusion Strategies

| Strategy | Description | Use Case |
|----------|-------------|----------|
| `rrf` | Reciprocal Rank Fusion | Balanced ranking (default) |
| `average` | Average scores across vectors | Equal weight queries |
| `maximum` | Take highest score per result | Best match wins |
| `weighted` | Weighted combination | Custom importance per vector |

### Examples

```sql
-- Multi-modal search: text + image embeddings
SELECT * FROM products
WHERE vector NEAR_FUSED [$text_embedding, $image_embedding]
  USING FUSION 'rrf'
LIMIT 10

-- Weighted: 70% text, 30% image
SELECT * FROM products
WHERE vector NEAR_FUSED [$text_vec, $image_vec]
  USING FUSION 'weighted' (weight_0 = 0.7, weight_1 = 0.3)
LIMIT 20

-- Average fusion with metadata filter
SELECT * FROM docs
WHERE vector NEAR_FUSED [$query1, $query2]
  USING FUSION 'average'
  AND category = 'research'
LIMIT 15
```

### Constraints

- Cannot combine `NEAR_FUSED` with `NEAR` in the same query
- Cannot combine `NEAR_FUSED` with `similarity()` in the same query
- All vectors must have the same dimensionality as the collection

## LIMIT and OFFSET

```sql
-- Limit results
SELECT * FROM docs LIMIT 10

-- Pagination
SELECT * FROM docs LIMIT 10 OFFSET 20
```

## WITH Clause (Search Options)

Control search behavior with the `WITH` clause:

```sql
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
WITH (mode = 'accurate', ef_search = 256, timeout_ms = 5000)
```

### Available Options

| Option | Type | Values | Description |
|--------|------|--------|-------------|
| `mode` | string | `fast`, `balanced`, `accurate`, `high_recall`, `perfect` | Search mode preset |
| `ef_search` | integer | 16-4096 | HNSW ef_search parameter |
| `timeout_ms` | integer | >=100 | Query timeout in milliseconds |
| `rerank` | boolean | `true`/`false` | Enable reranking for quantized vectors |

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
```

## Value Types

### Strings

```sql
'hello world'
"double quotes also work"
```

#### String Escaping

To include a single quote inside a string, **double it** (`''`):

```sql
-- Correct: Use '' to escape single quotes
SELECT * FROM docs WHERE name = 'O''Brien'      -- Matches "O'Brien"
SELECT * FROM docs WHERE text = 'It''s working' -- Matches "It's working"

-- Empty string
SELECT * FROM docs WHERE name = ''
```

⚠️ **Backslash escaping is NOT supported**:

```sql
-- ❌ WRONG: This will cause a parse error
SELECT * FROM docs WHERE name = 'O\'Brien'

-- ✅ CORRECT: Use double single-quote
SELECT * FROM docs WHERE name = 'O''Brien'
```

Unicode characters are fully supported:

```sql
SELECT * FROM docs WHERE title = '日本語テキスト'
SELECT * FROM docs WHERE emoji = '🚀 Launch'
```

### Numbers

```sql
42          -- integer
3.14        -- float
-100        -- negative
```

### Booleans

```sql
true
false
```

### Vectors

```sql
[0.1, 0.2, 0.3, 0.4]           -- literal vector
$query_vector                   -- parameter reference
```

### NULL

```sql
NULL
```

## Parameters

Use `$name` syntax for parameterized queries:

```sql
SELECT * FROM docs WHERE vector NEAR $query_vector AND category = $cat
```

Parameters are resolved at runtime from the query context.

## Reserved Keywords

The following keywords are reserved and cannot be used as identifiers without escaping:

```
SELECT, FROM, WHERE, AND, OR, NOT, IN, BETWEEN, LIKE, ILIKE, MATCH,
IS, NULL, TRUE, FALSE, LIMIT, OFFSET, WITH, NEAR, NEAR_FUSED, ASC, DESC,
ORDER, BY, AS, SIMILARITY, DISTINCT, GROUP, HAVING, JOIN, INNER, LEFT, RIGHT,
FULL, OUTER, ON, USING, FUSION, UNION, INTERSECT, EXCEPT, RETURN, NOW, INTERVAL
```

### Identifier Quoting (v1.3+)

To use reserved keywords as column or table names, quote them with **backticks** or **double quotes**:

```sql
-- Backtick escaping (MySQL-style)
SELECT `select`, `from`, `order` FROM docs

-- Double-quote escaping (SQL standard)
SELECT "select", "from", "order" FROM docs

-- Mixed styles in same query
SELECT `select`, "order" FROM my_table WHERE `limit` > 10
```

#### Escaping Quotes Inside Identifiers

To include a double-quote inside a double-quoted identifier, **double it**:

```sql
-- Column named: col"name
SELECT "col""name" FROM docs
```

#### Examples with All Clauses

```sql
-- Reserved keyword as table name
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

#### Complete List of Reserved Keywords

| Keyword | Category |
|---------|----------|
| `SELECT`, `FROM`, `WHERE` | Query structure |
| `AND`, `OR`, `NOT` | Logical operators |
| `IN`, `BETWEEN`, `LIKE`, `MATCH` | Comparison operators |
| `IS`, `NULL` | NULL handling |
| `TRUE`, `FALSE` | Boolean literals |
| `LIMIT`, `OFFSET` | Pagination |
| `ORDER`, `BY`, `ASC`, `DESC` | Sorting |
| `GROUP`, `HAVING` | Aggregation |
| `WITH`, `AS` | Options and aliases |
| `NEAR`, `NEAR_FUSED`, `SIMILARITY` | Vector operations |
| `ILIKE` | Case-insensitive matching |
| `DISTINCT` | Deduplication |
| `JOIN`, `INNER`, `LEFT`, `RIGHT`, `FULL`, `OUTER`, `ON` | Joins (INNER/LEFT executed, RIGHT/FULL parser only) |
| `UNION`, `INTERSECT`, `EXCEPT` | Set operations (fully executed) |
| `USING`, `FUSION` | Hybrid search |
| `RETURN` | Graph query results |
| `NOW`, `INTERVAL` | Temporal expressions |

## Grammar (EBNF) - v2.1

```ebnf
(* Top-level query: MATCH or SELECT with optional set operations *)
query           = match_stmt | select_stmt { set_operator select_stmt } ;
set_operator    = "UNION" ["ALL"] | "INTERSECT" | "EXCEPT" ;

(* ============ MATCH query (graph pattern matching) ============ *)
match_stmt      = "MATCH" graph_pattern
                  [where_clause]
                  return_clause
                  [order_by_clause]
                  [limit_clause] ;

graph_pattern   = node_pattern { relationship_pattern node_pattern } ;
node_pattern    = "(" [node_alias] [node_labels] [node_props] ")" ;
node_alias      = identifier ;
node_labels     = ":" label_name { ":" label_name } ;
node_props      = "{" property_list "}" ;
property_list   = property { "," property } ;
property        = identifier ":" value ;

relationship_pattern = rel_outgoing | rel_incoming | rel_undirected ;
rel_outgoing    = "-" [rel_spec] "->" ;
rel_incoming    = "<-" [rel_spec] "-" ;
rel_undirected  = "-" [rel_spec] "-" ;
rel_spec        = "[" [rel_alias] [rel_types] [rel_range] [node_props] "]" ;
rel_alias       = identifier ;
rel_types       = ":" rel_type { "|" rel_type } ;
rel_range       = "*" [range_spec] ;
range_spec      = integer ".." [integer] | ".." integer | integer ;

return_clause   = "RETURN" return_item { "," return_item } ;
return_item     = return_expr ["AS" identifier] ;
return_expr     = "similarity" "(" ")" | property_access | identifier | "*" ;
property_access = identifier "." identifier ;

(* ============ SELECT statement ============ *)
select_stmt     = "SELECT" ["DISTINCT"] select_list 
                  "FROM" table_ref
                  { join_clause }
                  [where_clause] 
                  [group_by_clause]
                  [having_clause]
                  [order_by_clause]
                  [limit_clause] 
                  [offset_clause] 
                  [with_clause]
                  [using_fusion_clause] ;

(* SELECT list *)
select_list     = "*" | select_item { "," select_item } ;
select_item     = (column | aggregate_func) ["AS" identifier] ;
column          = identifier { "." identifier } ;

(* Aggregate functions *)
aggregate_func  = ("COUNT" | "SUM" | "AVG" | "MIN" | "MAX") 
                  "(" ("*" | column) ")" ;

(* Table reference *)
table_ref       = identifier ["AS" identifier] ;

(* JOIN clause — INNER/LEFT executed, RIGHT/FULL parsed only *)
join_clause     = [join_type] "JOIN" table_ref ("ON" condition | "USING" "(" identifier ")") ;
join_type       = "INNER" | "LEFT" ["OUTER"] | "RIGHT" ["OUTER"] | "FULL" ["OUTER"] ;

(* WHERE clause *)
where_clause    = "WHERE" or_expr ;
or_expr         = and_expr { "OR" and_expr } ;
and_expr        = condition { "AND" condition } ;
condition       = comparison | vector_search | fused_search | similarity_cond 
                | in_cond | between_cond | like_cond | is_null_cond 
                | match_cond | "(" or_expr ")" ;

(* Vector operations *)
vector_search   = "vector" "NEAR" vector_expr ;
fused_search    = "vector" "NEAR_FUSED" "[" vector_expr { "," vector_expr } "]"
                  ["USING" "FUSION" string ["(" fusion_params ")"]] ;
similarity_cond = "similarity" "(" identifier "," vector_expr ")" compare_op number ;
vector_expr     = "$" identifier | "[" number { "," number } "]" ;

(* Full-text search *)
match_cond      = identifier "MATCH" string ;

(* Comparisons *)
comparison      = column compare_op value ;
compare_op      = "=" | "!=" | "<>" | ">" | ">=" | "<" | "<=" ;

(* Special conditions *)
in_cond         = column "IN" "(" value { "," value } ")" ;
between_cond    = column "BETWEEN" value "AND" value ;
like_cond       = column ("LIKE" | "ILIKE") string ;
is_null_cond    = column "IS" ["NOT"] "NULL" ;

(* GROUP BY and HAVING *)
group_by_clause = "GROUP" "BY" column { "," column } ;
having_clause   = "HAVING" having_expr ;
having_expr     = having_cond { ("AND" | "OR") having_cond } ;
having_cond     = aggregate_func compare_op value ;

(* ORDER BY *)
order_by_clause = "ORDER" "BY" order_item { "," order_item } ;
order_item      = (column | aggregate_func | similarity_expr) ["ASC" | "DESC"] ;
similarity_expr = "similarity" "(" identifier "," vector_expr ")" ;

(* Pagination *)
limit_clause    = "LIMIT" integer ;
offset_clause   = "OFFSET" integer ;

(* WITH options *)
with_clause     = "WITH" "(" with_option { "," with_option } ")" ;
with_option     = identifier "=" value ;

(* USING FUSION for hybrid search *)
using_fusion_clause = "USING" "FUSION" ["(" fusion_strategy ["," fusion_params] ")"] ;
fusion_strategy = identifier ;
fusion_params   = fusion_param { "," fusion_param } ;
fusion_param    = identifier "=" value ;

(* Values — includes temporal and subquery *)
value           = subquery_expr | temporal_expr | string | number | boolean 
                | "NULL" | "$" identifier | vector_literal ;
subquery_expr   = "(" "SELECT" select_list "FROM" identifier [where_clause] 
                  [group_by_clause] [having_clause] [limit_clause] ")" ;
temporal_expr   = temporal_arith | now_func | interval_expr ;
temporal_arith  = (now_func | interval_expr) ("+" | "-") (now_func | interval_expr) ;
now_func        = "NOW" "(" ")" ;
interval_expr   = "INTERVAL" string ;

vector_literal  = "[" number { "," number } "]" ;
string          = "'" { char } "'" ;
number          = ["-"] digit { digit } ["." digit { digit }] ;
boolean         = "true" | "false" ;
integer         = digit { digit } ;
identifier      = (letter | "_") { letter | digit | "_" } 
                | "`" { char } "`" 
                | '"' { char } '"' ;
```

## Examples

### Basic Queries

```sql
-- Get all documents
SELECT * FROM documents

-- Get specific fields with limit
SELECT id, payload.title FROM articles LIMIT 100

-- Pagination
SELECT * FROM products LIMIT 20 OFFSET 40
```

### Vector Search

```sql
-- Simple vector search
SELECT * FROM embeddings WHERE vector NEAR $query LIMIT 10

-- Vector search with metadata filter
SELECT id, score, payload.title FROM docs 
WHERE vector NEAR $v AND category = 'technology' 
LIMIT 5

-- High-accuracy search
SELECT * FROM legal_docs WHERE vector NEAR $q LIMIT 10 
WITH (mode = 'high_recall')
```

### Complex Filters

```sql
-- Multiple conditions
SELECT * FROM products 
WHERE category IN ('electronics', 'computers') 
  AND price BETWEEN 100 AND 1000
  AND rating >= 4.0
LIMIT 50

-- Text matching with vector search
SELECT * FROM articles 
WHERE vector NEAR $v 
  AND title LIKE '%AI%'
  AND published IS NOT NULL
LIMIT 10
```

## MATCH Clause (v2.1+)

Execute graph pattern matching queries with optional vector similarity filtering.

### Basic Syntax

```sql
MATCH <pattern>
[WHERE <conditions>]
RETURN <projection>
[ORDER BY <expression>]
[LIMIT <n>]
```

### Pattern Syntax

```sql
-- Node pattern
(alias:Label {property: value})

-- Relationship pattern
-[alias:TYPE]->    -- outgoing
<-[alias:TYPE]-    -- incoming
-[alias:TYPE]-     -- undirected

-- Variable length paths
-[*1..3]->         -- 1 to 3 hops
```

### Examples

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

-- Combined graph + vector + column (AIOps)
MATCH (incident:Ticket)-[:IMPACTS]->(service:Microservice)
WHERE similarity(incident.log_embedding, $error_vec) > 0.85
  AND incident.status = 'RESOLVED'
  AND service.criticality = 'HIGH'
RETURN incident.solution, service.name
ORDER BY similarity() DESC
LIMIT 3
```

### Execution Strategies

The query planner automatically chooses the optimal execution strategy:

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

## License

ELv2 (Elastic License 2.0)
