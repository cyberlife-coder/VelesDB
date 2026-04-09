# Graph Patterns Guide

*Version 1.12.0 -- April 2026*

Practical guide for using VelesQL `MATCH` graph patterns in VelesDB.

---

## How MATCH Works

MATCH graph patterns operate within a **single collection's internal graph store**. When you write:

```sql
MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)
RETURN doc.title, author.name
```

VelesDB iterates over points in the current collection, filters by `_labels` payload arrays, follows edges in the collection's edge store, and projects the requested fields.

Both `doc` and `author` are points **in the same collection**. Labels are metadata tags, not collection names.

---

## Setting Up Data for MATCH

Every point that participates in label-based MATCH patterns needs a `_labels` array in its payload.

```bash
curl -X POST http://localhost:8080/collections/knowledge/points \
  -H "Content-Type: application/json" \
  -d '{"points": [
    {"id": 1, "vector": [0.1, 0.2, 0.3, 0.4],
     "payload": {"_labels": ["Document"], "title": "HNSW Overview"}},
    {"id": 2, "vector": [0.5, 0.6, 0.7, 0.8],
     "payload": {"_labels": ["Person"], "name": "Alice"}}
  ]}'
```

```rust
collection.upsert(1, &[0.1, 0.2, 0.3, 0.4], json!({
    "_labels": ["Document"], "title": "HNSW Overview"
}))?;
collection.upsert(2, &[0.5, 0.6, 0.7, 0.8], json!({
    "_labels": ["Person"], "name": "Alice"
}))?;
```

A point can have multiple labels: `"_labels": ["Document", "Published", "Reviewed"]`.

---

## Creating Edges

Edges must exist in the collection's edge store before MATCH can traverse them.

```bash
curl -X POST http://localhost:8080/collections/knowledge/graph/edges \
  -H "Content-Type: application/json" \
  -d '{"source": 1, "target": 2, "label": "AUTHORED_BY", "properties": {"year": 2026}}'
```

```rust
collection.add_edge(1, 2, "AUTHORED_BY", json!({"year": 2026}))?;
```

---

## Example Queries

```sql
-- Simple traversal
MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)
RETURN doc.title, author.name

-- With vector similarity
MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)
WHERE similarity(doc.embedding, $question) > 0.8
RETURN author.name, doc.title
ORDER BY similarity() DESC LIMIT 5

-- Co-purchase recommendations
MATCH (product:Product)-[:BOUGHT_TOGETHER]->(related:Product)
WHERE similarity(product.embedding, $query) > 0.7
RETURN related.name, related.price
```

---

## Common Pitfalls

### 1. Missing `_labels` array

MATCH returns no results even though points and edges exist. Points without `"_labels"` in their payload are silently skipped. **Fix**: add `"_labels": ["YourLabel"]` to each point's payload.

### 2. Edges in the wrong collection

MATCH only sees edges in the current collection's edge store. If edges were created in a different collection, MATCH will not find them. **Fix**: create edges in the same collection where your points live.

### 3. Expecting cross-collection traversal

MATCH is scoped to a single collection. It does not join across collections. **Fix**: store all relevant points and edges in one collection, or use the direct graph API to query each collection separately and merge results in application code.

### 4. Label mismatch (case-sensitive)

Labels are case-sensitive. `"Product"` and `"product"` are different labels. Ensure `_labels` values match the MATCH pattern exactly.

---

## MATCH vs Direct Graph API

| Feature | VelesQL MATCH | Direct Graph API |
|---------|--------------|-----------------|
| **Scope** | Single collection | Single `GraphCollection` |
| **Query style** | Declarative (SQL-like) | Imperative (method calls) |
| **Vector + graph fusion** | Built-in (`similarity()` in WHERE) | Manual (search + traverse separately) |
| **Cross-collection** | Not supported | Possible via application code |

**Use MATCH when**: you need graph patterns combined with vector similarity or metadata filtering in a single query within one collection.

**Use the direct graph API when**: you need to traverse a standalone `GraphCollection`, combine results from multiple collections, or perform BFS/DFS with custom logic.

```rust
// Direct graph API example
let outgoing = graph_collection.get_outgoing(node_id)?;
let bfs_results = graph_collection.traverse_bfs(start_id, max_depth)?;
```

---

## Parallel BFS Traversal

Multi-source BFS launches concurrent traversals from multiple starting nodes with automatic deduplication by path signature. Available across all components.

### REST API

```bash
curl -X POST http://localhost:8080/collections/kg/graph/traverse/parallel \
  -H "Content-Type: application/json" \
  -d '{"sources": [1, 5, 10], "max_depth": 3, "limit": 100}'
```

### Python

```python
results = graph_collection.traverse_bfs_parallel(
    source_ids=[1, 5, 10],
    max_depth=3,
    limit=100
)
```

### Tauri

```javascript
const results = await invoke('plugin:velesdb|traverse_graph_parallel', {
  request: { collection: 'kg', sources: [1, 5, 10], maxDepth: 3, limit: 100 }
});
```

### Mobile (UniFFI)

```swift
let results = try graphStore.bfsTraverseParallel(sourceIds: [1, 5, 10], maxDepth: 3, limit: 100)
```

---

## Cross-Collection MATCH (`@collection`)

By default, MATCH operates within a single collection. The `@collection` annotation
lets you enrich results with data from other collections.

### Syntax

```sql
MATCH (p:Product)-[:STORED_IN]->(w:Warehouse@inventory)
RETURN p.name, w.price, w.stock
LIMIT 20
```

- `p:Product` — resolved from the primary collection (the one with edges)
- `w:Warehouse@inventory` — after traversal, node `w`'s payload is looked up from the `inventory` collection

### How it works

1. The MATCH query executes on the primary collection (specified via `_collection` param or `\use` in REPL)
2. Graph traversal follows edges in that collection
3. After traversal, for each node annotated with `@collection`, the engine looks up the node's payload from the named collection
4. Enriched fields are merged into the result, prefixed with the node alias

### Example: E-commerce catalog

```python
# Python SDK
params = {"_collection": "catalog_graph"}
results = db.query(
    """MATCH (p:Product)-[:STORED_IN]->(inv:Inventory@inventory)
       WHERE p.category = 'audio'
       RETURN p.name, inv.price, inv.stock
       LIMIT 20""",
    params=params
)
```

### REST API

```bash
curl -X POST http://localhost:8080/query \
  -H "Content-Type: application/json" \
  -d '{
    "query": "MATCH (p:Product)-[:STORED_IN]->(inv:Inventory@inventory) RETURN p, inv LIMIT 20",
    "collection": "catalog_graph",
    "params": {}
  }'
```

### Limitations

- The graph traversal (edges) always runs on the primary collection
- `@collection` only enriches payloads — it does not change which nodes are traversed
- If the annotated collection doesn't exist, enrichment is silently skipped
- Cross-collection vector search (`similarity()` on an annotated node) is not yet supported

---

## Related Documentation

- [VelesQL Specification](../VELESQL_SPEC.md) -- Full VelesQL language reference
- [Search Modes Guide](SEARCH_MODES.md) -- Configuring recall vs latency
- [Agent Memory Guide](AGENT_MEMORY.md) -- AI agent memory subsystems
