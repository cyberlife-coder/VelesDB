# VelesQL Language Specification

> SQL-like query language for vector search in VelesDB.

**Version**: 2.0.0 | **Last Updated**: 2026-01-26

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
| ORDER BY | ✅ Stable | 2.0 |
| GROUP BY, HAVING | ✅ Stable | 2.0 |
| JOIN (LEFT, RIGHT, FULL) | ✅ Stable | 2.0 |
| Set Operations (UNION, INTERSECT, EXCEPT) | ✅ Stable | 2.0 |
| USING FUSION | ✅ Stable | 2.0 |
| MATCH graph traversal | 🔜 Planned | - |
| Table aliases | 🔜 Planned | - |

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
-- LIKE with wildcards
SELECT * FROM docs WHERE title LIKE '%database%'
SELECT * FROM docs WHERE name LIKE 'vec%'

-- MATCH for full-text (if supported)
SELECT * FROM docs WHERE content MATCH 'vector database'
```

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
SELECT, FROM, WHERE, AND, OR, NOT, IN, BETWEEN, LIKE, MATCH,
IS, NULL, TRUE, FALSE, LIMIT, OFFSET, WITH, NEAR, ASC, DESC,
ORDER, BY, AS, SIMILARITY
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
| `NEAR`, `SIMILARITY` | Vector operations |

## Grammar (EBNF) - v2.0

```ebnf
(* Top-level query with optional set operations *)
query           = select_stmt { set_operator select_stmt } ;
set_operator    = "UNION" ["ALL"] | "INTERSECT" | "EXCEPT" ;

(* SELECT statement with all clauses *)
select_stmt     = "SELECT" select_list 
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
table_ref       = identifier [alias] ;
alias           = ["AS"] identifier ;

(* JOIN clause *)
join_clause     = [join_type] "JOIN" table_ref ("ON" condition | "USING" "(" identifier ")") ;
join_type       = "INNER" | "LEFT" ["OUTER"] | "RIGHT" ["OUTER"] | "FULL" ["OUTER"] ;

(* WHERE clause *)
where_clause    = "WHERE" or_expr ;
or_expr         = and_expr { "OR" and_expr } ;
and_expr        = condition { "AND" condition } ;
condition       = comparison | vector_search | similarity_cond | in_cond 
                | between_cond | like_cond | is_null_cond | "(" or_expr ")" ;

(* Vector operations *)
vector_search   = identifier "NEAR" vector_expr ;
similarity_cond = "similarity" "(" identifier "," vector_expr ")" compare_op number ;
vector_expr     = "$" identifier | "[" number { "," number } "]" ;

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
order_item      = (column | similarity_expr) ["ASC" | "DESC"] ;
similarity_expr = "similarity" "(" identifier "," vector_expr ")" ;

(* Pagination *)
limit_clause    = "LIMIT" integer ;
offset_clause   = "OFFSET" integer ;

(* WITH options *)
with_clause     = "WITH" "(" with_option { "," with_option } ")" ;
with_option     = identifier "=" value ;

(* USING FUSION for hybrid search *)
using_fusion_clause = "USING" "FUSION" "(" fusion_strategy ["," fusion_params] ")" ;
fusion_strategy = "rrf" | "weighted" | "maximum" ;
fusion_params   = fusion_param { "," fusion_param } ;
fusion_param    = identifier "=" value ;

(* Values *)
value           = string | number | boolean | "NULL" | vector_literal ;
vector_literal  = "[" number { "," number } "]" ;
string          = "'" { char } "'" | '"' { char } '"' ;
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
