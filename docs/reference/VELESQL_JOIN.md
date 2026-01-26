# VelesQL JOIN Reference

*Version 0.3.0 — January 2026*

This document describes the JOIN syntax for cross-store queries in VelesDB, enabling queries that combine **Graph** (vector + relationships) and **ColumnStore** (structured tabular) data.

---

## Overview

VelesQL supports **cross-store JOINs** that bridge the vector/graph layer with relational column stores:

```sql
MATCH (p:Product)-[:HAS_PRICE]->(price)
JOIN prices ON prices.product_id = p.id
WHERE vector NEAR $query AND prices.amount < 100
RETURN p.name, prices.amount, prices.currency
```

---

## Syntax

### Basic JOIN

```bnf
<join_clause> ::= [<join_type>] "JOIN" <table_name> [<alias>] <join_spec>
<join_type>   ::= "LEFT" | "RIGHT" | "FULL" | ε
<alias>       ::= "AS" <identifier>
<join_spec>   ::= "ON" <join_condition> | "USING" "(" <column_list> ")"
<join_condition> ::= <column_ref> "=" <column_ref>
<column_ref> ::= <table_or_alias> "." <column_name>
```

### JOIN Types (EPIC-040 US-003)

| Type | Behavior |
|------|----------|
| `JOIN` / `INNER JOIN` | Only matching rows from both sides |
| `LEFT JOIN` | All rows from left, matching from right (NULL if no match) |
| `RIGHT JOIN` | All rows from right, matching from left (NULL if no match) |
| `FULL JOIN` | All rows from both sides (NULL where no match) |

### Examples

```sql
-- Simple JOIN with graph pattern
MATCH (doc:Document)
JOIN metadata ON metadata.doc_id = doc.id
WHERE vector NEAR $query
LIMIT 20

-- LEFT JOIN - keep all orders even without customers
SELECT * FROM orders 
LEFT JOIN customers ON orders.customer_id = customers.id

-- RIGHT JOIN - keep all customers even without orders
SELECT * FROM orders 
RIGHT JOIN customers ON orders.customer_id = customers.id

-- FULL JOIN - keep all from both sides
SELECT * FROM orders 
FULL JOIN customers ON orders.customer_id = customers.id

-- JOIN with alias
SELECT * FROM orders 
JOIN customers AS c ON orders.customer_id = c.id
WHERE c.status = 'active'

-- JOIN with USING clause (shared column name)
SELECT * FROM orders 
JOIN customers USING (customer_id)

-- USING with multiple columns
SELECT * FROM orders 
JOIN customers USING (customer_id, region_id)
```

---

## Data Sources

### Graph Layer (MATCH)

The graph layer stores:
- **Vectors**: Embeddings for similarity search
- **Properties**: JSON payload per node
- **Relationships**: Edges between nodes

```sql
MATCH (d:Doc)  -- d refers to graph node
```

### ColumnStore (JOIN)

The column store contains:
- **Tables**: Structured relational data
- **Indexes**: B-tree indexes on columns
- **Types**: Strongly typed columns (INT, FLOAT, STRING, etc.)

```sql
JOIN prices ON prices.product_id = d.id  -- prices is a ColumnStore table
```

---

## Column Qualification

### Rule: Always Qualify ColumnStore Columns

```sql
-- ✅ Correct: qualified column names
WHERE prices.amount < 100

-- ❌ Incorrect: unqualified column (ambiguous)
WHERE amount < 100  -- Interpreted as graph property!
```

### Unqualified Names

Unqualified column names in a JOIN context default to the **graph layer**:

| Expression | Interpreted As | Source |
|------------|----------------|--------|
| `doc.title` | Graph property | Graph |
| `prices.amount` | ColumnStore column | ColumnStore |
| `title` | Graph property | Graph |
| `amount` | Graph property | Graph ⚠️ |

---

## Filter Pushdown

VelesDB automatically optimizes WHERE clauses by pushing filters to the appropriate store:

### Pushdown Analysis

```sql
MATCH (p:Product)
JOIN prices ON prices.product_id = p.id
WHERE vector NEAR $q           -- → Graph (vector search)
  AND p.category = 'electronics' -- → Graph (graph property)
  AND prices.amount < 500        -- → ColumnStore (qualified)
  AND quantity > 10              -- → Graph (unqualified = graph)
```

### Filter Classification

| Filter Type | Pushed To | Example |
|-------------|-----------|---------|
| Vector search | Graph | `vector NEAR $v` |
| Unqualified column | Graph | `category = 'tech'` |
| Qualified graph var | Graph | `p.name = 'X'` |
| Qualified table | ColumnStore | `prices.amount > 100` |
| Mixed (OR across stores) | Post-JOIN | `p.x = 1 OR prices.y = 2` |

---

## Execution Flow

```
1. Parse Query
   ↓
2. Analyze Filters (PushdownAnalysis)
   ├── graph_filters → Graph Engine
   ├── column_store_filters → ColumnStore Engine
   └── post_join_filters → Applied after JOIN
   ↓
3. Execute Graph Query (vector + graph filters)
   ↓
4. Execute ColumnStore Query (column filters)
   ↓
5. JOIN Results (batch adaptive)
   ↓
6. Apply Post-JOIN Filters
   ↓
7. Return Results
```

---

## Best Practices

### 1. Always Qualify Ambiguous Columns

```sql
-- Good: explicit qualification
WHERE prices.amount < 100 AND p.score > 0.8

-- Bad: relies on default behavior
WHERE amount < 100  -- Which amount?
```

### 2. Push Filters Early

Place high-selectivity filters in the appropriate store:

```sql
-- Good: filters pushed to each store
WHERE vector NEAR $v 
  AND prices.amount < 50      -- Pushed to ColumnStore
  AND p.category = 'books'    -- Pushed to Graph

-- Less optimal: everything post-join
WHERE prices.amount < 50 OR p.category = 'books'  -- Can't be split
```

### 3. Use LIMIT

JOIN operations can be expensive. Always limit results:

```sql
MATCH (p:Product)
JOIN prices ON prices.product_id = p.id
WHERE vector NEAR $v
LIMIT 100  -- Always set a reasonable limit
```

### 4. Index ColumnStore JOIN Columns

Ensure JOIN columns have indexes in the ColumnStore:

```sql
-- Create index on the JOIN column
CREATE INDEX idx_product_id ON prices(product_id);
```

---

## Limitations

| Feature | Status | Notes |
|---------|--------|-------|
| Single JOIN | ✅ | One JOIN per query |
| Multiple JOINs | ❌ | Planned for v0.4 |
| LEFT/RIGHT JOIN | ❌ | Only INNER JOIN |
| Self-JOIN | ❌ | Not supported |
| Subqueries in JOIN | ❌ | Not supported |

---

## Troubleshooting

### "Unknown table in JOIN"

```
Error: Table 'unknown_table' not found in ColumnStore
```

**Solution**: Ensure the table exists in the ColumnStore before querying.

### "Ambiguous column reference"

```
Error: Column 'amount' is ambiguous - qualify with table name
```

**Solution**: Use `prices.amount` instead of `amount`.

### "Cannot push filter to both stores"

This warning indicates an OR condition spans both stores:

```sql
WHERE prices.x = 1 OR p.y = 2  -- Cannot be split
```

**Solution**: Restructure query or accept post-JOIN filtering.

---

## Performance

| Operation | Typical Time |
|-----------|--------------|
| Filter pushdown analysis | < 1ms |
| Graph query (HNSW) | 1-10ms |
| ColumnStore query | 1-5ms |
| JOIN (batch adaptive) | 2-20ms |
| Total (typical) | 5-35ms |

---

## See Also

- [VelesQL Specification](./VELESQL_SPEC.md)
- [Multi-Model Queries Guide](../guides/MULTIMODEL_QUERIES.md)
- [ORDER BY Expressions](./VELESQL_ORDERBY.md)
