# Python VelesQL parser API

Moved out of [`crates/velesdb-python/README.md`](../../crates/velesdb-python/README.md)
to keep that file under the documentation line budget.

VelesDB (since v1.7.2) exposes the VelesQL parser as a standalone Python API for query
introspection, validation, and tooling integration. Parse any VelesQL statement
into a `ParsedStatement` object and inspect its structure without executing it.

```python
from velesdb import VelesQL

# Parse a query and inspect its structure
parsed = VelesQL.parse("SELECT id, title FROM documents WHERE category = 'tech' ORDER BY date DESC LIMIT 20")

print(parsed.collection_name)  # "documents"
print(parsed.columns)          # ["id", "title"]
print(parsed.limit)            # 20
print(parsed.offset)           # None
print(parsed.has_where_clause())   # True
print(parsed.has_order_by())       # True
print(parsed.has_vector_search())  # False
print(parsed.order_by)            # [("date", "DESC")]
print(parsed.is_select())         # True
print(parsed.is_match())          # False
```

**Validate queries without parsing:**

```python
# Fast validation (no full parse tree)
VelesQL.is_valid("SELECT * FROM docs LIMIT 10")     # True
VelesQL.is_valid("SELEC * FROM docs")                # False
```

**Inspect advanced query features:**

```python
# Vector search detection
parsed = VelesQL.parse("SELECT * FROM docs WHERE vector NEAR $q LIMIT 5")
print(parsed.has_vector_search())  # True

# MATCH (graph) queries
parsed = VelesQL.parse("MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name")
print(parsed.is_match())    # True
print(parsed.is_select())   # False

# GROUP BY, HAVING, JOINs, DISTINCT
parsed = VelesQL.parse("SELECT DISTINCT category, COUNT(*) FROM products GROUP BY category")
print(parsed.has_distinct())   # True
print(parsed.has_group_by())   # True
print(parsed.group_by)         # ["category"]

# JOIN inspection
parsed = VelesQL.parse(
    "SELECT * FROM orders JOIN products ON orders.product_id = products.id"
)
print(parsed.has_joins())   # True
print(parsed.join_count)    # 1
```

**Error handling with typed exceptions:**

```python
from velesdb import VelesQL, VelesQLSyntaxError

try:
    parsed = VelesQL.parse("SELEC * FROM docs")
except VelesQLSyntaxError as e:
    print(f"Syntax error: {e}")
```

Key parameters for `ParsedStatement`:

| Property / Method | Returns | Description |
|-------------------|---------|-------------|
| `collection_name` | `str` or `None` | FROM clause collection name |
| `columns` | `list[str]` | Selected columns (or `["*"]`) |
| `limit` | `int` or `None` | LIMIT value |
| `offset` | `int` or `None` | OFFSET value |
| `order_by` | `list[tuple[str, str]]` | (column, "ASC"/"DESC") pairs |
| `group_by` | `list[str]` | GROUP BY columns |
| `table_alias` | `str` or `None` | First FROM alias |
| `table_aliases` | `list[str]` | All aliases in scope |
| `join_count` | `int` | Number of JOIN clauses |
| `is_select()` | `bool` | True for SELECT queries |
| `is_match()` | `bool` | True for MATCH (graph) queries |
| `has_where_clause()` | `bool` | True if WHERE is present |
| `has_vector_search()` | `bool` | True if NEAR clause is present |
| `has_order_by()` | `bool` | True if ORDER BY is present |
| `has_group_by()` | `bool` | True if GROUP BY is present |
| `has_having()` | `bool` | True if HAVING is present |
| `has_joins()` | `bool` | True if JOINs are present |
| `has_distinct()` | `bool` | True if SELECT DISTINCT |
| `has_fusion()` | `bool` | True if USING FUSION is present |

Executing (rather than inspecting) VelesQL from Python is documented in
[PYTHON_API_REFERENCE.md](PYTHON_API_REFERENCE.md) — `collection.query()`,
`collection.query_ids()`, `collection.explain()` and
`collection.match_query()`.

---

Last updated: 2026-07-25 · Applies to: velesdb-core 4.0.0
