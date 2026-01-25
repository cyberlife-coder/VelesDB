# Multi-Model Queries Guide

VelesDB supports **multi-model queries** that combine vector similarity, graph traversal, and structured data in a single unified query language: **VelesQL**.

## Quick Start

### Python
```python
import velesdb

db = velesdb.Database("./data")
collection = db.get_collection("documents")

# Multi-model query: vector similarity + graph context
results = collection.query(
    "MATCH (d:Doc) WHERE vector NEAR $q LIMIT 20",
    params={"q": query_embedding}
)

for r in results:
    print(f"Node: {r['node_id']}, Score: {r['fused_score']:.3f}")
```

### TypeScript
```typescript
import { VelesDB } from 'velesdb-client';

const db = new VelesDB({ baseUrl: 'http://localhost:8080' });
await db.init();

const response = await db.query('documents', 
    'MATCH (d:Doc) WHERE vector NEAR $q LIMIT 20',
    { q: queryVector }
);

response.results.forEach(r => {
    console.log(`Node: ${r.nodeId}, Score: ${r.fusedScore.toFixed(3)}`);
});
```

### Rust
```rust
use velesdb_core::{Database, velesql::Parser};

let db = Database::open("./data")?;
let collection = db.get_collection("documents").unwrap();

let query = Parser::parse("MATCH (d:Doc) WHERE vector NEAR $q LIMIT 20")?;
let params = [("q".to_string(), serde_json::json!(query_vector))].into();

let results = collection.execute_query(&query, &params)?;
for r in results {
    println!("Node: {}, Score: {:.3}", r.point.id, r.score);
}
```

### Tauri (Desktop)
```typescript
import { invoke } from '@tauri-apps/api/tauri';

const results = await invoke('plugin:velesdb|query', {
    query: 'MATCH (d:Doc) WHERE vector NEAR $q LIMIT 20',
    params: { q: queryVector }
});
```

## Result Format

All SDKs return results in a unified **HybridResult** format:

| Field | Type | Description |
|-------|------|-------------|
| `nodeId` / `node_id` | u64 | Point/node identifier |
| `vectorScore` / `vector_score` | f32? | Vector similarity score |
| `graphScore` / `graph_score` | f32? | Graph relevance score |
| `fusedScore` / `fused_score` | f32 | Combined score |
| `bindings` | object? | Matched node properties |
| `columnData` / `column_data` | object? | JOIN data (future) |

## VelesQL Syntax

### Vector Similarity
```sql
-- Basic vector search
SELECT * FROM collection WHERE vector NEAR $query LIMIT 10

-- With threshold
SELECT * FROM collection WHERE similarity(vector, $query) > 0.8 LIMIT 10
```

### Graph Traversal (MATCH)
```sql
-- Find connected nodes
MATCH (doc:Document)-[:REFERENCES]->(ref:Document)
WHERE vector NEAR $query
LIMIT 20
```

### ORDER BY Expressions
```sql
-- Custom scoring formula
SELECT * FROM docs 
WHERE vector NEAR $q 
ORDER BY 0.7 * vector_score + 0.3 * graph_score DESC
LIMIT 10
```

## Fusion Strategies

When combining vector and graph results:

| Strategy | Use Case |
|----------|----------|
| `RRF` (default) | Balanced, works well in most cases |
| `average` | Equal weight to all signals |
| `maximum` | Prefer strongest signal |
| `weighted` | Custom weights per signal |

## Best Practices

1. **Always set LIMIT** - Unbounded queries are expensive
2. **Use indexes** - Create property indexes for filtered fields
3. **Batch queries** - Use multi-query search for multiple vectors
4. **Monitor timing** - Check `timing_ms` in responses

## SDK Availability

| SDK | Multi-Model Query | Status |
|-----|-------------------|--------|
| Python | `collection.query()` | ✅ |
| TypeScript | `db.query()` | ✅ |
| WASM | `store.query()` | ✅ |
| Tauri | `invoke('plugin:velesdb\|query')` | ✅ |
| REST API | `POST /collections/{name}/query` | ✅ |

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        VelesQL Parser                           │
│   "MATCH (d:Doc) JOIN prices ON ... WHERE vector NEAR $v"       │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Filter Pushdown Analysis                     │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────┐         │
│  │ Graph       │  │ ColumnStore  │  │ Post-JOIN      │         │
│  │ Filters     │  │ Filters      │  │ Filters        │         │
│  └─────────────┘  └──────────────┘  └────────────────┘         │
└─────────────────────────────────────────────────────────────────┘
          │                  │                   │
          ▼                  ▼                   │
┌─────────────────┐  ┌─────────────────┐        │
│   Graph Engine  │  │ ColumnStore     │        │
│ ┌─────────────┐ │  │ ┌─────────────┐ │        │
│ │ HNSW Vector │ │  │ │ B-Tree      │ │        │
│ │ Index       │ │  │ │ Indexes     │ │        │
│ └─────────────┘ │  │ └─────────────┘ │        │
│ ┌─────────────┐ │  │ ┌─────────────┐ │        │
│ │ Graph       │ │  │ │ Column      │ │        │
│ │ Traversal   │ │  │ │ Storage     │ │        │
│ └─────────────┘ │  │ └─────────────┘ │        │
└─────────────────┘  └─────────────────┘        │
          │                  │                   │
          └────────┬─────────┘                   │
                   ▼                             │
┌─────────────────────────────────────────────────────────────────┐
│                    JOIN Executor (Batch Adaptive)               │
│   Combines Graph results with ColumnStore data                  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Score Evaluator                              │
│   ORDER BY 0.7 * vector_score + 0.3 * graph_score DESC         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    HybridResult[]                               │
│   { nodeId, vectorScore, graphScore, fusedScore, bindings }    │
└─────────────────────────────────────────────────────────────────┘
```

## Troubleshooting

### Common Errors

| Error | Cause | Solution |
|-------|-------|----------|
| `Parameter '$v' not provided` | Missing query vector | Add vector to params: `params={"v": [...]}` |
| `Collection not found` | Wrong collection name | Check collection exists with `db.list_collections()` |
| `Unknown column` | Invalid field in WHERE | Verify field exists in payload |
| `Dimension mismatch` | Vector size ≠ collection dimension | Check embedding dimension |

### Performance Tips

1. **Always set LIMIT** - Unbounded queries scan entire collection
2. **Filter early** - High-selectivity filters reduce candidates
3. **Use indexes** - Create property indexes for filtered fields
4. **Batch queries** - Use multi-query fusion for multiple vectors
5. **Monitor timing** - Check `timing_ms` in response

## See Also

- [VelesQL Specification](../reference/VELESQL_SPEC.md)
- [JOIN Reference](../reference/VELESQL_JOIN.md)
- [ORDER BY Reference](../reference/VELESQL_ORDERBY.md)
- [Search Modes](./SEARCH_MODES.md)
- [Benchmarks](../BENCHMARKS.md)
