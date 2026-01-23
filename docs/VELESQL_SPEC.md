# VelesQL Language Specification

> SQL-like query language for vector search in VelesDB.

## Overview

VelesQL is a SQL-inspired query language designed specifically for vector similarity search. It combines familiar SQL syntax with vector-specific operations like `NEAR` for semantic search.

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

## Grammar (EBNF)

```ebnf
query       = select_stmt ;
select_stmt = "SELECT" select_list "FROM" identifier
              [where_clause] [limit_clause] [offset_clause] [with_clause] ;

select_list = "*" | column_list ;
column_list = column ("," column)* ;
column      = identifier ("." identifier)* ;

where_clause  = "WHERE" or_expr ;
or_expr       = and_expr ("OR" and_expr)* ;
and_expr      = condition ("AND" condition)* ;
condition     = comparison | vector_search | similarity_cond | in_cond 
              | between_cond | like_cond | is_null_cond | "(" or_expr ")" ;

vector_search   = "vector" "NEAR" vector_expr ;
similarity_cond = "similarity" "(" identifier "," vector_expr ")" compare_op number ;
vector_expr   = "$" identifier | "[" number ("," number)* "]" ;

comparison    = identifier compare_op value ;
compare_op    = "=" | "!=" | "<>" | ">" | ">=" | "<" | "<=" ;

in_cond       = identifier "IN" "(" value ("," value)* ")" ;
between_cond  = identifier "BETWEEN" value "AND" value ;
like_cond     = identifier "LIKE" string ;
is_null_cond  = identifier "IS" ["NOT"] "NULL" ;

limit_clause  = "LIMIT" integer ;
offset_clause = "OFFSET" integer ;

with_clause   = "WITH" "(" with_option ("," with_option)* ")" ;
with_option   = identifier "=" value ;

value         = string | number | boolean | "NULL" ;
identifier    = letter (letter | digit | "_")* ;
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
