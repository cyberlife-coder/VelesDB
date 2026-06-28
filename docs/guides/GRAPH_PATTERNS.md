# Graph Patterns Guide

*Version 3.4.0 -- May 2026*

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

MATCH traversal is scoped to the primary collection. `@collection` annotations can enrich bound node payloads from another collection after traversal, but they do not make edges span multiple graph stores. **Fix**: keep edges in the primary graph collection, then use `alias@collection` for payload enrichment when needed.

### 4. Label mismatch (case-sensitive)

Labels are case-sensitive. `"Product"` and `"product"` are different labels. Ensure `_labels` values match the MATCH pattern exactly.

---

## MATCH vs Direct Graph API

| Feature | VelesQL MATCH | Direct Graph API |
|---------|--------------|-----------------|
| **Scope** | Single collection | Single `GraphCollection` |
| **Query style** | Declarative (SQL-like) | Imperative (method calls) |
| **Vector + graph fusion** | Built-in (`similarity()` in WHERE) | Manual (search + traverse separately) |
| **Cross-collection** | Payload enrichment via `@collection` | Possible via application code |

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

## VelesQL MATCH / Cypher Support Surface

VelesQL's `MATCH` is a focused subset of Cypher, not a full implementation. The
supported surface is defined by the grammar (`crates/velesdb-core/src/velesql/grammar.pest`)
and the parser's complexity validation. The limits below are verified against the
grammar and parser; queries that violate them are rejected at parse time.

### Supported

- **A single linear pattern**: `(node)-[rel]->(node)` chains, e.g.
  `MATCH (a:A)-[:R]->(b:B)-[:S]->(c:C)`. The pattern is one connected path.
- **Directed and undirected relationships**: `-[:R]->`, `<-[:R]-`, `-[:R]-`.
- **Relationship type alternation**: `-[:R|S|T]->`.
- **Relationship aliases and inline properties**: `-[r:R {year: 2026}]->`.
- **`WHERE`** filtering, including `similarity(node.embedding, $param)`.
- **`RETURN`** of: an alias (`a`), a property access (`a.title`), `*`, or the
  zero-argument `similarity()` pseudo-function — each optionally `AS`-aliased.
- **`ORDER BY`** — supported expressions: the zero-arg `similarity()`, `depth`,
  a valid `alias.property` path, `similarity(field, $vec)`, and arithmetic over a
  **bare** property identifier (e.g. `ORDER BY year - 2000 DESC`). Arithmetic over
  a dotted path (e.g. `ORDER BY d.year - 2000`) is a parse error. Aggregates (no
  `GROUP BY`) and bare aliases are rejected with error `VELES-018` rather than
  silently ignored.
- **`LIMIT`**.
- **Cross-collection payload enrichment** via `alias@collection` (see above).

### Not supported

- **No comma-separated multi-pattern MATCH.** `MATCH (a), (b) RETURN a` is a
  parse error. The grammar's `graph_pattern` is one node-relationship chain only.
- **No `OPTIONAL MATCH`, no `WITH` pipeline, no `UNWIND`, no subqueries.** The
  `match_query` rule is `MATCH pattern [WHERE] RETURN [ORDER BY] [LIMIT]` — none
  of those keywords appear in it.
- **No aggregation over a MATCH in `RETURN`.** `RETURN count(a)` or any arbitrary
  function call (`RETURN upper(a.name)`) is a parse error. The only callable form
  in a MATCH `RETURN` is the zero-arg `similarity()`.

### Variable-length relationships — depth limits

Variable-length is written with an explicit bounded range: `-[*1..5]->`,
`-[*3]->` (exact), `-[:R*2..4]->`. Two limits apply, and they are easy to confuse:

- **Parse-time complexity budget — 32.** The largest range upper bound in a query
  may not exceed `DEFAULT_MAX_GRAPH_EXPANSION = 32`
  (`crates/velesdb-core/src/velesql/validation_types.rs`). `MATCH (a)-[*1..50]->(b)`
  is rejected with `[E007] Graph expansion exceeded: max=32`. An **unbounded**
  range (`-[*]->` or `-[*1..]->`) defaults to `u32::MAX` and is therefore rejected
  too — you must give an explicit upper bound `<= 32`. This budget is per
  relationship (the *maximum* single range bound), not a sum across the path, so
  `(a)-[*1..20]->(b)-[*1..20]->(c)` parses.
- **Runtime traversal guardrail — default 10.** On the standard VelesQL query path
  a `QueryContext` guardrail is installed with `DEFAULT_MAX_DEPTH = 10`
  (`crates/velesdb-core/src/guardrails/limits.rs`); a traversal exceeding that depth
  raises a guard-rail error. This is configurable (`QueryLimits::with_max_depth`),
  and the direct `execute_match` API (no context) does not apply it.

Net effect for everyday queries: keep variable-length upper bounds within the
runtime guardrail (default 10), and never above the parse-time budget of 32.

> Note: earlier drafts of this guide referenced a "10-hop" or "100-hop" cap. The
> precise picture is the two distinct limits above (32 at parse time, 10 by
> default at run time); `SAFETY_MAX_DEPTH = 100` in
> `crates/velesdb-core/src/collection/graph/traversal.rs` is only
> the cap applied by the imperative `with_unbounded_range` traversal builder, not
> by the VelesQL MATCH path.

### `LET` before `MATCH`

`LET x = 0.5 MATCH (a)-[:R]->(b) RETURN b` **parses** — the grammar allows
`let_clause*` before any statement — but is **rejected at execution**. MATCH runs
on a dedicated traversal path that bypasses LET evaluation, so the engine returns
an explicit error rather than silently discarding the binding:

```
Error: LET bindings are not supported with MATCH queries in this version
```

Rank graph rows with `RETURN ... ORDER BY` instead. `LET` bindings are supported
only on `SELECT` queries (dense `NEAR`, text `MATCH '...'`, hybrid, scalar-filter);
see the LET Clause section of `docs/VELESQL_SPEC.md` for the full list of
unsupported shapes.

### Hybrid fusion entry points

Two distinct fusion paths exist, with different reach:

- The **everyday vector + BM25 hybrid** (`hybrid_search`) uses **weighted RRF**:
  reciprocal-rank fusion with a `vector_weight`/`text_weight` split (default
  0.5/0.5), constant `k = 60`, and 0-based ranks. This is what a plain
  `MATCH ... NEAR ...` hybrid query uses.
- The **richer `fusion::FusionStrategy`** (RRF, RSF, weighted, average, max) is
  currently reachable only through dense+sparse fusion and `NEAR_FUSED`. RSF /
  weighted / avg / max are **not** selectable for the plain vector + BM25 hybrid;
  use `NEAR_FUSED` (or dense+sparse) to choose a non-RRF strategy.

## Related Documentation

- [VelesQL Specification](../VELESQL_SPEC.md) -- Full VelesQL language reference
- [Search Modes Guide](SEARCH_MODES.md) -- Configuring recall vs latency
- [Agent Memory Guide](AGENT_MEMORY.md) -- AI agent memory subsystems
