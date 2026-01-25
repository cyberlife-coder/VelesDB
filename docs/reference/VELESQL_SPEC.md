# VelesQL Language Specification

*Version 2.0.0 — January 2026*

VelesQL is a **SQL-like query language** designed specifically for vector search operations. If you know SQL, you already know VelesQL.

---

## Table of Contents

1. [Introduction](#introduction)
2. [Grammar (BNF)](#grammar-bnf)
3. [Data Types](#data-types)
4. [Operators](#operators)
5. [Clauses](#clauses)
6. [Vector Search](#vector-search)
7. [Full-Text Search](#full-text-search)
8. [Aggregations & GROUP BY](#aggregations--group-by)
9. [JOIN](#join)
10. [Set Operations](#set-operations)
11. [Parameters](#parameters)
12. [Examples](#examples)
13. [Limitations](#limitations)
14. [Troubleshooting](#troubleshooting)

---

## Introduction

### What is VelesQL?

VelesQL is a query language that combines familiar SQL syntax with vector similarity search capabilities. It allows you to:

- Search for similar vectors using `NEAR` operator
- Filter results with standard SQL conditions
- Combine vector search with full-text search (hybrid search)
- Use parameterized queries for safe, injection-free operations

### Key Differences from SQL

| Feature | SQL | VelesQL |
|---------|-----|---------|
| Vector search | ❌ Not supported | ✅ `vector NEAR $v` |
| Distance metrics | ❌ | ✅ `COSINE`, `EUCLIDEAN`, `DOT` |
| Full-text search | `LIKE '%..%'` (slow) | ✅ `MATCH 'query'` (BM25) |
| JOINs | ✅ | ✅ `JOIN ... ON` (v2.0) |
| GROUP BY / HAVING | ✅ | ✅ With aggregates (v2.0) |
| ORDER BY | ✅ | ✅ Columns & similarity (v2.0) |
| UNION/INTERSECT/EXCEPT | ✅ | ✅ Set operations (v2.0) |
| Subqueries | ✅ | ❌ Not supported |

---

## Grammar (BNF)

```bnf
<query>         ::= <compound_query> [";"]

<compound_query>::= <select_stmt> [<set_operator> <select_stmt>]
<set_operator>  ::= "UNION" ["ALL"] | "INTERSECT" | "EXCEPT"

<select_stmt>   ::= "SELECT" <select_list>
                    "FROM" <identifier>
                    [<join_clause>*]
                    [<where_clause>]
                    [<group_by_clause>]
                    [<having_clause>]
                    [<order_by_clause>]
                    [<limit_clause>]
                    [<offset_clause>]
                    [<with_clause>]

<join_clause>   ::= [<join_type>] "JOIN" <identifier> ["AS" <identifier>] <join_spec>
<join_type>     ::= "LEFT" | "RIGHT" | "FULL" | ε
<join_spec>     ::= ("ON" <join_condition>) | ("USING" "(" <column_list> ")")
<join_condition>::= <column_ref> "=" <column_ref>
<column_ref>    ::= <identifier> "." <identifier>

<group_by_clause> ::= "GROUP" "BY" <identifier> ("," <identifier>)*
<having_clause>   ::= "HAVING" <having_condition>
<having_condition>::= <having_term> (("AND" | "OR") <having_term>)*
<having_term>     ::= <aggregate_function> <compare_op> <value>

<order_by_clause> ::= "ORDER" "BY" <order_item> ("," <order_item>)*
<order_item>      ::= (<identifier> | <similarity_func>) ["ASC" | "DESC"]
<similarity_func> ::= "similarity" "(" <identifier> "," <vector_value> ")"

<select_list>   ::= "*" | <column_list>
<column_list>   ::= <column> ("," <column>)*
<column>        ::= <column_name> ["AS" <identifier>]
<column_name>   ::= <identifier> ["." <identifier>]

<where_clause>  ::= "WHERE" <or_expr>

<or_expr>       ::= <and_expr> ("OR" <and_expr>)*
<and_expr>      ::= <primary_expr> ("AND" <primary_expr>)*

<primary_expr>  ::= "(" <or_expr> ")"
                  | <vector_search>
                  | <match_expr>
                  | <in_expr>
                  | <between_expr>
                  | <like_expr>
                  | <is_null_expr>
                  | <compare_expr>

<vector_search> ::= "vector" "NEAR" [<metric>] <vector_value>
<metric>        ::= "COSINE" | "EUCLIDEAN" | "DOT"
<vector_value>  ::= <vector_literal> | <parameter>
<vector_literal>::= "[" <float> ("," <float>)* "]"

<match_expr>    ::= <identifier> "MATCH" <string>

<in_expr>       ::= <identifier> "IN" "(" <value_list> ")"
<value_list>    ::= <value> ("," <value>)*

<between_expr>  ::= <identifier> "BETWEEN" <value> "AND" <value>

<like_expr>     ::= <identifier> "LIKE" <string>

<is_null_expr>  ::= <identifier> "IS" ["NOT"] "NULL"

<compare_expr>  ::= <identifier> <compare_op> <value>
<compare_op>    ::= "=" | "!=" | "<>" | ">" | "<" | ">=" | "<="

<limit_clause>  ::= "LIMIT" <integer>
<offset_clause> ::= "OFFSET" <integer>
<with_clause>   ::= "WITH" "(" <option_list> ")"
<option_list>   ::= <option> ("," <option>)*
<option>        ::= <identifier> "=" <option_value>
<option_value>  ::= <string> | <integer> | <float> | <boolean>

<value>         ::= <float> | <integer> | <string> | <boolean> | "NULL" | <parameter>
<parameter>     ::= "$" <identifier>
<boolean>       ::= "TRUE" | "FALSE"
<string>        ::= "'" <characters> "'"
<integer>       ::= ["-"] <digits>
<float>         ::= ["-"] <digits> "." <digits>
<identifier>    ::= (<letter> | "_") (<alphanumeric> | "_")*
```

---

## Data Types

| Type | Description | Examples |
|------|-------------|----------|
| `INTEGER` | 64-bit signed integer | `42`, `-100`, `0` |
| `FLOAT` | 64-bit floating point | `3.14`, `-0.5`, `100.0` |
| `STRING` | UTF-8 string (single quotes) | `'hello'`, `'VelesDB'` |
| `BOOLEAN` | Boolean value | `TRUE`, `FALSE` |
| `NULL` | Null value | `NULL` |
| `VECTOR` | Float32 array (via parameter) | `$query_vector` |

### Type Coercion

- Integers are automatically promoted to floats in comparisons
- Strings must match exactly (case-sensitive by default)
- NULL comparisons use `IS NULL` / `IS NOT NULL`

---

## Operators

### Comparison Operators

| Operator | Description | Example |
|----------|-------------|---------|
| `=` | Equal | `category = 'tech'` |
| `!=` or `<>` | Not equal | `status != 'deleted'` |
| `>` | Greater than | `price > 100` |
| `<` | Less than | `score < 0.5` |
| `>=` | Greater or equal | `rating >= 4` |
| `<=` | Less or equal | `count <= 10` |

### Logical Operators

| Operator | Description | Example |
|----------|-------------|---------|
| `AND` | Logical AND | `a = 1 AND b = 2` |
| `OR` | Logical OR | `a = 1 OR a = 2` |

**Precedence**: `AND` has higher precedence than `OR`. Use parentheses to override.

### Special Operators

| Operator | Description | Example |
|----------|-------------|---------|
| `NEAR` | Vector similarity search | `vector NEAR $v` |
| `MATCH` | BM25 full-text search | `content MATCH 'rust'` |
| `IN` | Value in list | `status IN ('a', 'b')` |
| `BETWEEN` | Range (inclusive) | `price BETWEEN 10 AND 100` |
| `LIKE` | Pattern matching | `title LIKE '%rust%'` |
| `IS NULL` | Null check | `deleted_at IS NULL` |
| `IS NOT NULL` | Non-null check | `created_at IS NOT NULL` |

---

## Clauses

### SELECT

```sql
-- All columns
SELECT * FROM documents

-- Specific columns
SELECT id, title, score FROM documents

-- Nested fields (dot notation)
SELECT id, metadata.author, metadata.tags FROM documents

-- Column aliases
SELECT title AS name, price AS cost FROM products
```

### FROM

Specifies the collection to query:

```sql
SELECT * FROM documents    -- Query 'documents' collection
SELECT * FROM products     -- Query 'products' collection
```

### WHERE

Filter conditions:

```sql
-- Simple condition
WHERE category = 'tech'

-- Vector search
WHERE vector NEAR $query_vector

-- Combined conditions
WHERE vector NEAR $v AND category = 'tech' AND price > 100

-- Complex logic
WHERE (status = 'active' OR status = 'pending') AND priority > 5
```

### LIMIT

Limit the number of results:

```sql
SELECT * FROM documents LIMIT 10      -- Return max 10 results
SELECT * FROM documents LIMIT 100     -- Return max 100 results
```

### OFFSET

Skip a number of results (for pagination):

```sql
SELECT * FROM documents LIMIT 10 OFFSET 20    -- Skip 20, return 10
```

### WITH (v0.8.0+)

Configure search parameters per-query:

```sql
-- Set search mode
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (mode = 'high_recall');

-- Set ef_search directly
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (ef_search = 512);

-- Multiple options
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 
WITH (mode = 'accurate', timeout_ms = 5000);
```

#### Available Options

| Option | Type | Values | Description |
|--------|------|--------|-------------|
| `mode` | string | `'fast'`, `'balanced'`, `'accurate'`, `'high_recall'`, `'perfect'` | Search quality preset |
| `ef_search` | integer | 16-4096 | HNSW ef_search parameter (overrides mode) |
| `timeout_ms` | integer | 100-300000 | Query timeout in milliseconds |
| `rerank` | boolean | `true`, `false` | Enable/disable reranking for quantized vectors |

#### Priority

Options follow this priority (highest to lowest):
1. `WITH` clause options (query-time)
2. Session settings (REPL `\set`)
3. Environment variables (`VELESDB_*`)
4. Configuration file (`velesdb.toml`)
5. Default values

#### Examples

```sql
-- Fast mode for autocomplete (low latency)
SELECT * FROM suggestions WHERE vector NEAR $v LIMIT 5 WITH (mode = 'fast');

-- Perfect mode for validation (100% recall guaranteed)
SELECT * FROM legal_docs WHERE vector NEAR $v LIMIT 10 WITH (mode = 'perfect');

-- Custom ef_search for fine-tuned recall/latency
SELECT * FROM products WHERE vector NEAR $v LIMIT 20 WITH (ef_search = 300);

-- Combine with filters
SELECT * FROM articles 
WHERE vector NEAR $v AND category = 'science'
LIMIT 10 
WITH (mode = 'accurate', timeout_ms = 10000);
```

---

## Vector Search

### Basic Syntax

```sql
SELECT * FROM documents WHERE vector NEAR $query_vector LIMIT 10
```

### Distance Metrics

| Metric | Keyword | Best For |
|--------|---------|----------|
| Cosine Similarity | `COSINE` (default) | Text embeddings, normalized vectors |
| Euclidean Distance | `EUCLIDEAN` | Spatial data, image features |
| Dot Product | `DOT` | Pre-normalized vectors, MIPS |

```sql
-- Cosine (default)
WHERE vector NEAR $v

-- Explicit cosine
WHERE vector NEAR COSINE $v

-- Euclidean
WHERE vector NEAR EUCLIDEAN $v

-- Dot product
WHERE vector NEAR DOT $v
```

### Vector Literals

You can inline small vectors (not recommended for production):

```sql
WHERE vector NEAR [0.1, 0.2, 0.3, 0.4]
```

### Combined with Filters

```sql
-- Vector search + metadata filter
SELECT * FROM products
WHERE vector NEAR $embedding
  AND category = 'electronics'
  AND price < 1000
LIMIT 20
```

---

## Full-Text Search

### BM25 Search

Use `MATCH` for full-text search with BM25 ranking:

```sql
-- Search in content field
SELECT * FROM documents
WHERE content MATCH 'rust programming'
LIMIT 10
```

### Hybrid Search (Vector + Text)

Combine vector similarity with text relevance:

```sql
-- Hybrid search
SELECT * FROM documents
WHERE vector NEAR $v
  AND content MATCH 'machine learning'
LIMIT 10
```

---

## Aggregations & GROUP BY

*New in VelesQL v2.0*

### Aggregate Functions

| Function | Description | Example |
|----------|-------------|---------|
| `COUNT(*)` | Count all rows | `SELECT COUNT(*) FROM products` |
| `COUNT(col)` | Count non-null values | `SELECT COUNT(price) FROM products` |
| `SUM(col)` | Sum of values | `SELECT SUM(price) FROM orders` |
| `AVG(col)` | Average value | `SELECT AVG(rating) FROM reviews` |
| `MIN(col)` | Minimum value | `SELECT MIN(price) FROM products` |
| `MAX(col)` | Maximum value | `SELECT MAX(score) FROM results` |

### GROUP BY

Group rows by one or more columns:

```sql
-- Single column grouping
SELECT category, COUNT(*) FROM products GROUP BY category

-- Multiple columns
SELECT category, status, COUNT(*) FROM products GROUP BY category, status

-- With aggregates
SELECT category, AVG(price), MIN(price), MAX(price) 
FROM products 
GROUP BY category
```

### HAVING

Filter groups after aggregation (like WHERE but for groups):

```sql
-- Filter by aggregate value
SELECT category, COUNT(*) 
FROM products 
GROUP BY category 
HAVING COUNT(*) > 10

-- Multiple conditions with AND/OR
SELECT category, AVG(price) 
FROM products 
GROUP BY category 
HAVING COUNT(*) > 5 AND AVG(price) > 50

-- OR conditions
SELECT category, SUM(quantity) 
FROM products 
GROUP BY category 
HAVING SUM(quantity) > 100 OR AVG(price) > 200
```

### WITH (max_groups)

Limit the number of groups to prevent memory issues:

```sql
SELECT category, COUNT(*) 
FROM products 
GROUP BY category 
WITH (max_groups = 100)
```

---

## JOIN

*New in VelesQL v2.0*

### Basic JOIN

Join two collections on a condition:

```sql
-- Simple join
SELECT * FROM orders 
JOIN customers ON orders.customer_id = customers.id

-- With alias
SELECT * FROM orders 
JOIN customers AS c ON orders.customer_id = c.id

-- Multiple joins
SELECT * FROM orders 
JOIN customers ON orders.customer_id = customers.id
JOIN products ON orders.product_id = products.id
```

### JOIN with WHERE

Combine joins with filtering:

```sql
SELECT * FROM orders 
JOIN customers ON orders.customer_id = customers.id
WHERE status = 'completed'
LIMIT 100
```

---

## Set Operations

*New in VelesQL v2.0*

Combine results from multiple queries.

### UNION

Merge results from two queries, removing duplicates:

```sql
SELECT name, email FROM active_users
UNION
SELECT name, email FROM archived_users
```

### UNION ALL

Merge results keeping all rows (including duplicates):

```sql
SELECT * FROM orders_2024
UNION ALL
SELECT * FROM orders_2025
```

### INTERSECT

Return only rows that appear in both queries:

```sql
SELECT id FROM premium_users
INTERSECT
SELECT id FROM active_users
```

### EXCEPT

Return rows from first query that don't appear in second:

```sql
SELECT id FROM all_users
EXCEPT
SELECT id FROM banned_users
```

---

## Parameters

Parameters provide safe, injection-free query binding.

### Syntax

Parameters are prefixed with `$`:

```sql
SELECT * FROM documents
WHERE vector NEAR $query_vector
  AND category = $cat
LIMIT $limit
```

### Usage (REST API)

```json
{
  "query": "SELECT * FROM docs WHERE vector NEAR $v AND category = $cat LIMIT 10",
  "params": {
    "v": [0.1, 0.2, 0.3, ...],
    "cat": "tech"
  }
}
```

### Usage (Rust)

```rust
use velesdb_core::velesql::Parser;

let query = Parser::parse("SELECT * FROM docs WHERE vector NEAR $v LIMIT 10")?;
// Bind parameters at execution time
```

### Usage (Python)

```python
results = collection.query(
    "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10",
    params={"v": query_vector}
)
```

---

## Examples

### Example 1: Simple Vector Search

```sql
SELECT * FROM documents
WHERE vector NEAR $query_embedding
LIMIT 10
```

**Use case**: Semantic search, RAG retrieval

### Example 2: Filtered Vector Search

```sql
SELECT id, title, score FROM products
WHERE vector NEAR $embedding
  AND category = 'electronics'
  AND price BETWEEN 100 AND 500
  AND in_stock = TRUE
LIMIT 20
```

**Use case**: E-commerce product recommendations

### Example 3: Full-Text Search

```sql
SELECT * FROM articles
WHERE content MATCH 'rust async programming'
LIMIT 10
```

**Use case**: Documentation search, blog search

### Example 4: Hybrid Search

```sql
SELECT * FROM knowledge_base
WHERE vector NEAR COSINE $query_vector
  AND content MATCH 'vector database'
LIMIT 5
```

**Use case**: RAG with keyword boost

### Example 5: Complex Filtering

```sql
SELECT id, title, metadata.author FROM documents
WHERE vector NEAR $v
  AND (category = 'tech' OR category = 'science')
  AND published_at IS NOT NULL
  AND tags IN ('ai', 'ml', 'rust')
LIMIT 15
```

**Use case**: Advanced faceted search

### Example 6: Pattern Matching

```sql
SELECT * FROM users
WHERE name LIKE 'John%'
  AND email LIKE '%@gmail.com'
LIMIT 50
```

**Use case**: User search

### Example 7: Null Handling

```sql
SELECT * FROM tasks
WHERE assigned_to IS NOT NULL
  AND completed_at IS NULL
LIMIT 100
```

**Use case**: Find incomplete assigned tasks

### Example 8: Range Query

```sql
SELECT * FROM logs
WHERE timestamp BETWEEN 1703289600 AND 1703376000
  AND level = 'error'
LIMIT 1000
```

**Use case**: Time-range log search

### Example 9: Pagination

```sql
SELECT * FROM products
WHERE category = 'books'
LIMIT 20 OFFSET 40
```

**Use case**: Page 3 of results (20 items per page)

### Example 10: Euclidean Distance

```sql
SELECT * FROM images
WHERE vector NEAR EUCLIDEAN $image_embedding
LIMIT 10
```

**Use case**: Image similarity search

---

## Limitations

### Current Limitations

| Feature | Status | Workaround |
|---------|--------|------------|
| `LEFT/RIGHT/FULL JOIN` | ❌ Not supported | Use basic `JOIN` |
| Subqueries | ❌ Not supported | Use multiple queries |
| `DISTINCT` | ❌ Not supported | Dedupe in application |
| `ORDER BY` aggregates | ❌ Not supported | Sort in application |
| Nested `GROUP BY` fields | ❌ Not supported | Use simple column names |

### ✅ Implemented Features (v2.0)

| Feature | Status | Notes |
|---------|--------|-------|
| `JOIN ... ON` | ✅ Supported | Basic inner join |
| `GROUP BY` | ✅ Supported | With aggregates |
| `HAVING` | ✅ Supported | AND/OR operators |
| `ORDER BY` | ✅ Supported | Columns, similarity() |
| `UNION/INTERSECT/EXCEPT` | ✅ Supported | Set operations |
| `COUNT/SUM/AVG/MIN/MAX` | ✅ Supported | Aggregate functions |
| `WITH (options)` | ✅ Supported | Query-time config |

### Planned Features (Roadmap)

- `LEFT/RIGHT/FULL OUTER JOIN`
- `JOIN USING (column)`
- `ORDER BY` with aggregates
- `EXPLAIN` for query analysis
- Prepared query caching

---

## Troubleshooting

### Common Errors

#### Syntax Error: Expected SELECT

```
Error: Expected SELECT statement
```

**Solution**: Ensure query starts with `SELECT`:
```sql
-- Wrong
FROM documents LIMIT 10

-- Correct
SELECT * FROM documents LIMIT 10
```

#### Unknown Column

```
Error: Unknown column 'unknown_field'
```

**Solution**: Check that the field exists in your payload.

#### Invalid Parameter

```
Error: Parameter '$v' not provided
```

**Solution**: Ensure all `$param` references are provided in the `params` object.

#### Type Mismatch

```
Error: Cannot compare string to integer
```

**Solution**: Use matching types:
```sql
-- Wrong
WHERE price = 'expensive'

-- Correct
WHERE price > 100
```

### Performance Tips

1. **Always use LIMIT**: Without LIMIT, VelesDB may return many results
2. **Filter early**: Place high-selectivity filters first
3. **Use parameters**: Avoid string concatenation for safety and caching
4. **Prefer MATCH over LIKE**: `MATCH` uses BM25 index, `LIKE` scans all

---

## EXPLAIN Query Plan

The `QueryPlan` API allows you to inspect how VelesDB will execute a query before running it.

### Usage

```rust
use velesdb_core::velesql::{Parser, QueryPlan};

let query = Parser::parse("SELECT * FROM docs WHERE vector NEAR $v AND category = 'tech' LIMIT 10")?;
let plan = QueryPlan::from_select(&query.select);

// Human-readable tree format
println!("{}", plan.to_tree());

// JSON format for tooling
let json = plan.to_json()?;
```

### Output Format

```
Query Plan:
├─ VectorSearch
│   ├─ Collection: docs
│   ├─ Metric: cosine
│   ├─ ef_search: 100
│   └─ Candidates: 10
├─ Filter
│   ├─ Conditions: category = ?
│   └─ Selectivity: 50.0%
└─ Limit: 10

Estimated cost: 0.105ms
Index used: HNSW
Filter strategy: post-filtering (low selectivity)
```

### Plan Nodes

| Node | Description |
|------|-------------|
| `VectorSearch` | HNSW approximate nearest neighbor search |
| `Filter` | Metadata filter (pre or post vector search) |
| `Limit` | Maximum results to return |
| `Offset` | Skip N results |
| `TableScan` | Full collection scan (no vector search) |

### Filter Strategies

| Strategy | When Used | Performance |
|----------|-----------|-------------|
| `pre-filtering` | High selectivity (<10% match) | Filter first, then vector search |
| `post-filtering` | Low selectivity (>10% match) | Vector search first, then filter |

### Performance

| Operation | Time |
|-----------|------|
| `QueryPlan::from_select()` (simple) | ~61 ns |
| `QueryPlan::from_select()` (complex) | ~398 ns |
| `to_tree()` | ~893 ns |
| `to_json()` | ~702 ns |

---

## Parser Performance

| Query Type | Parse Time | Throughput |
|------------|------------|------------|
| Simple SELECT | ~528 ns | 1.9M queries/sec |
| Vector search | ~835 ns | 1.2M queries/sec |
| Complex (5+ conditions) | ~3.6 µs | 277K queries/sec |

---

## See Also

- [API Reference](./api-reference.md)
- [Benchmarks](./BENCHMARKS.md)
- [REST API Documentation](./api-reference.md)
