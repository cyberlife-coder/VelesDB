# @wiscale/velesdb-sdk

Official TypeScript SDK for VelesDB - Vector Search in Microseconds.

## Installation

```bash
npm install @wiscale/velesdb-sdk
```

## Quick Start

### WASM Backend (Browser/Node.js)

```typescript
import { VelesDB } from '@wiscale/velesdb-sdk';

// Initialize with WASM backend
const db = new VelesDB({ backend: 'wasm' });
await db.init();

// Create a collection
await db.createCollection('documents', {
  dimension: 768,  // BERT embedding dimension
  metric: 'cosine'
});

// Insert vectors
await db.insert('documents', {
  id: 'doc-1',
  vector: new Float32Array(768).fill(0.1),
  payload: { title: 'Hello World', category: 'greeting' }
});

// Batch insert
await db.insertBatch('documents', [
  { id: 'doc-2', vector: [...], payload: { title: 'Second doc' } },
  { id: 'doc-3', vector: [...], payload: { title: 'Third doc' } },
]);

// Search
const results = await db.search('documents', queryVector, { k: 5 });
console.log(results);
// [{ id: 'doc-1', score: 0.95, payload: { title: '...' } }, ...]

// Cleanup
await db.close();
```

### REST Backend (Server)

```typescript
import { VelesDB } from '@wiscale/velesdb-sdk';

const db = new VelesDB({
  backend: 'rest',
  url: 'http://localhost:8080',
  apiKey: 'your-api-key' // optional
});

await db.init();

// Same API as WASM backend
await db.createCollection('products', { dimension: 1536 });
await db.insert('products', { id: 'p1', vector: [...] });
const results = await db.search('products', query, { k: 10 });
```

## API Reference

### `new VelesDB(config)`

Create a new VelesDB client.

| Option | Type | Required | Description |
|--------|------|----------|-------------|
| `backend` | `'wasm' \| 'rest'` | Yes | Backend type |
| `url` | `string` | REST only | Server URL |
| `apiKey` | `string` | No | API key for authentication |
| `timeout` | `number` | No | Request timeout (ms, default: 30000) |

### `db.init()`

Initialize the client. Must be called before any operations.

### `db.createCollection(name, config)`

Create a new collection.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `dimension` | `number` | Required | Vector dimension |
| `metric` | `'cosine' \| 'euclidean' \| 'dot' \| 'hamming' \| 'jaccard'` | `'cosine'` | Distance metric |
| `storageMode` | `'full' \| 'sq8' \| 'binary'` | `'full'` | Memory optimization mode |

#### Storage Modes

| Mode | Memory (768D) | Compression | Use Case |
|------|---------------|-------------|----------|
| `full` | 3 KB/vector | 1x | Default, max precision |
| `sq8` | 776 B/vector | **4x** | Scale, RAM-constrained |
| `binary` | 96 B/vector | **32x** | Edge, IoT |

```typescript
// Memory-optimized collection
await db.createCollection('embeddings', {
  dimension: 768,
  metric: 'cosine',
  storageMode: 'sq8'  // 4x memory reduction
});
```

### `db.createMetadataCollection(name)` (v1.4.0+)

Create a metadata-only collection (no vectors). Useful for reference data that can be JOINed with vector collections.

```typescript
await db.createMetadataCollection('products');
await db.insert('products', { id: 'P001', vector: [], payload: { name: 'Widget', price: 99 } });

// JOIN with vector collection
const result = await db.query('orders', `
  SELECT * FROM orders JOIN products AS p ON orders.product_id = p.id
`);
```

### `db.insert(collection, document)`

Insert a single vector.

```typescript
await db.insert('docs', {
  id: 'unique-id',
  vector: [0.1, 0.2, ...],  // or Float32Array
  payload: { key: 'value' } // optional metadata
});
```

### `db.insertBatch(collection, documents)`

Insert multiple vectors efficiently.

### `db.search(collection, query, options)`

Search for similar vectors.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `k` | `number` | `10` | Number of results |
| `filter` | `object` | - | Filter expression |
| `includeVectors` | `boolean` | `false` | Include vectors in results |
| `efSearch` | `number` | - | HNSW ef_search parameter (higher = better recall, slower) |
| `mode` | `'fast' \| 'balanced' \| 'accurate' \| 'perfect'` | - | Search mode preset (sets efSearch automatically) |
| `timeoutMs` | `number` | - | Server-side query timeout in milliseconds |

#### Search Modes

| Mode | ef_search | Use Case |
|------|-----------|----------|
| `fast` | 64 | Low-latency, acceptable recall |
| `balanced` | 128 | Default tradeoff |
| `accurate` | 256 | High recall |
| `perfect` | max | Maximum recall, slowest |

### `db.searchBatch(collection, searches)` (v1.4.0+)

Search for multiple vectors in parallel.

```typescript
const results = await db.searchBatch('docs', [
  { vector: queryVec1, k: 5 },
  { vector: queryVec2, k: 10, filter: { category: 'tech' } },
]);
// results[0] = results for queryVec1, results[1] = results for queryVec2
```

### `db.delete(collection, id)`

Delete a vector by ID. Returns `true` if deleted.

### `db.get(collection, id)`

Get a vector by ID. Returns `null` if not found.

### `db.textSearch(collection, query, options)` (v0.8.5+)

Full-text search using BM25 algorithm.

```typescript
const results = await db.textSearch('docs', 'machine learning', { k: 10 });
```

### `db.hybridSearch(collection, vector, textQuery, options)` (v0.8.5+)

Combined vector + text search with RRF fusion.

```typescript
const results = await db.hybridSearch(
  'docs',
  queryVector,
  'machine learning',
  { k: 10, vectorWeight: 0.7 }  // 0.7 = 70% vector, 30% text
);
```

### `db.query(collection, queryString, params?, options?)` (v0.8.5+)

Execute a VelesQL query.

```typescript
// Simple query
const results = await db.query(
  'documents',
  "SELECT * FROM documents WHERE category = 'tech' LIMIT 10"
);

// With vector parameter
const results = await db.query(
  'documents',
  "SELECT * FROM documents WHERE VECTOR NEAR $query LIMIT 5",
  { query: [0.1, 0.2, ...] }
);

// Hybrid query
const results = await db.query(
  'docs',
  "SELECT * FROM docs WHERE VECTOR NEAR $v AND content MATCH 'rust' LIMIT 10",
  { v: queryVector }
);
```

### `db.multiQuerySearch(collection, vectors, options)` (v1.1.0+) ⭐ NEW

Multi-query fusion search for RAG pipelines using Multiple Query Generation (MQG).

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `k` | `number` | `10` | Number of results |
| `fusion` | `'rrf' \| 'average' \| 'maximum' \| 'weighted'` | `'rrf'` | Fusion strategy |
| `fusionParams` | `object` | `{ k: 60 }` | Strategy-specific parameters |
| `filter` | `object` | - | Filter expression |

```typescript
// RRF fusion (default) - best for most RAG use cases
const results = await db.multiQuerySearch('docs', [emb1, emb2, emb3], {
  k: 10,
  fusion: 'rrf',
  fusionParams: { k: 60 }
});

// Weighted fusion - like SearchXP scoring
const results = await db.multiQuerySearch('docs', [emb1, emb2], {
  k: 10,
  fusion: 'weighted',
  fusionParams: { avgWeight: 0.6, maxWeight: 0.3, hitWeight: 0.1 }
});

// Average/Maximum fusion
const results = await db.multiQuerySearch('docs', vectors, {
  k: 10,
  fusion: 'average'  // or 'maximum'
});
```

> **Note:** Multi-query fusion is only available with the REST backend.

### `db.isEmpty(collection)` (v0.8.11+)

Check if a collection is empty.

```typescript
const empty = await db.isEmpty('documents');
if (empty) {
  console.log('No vectors in collection');
}
```

### `db.flush(collection)` (v0.8.11+)

Flush pending changes to disk.

```typescript
await db.flush('documents');
```

### `db.close()`

Close the client and release resources.

## Knowledge Graph API (v1.2.0+)

VelesDB supports hybrid vector + graph queries.

### `db.addEdge(collection, edge)`

```typescript
await db.addEdge('social', {
  id: 1, source: 100, target: 200,
  label: 'FOLLOWS',
  properties: { since: '2024-01-01' }
});
```

### `db.getEdges(collection, options?)`

```typescript
const edges = await db.getEdges('social', { label: 'FOLLOWS' });
```

### `db.traverseGraph(collection, request)`

```typescript
const result = await db.traverseGraph('social', {
  source: 100, strategy: 'bfs', maxDepth: 3
});
```

### `db.streamTraverseGraph(collection, options, callbacks)` (v1.5.0+)

Stream graph traversal results via Server-Sent Events. Nodes are delivered as they are discovered, ideal for large graphs or real-time UIs.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `source` | `number` | Required | Source node ID |
| `strategy` | `'bfs' \| 'dfs'` | `'bfs'` | Traversal algorithm |
| `maxDepth` | `number` | `5` | Maximum depth |
| `limit` | `number` | `1000` | Max results |
| `relTypes` | `string[]` | all | Filter by relationship types |

```typescript
await db.streamTraverseGraph('social', { source: 1, strategy: 'bfs', maxDepth: 3 }, {
  onNode: (node) => console.log(`Node ${node.id} at depth ${node.depth}`),
  onStats: (stats) => console.log(`Visited ${stats.nodesVisited} nodes`),
  onDone: (done) => console.log(`Complete: ${done.totalNodes} nodes in ${done.elapsedMs}ms`),
  onError: (err) => console.error(err),
});
```

### `db.getNodeDegree(collection, nodeId)`

```typescript
const degree = await db.getNodeDegree('social', 100);
console.log(`In: ${degree.inDegree}, Out: ${degree.outDegree}`);
```

### `db.matchQuery(collection, query, params?, options?)` (v1.4.0+)

Execute a MATCH graph traversal query using Cypher-like pattern matching.

| Option | Type | Description |
|--------|------|-------------|
| `vector` | `number[] \| Float32Array` | Optional vector for similarity matching |
| `threshold` | `number` | Similarity threshold |

```typescript
const result = await db.matchQuery('docs',
  'MATCH (a:Person)-[:KNOWS]->(b) WHERE similarity(a.embedding, $v) > 0.8 RETURN a.name, b.name',
  { v: queryVector },
  { vector: queryVector, threshold: 0.8 }
);

for (const r of result.results) {
  console.log(`${r.projected['a.name']} knows ${r.projected['b.name']} (score: ${r.score})`);
}
```

## VelesQL v2.0 Queries (v1.4.0+)

Execute advanced SQL-like queries with aggregation, joins, and set operations.

### Aggregation with GROUP BY / HAVING

```typescript
// Group by with aggregates
const result = await db.query('products', `
  SELECT category, COUNT(*), AVG(price) 
  FROM products 
  GROUP BY category 
  HAVING COUNT(*) > 5 AND AVG(price) > 50
`);

// Access results
for (const row of result.results) {
  console.log(row.payload.category, row.payload.count);
}
```

### ORDER BY with similarity()

```typescript
// Order by semantic similarity
const result = await db.query('docs', `
  SELECT * FROM docs 
  ORDER BY similarity(vector, $v) DESC 
  LIMIT 10
`, { v: queryVector });
```

### JOIN across collections

```typescript
// Cross-collection join
const result = await db.query('orders', `
  SELECT * FROM orders 
  JOIN customers AS c ON orders.customer_id = c.id 
  WHERE status = $status
`, { status: 'active' });
```

### Set Operations (UNION / INTERSECT / EXCEPT)

```typescript
// Combine query results
const result = await db.query('users', `
  SELECT * FROM active_users 
  UNION 
  SELECT * FROM archived_users
`);

// Find common elements
const result = await db.query('users', `
  SELECT id FROM premium_users 
  INTERSECT 
  SELECT id FROM active_users
`);
```

### Hybrid Search with USING FUSION

```typescript
// RRF fusion (default)
const result = await db.query('docs', `
  SELECT * FROM docs 
  USING FUSION(strategy = 'rrf', k = 60) 
  LIMIT 20
`);

// Weighted fusion
const result = await db.query('docs', `
  SELECT * FROM docs 
  USING FUSION(strategy = 'weighted', vector_weight = 0.7, graph_weight = 0.3) 
  LIMIT 20
`);
```

### `db.explain(query, params?)` (v1.4.0+)

Analyze a VelesQL query without executing it. Returns the query plan, cost estimation, and feature detection.

```typescript
const plan = await db.explain(
  'SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8 LIMIT 10'
);

console.log(plan.queryType);                // 'SELECT'
console.log(plan.estimatedCost.complexity);  // 'O(log n)'
console.log(plan.features.hasVectorSearch);  // true
console.log(plan.features.hasFilter);        // false

// Inspect plan steps
for (const step of plan.plan) {
  console.log(`Step ${step.step}: ${step.operation} — ${step.description}`);
}
```

## MATCH Query Builder (v1.2.0+)

Build type-safe VelesQL MATCH queries with the fluent builder API.

```typescript
import { velesql } from '@wiscale/velesdb-sdk';

// Graph pattern query
const builder = velesql()
  .match('d', 'Document')
  .nearVector('$queryVector', queryVector)
  .andWhere('d.category = $cat', { cat: 'tech' })
  .limit(10);

const queryString = builder.toVelesQL();
const params = builder.getParams();
const results = await db.query('documents', queryString, params);

// Graph traversal with relationships
const graphQuery = velesql()
  .match('p', 'Person')
  .rel('KNOWS')
  .to('f', 'Person')
  .where('p.age > 25')
  .return(['p.name', 'f.name'])
  .toVelesQL();
```

## SELECT Query Builder (v1.4.0+)

Build type-safe VelesQL SELECT queries with the fluent builder API.

```typescript
import { selectql } from '@wiscale/velesdb-sdk';

// Vector search with filters
const { query, params } = selectql()
  .select('id', 'title', 'category')
  .from('documents')
  .similarity('embedding', 'v', queryVector, { threshold: 0.7 })
  .andWhere('category = $cat', { cat: 'tech' })
  .orderBy('title', 'ASC')
  .limit(20)
  .build();

const results = await db.query('documents', query, params);

// Aggregation query
const { query: aggQuery, params: aggParams } = selectql()
  .selectAgg('COUNT', '*', 'total')
  .selectAgg('AVG', 'price', 'avg_price')
  .select('category')
  .from('products')
  .where('price > $min', { min: 10 })
  .groupBy('category')
  .orderBy('total', 'DESC')
  .limit(10)
  .build();

// JOIN query
const { query: joinQuery } = selectql()
  .selectAll()
  .from('orders')
  .join('customers', 'orders.customer_id = customers.id', 'LEFT')
  .where('status = $s', { s: 'active' })
  .build();
```

## Error Handling

```typescript
import { VelesDBError, ValidationError, ConnectionError, NotFoundError } from '@wiscale/velesdb-sdk';

try {
  await db.search('nonexistent', query);
} catch (error) {
  if (error instanceof NotFoundError) {
    console.log('Collection not found');
  } else if (error instanceof ValidationError) {
    console.log('Invalid input:', error.message);
  } else if (error instanceof ConnectionError) {
    console.log('Connection failed:', error.message);
  }
}
```

## Performance Tips

1. **Use batch operations** for multiple inserts
2. **Reuse Float32Array** for queries when possible
3. **Use WASM backend** for browser apps (no network latency)
4. **Pre-initialize** the client at app startup

## License

MIT License - See [LICENSE](./LICENSE) for details.

VelesDB Core is licensed under ELv2 (source available).
