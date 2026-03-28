# @wiscale/velesdb-sdk

Official TypeScript SDK for [VelesDB](https://github.com/cyberlife-coder/VelesDB) -- the local-first vector database for AI and RAG. Sub-millisecond semantic search in Browser and Node.js.

**v1.9.2** | Node.js >= 18 | Browser (WASM) | MIT License

## What's New in v1.9.2

- **SearchQuality type** -- new `SearchQuality` type and `quality` field in `SearchOptions` for per-query recall/latency control (`fast`, `balanced`, `accurate`, `perfect`, `custom:N`, `adaptive:min:max`)
- **StorageMode in HnswParams** -- `storageMode` field in HNSW configuration
- **Relative score fusion** -- `'relative_score'` fusion strategy in `MultiQuerySearchOptions`
- **DistanceMetric "ip" alias** -- `"ip"` accepted as an alias for `"dot"` (inner product)
- **StorageMode aliases** -- `"f32"`, `"int8"`, `"bit"` accepted as aliases for `"full"`, `"sq8"`, `"binary"`

### Previous (v1.9.1)

- **Agent Memory API** -- semantic, episodic, and procedural memory for AI agents (REST only)
- **Graph collections** -- dedicated `createGraphCollection()` for knowledge graphs (REST only)
- **Metadata-only collections** -- reference tables with no vectors, joinable via VelesQL
- **Sparse vector support** -- hybrid sparse+dense search on insert and query (REST + WASM)
- **Stream insert with backpressure** -- `streamInsert()` for high-throughput ingestion (REST only)
- **Product Quantization training** -- `trainPq()` for further memory compression (REST only)
- **Collection analytics** -- `analyzeCollection()`, `getCollectionStats()`, `getCollectionConfig()` (REST only)
- **Property indexes** -- `createIndex()` / `listIndexes()` / `dropIndex()` for O(1) lookups (REST only)
- **Query introspection** -- `queryExplain()` and `collectionSanity()` diagnostics (REST only)
- **Batch search** -- `searchBatch()` for parallel multi-query execution
- **Lightweight search** -- `searchIds()` returns only IDs and scores (REST only)

## Installation

```bash
npm install @wiscale/velesdb-sdk
```

## Quick Start

### WASM Backend (Browser / Node.js)

The WASM backend runs entirely in-process -- no server required. Ideal for browser apps, prototyping, and edge deployments.

```typescript
import { VelesDB } from '@wiscale/velesdb-sdk';

// 1. Create a client with WASM backend
const db = new VelesDB({ backend: 'wasm' });
await db.init();

// 2. Create a collection (768 dimensions for BERT, 1536 for OpenAI, etc.)
await db.createCollection('documents', {
  dimension: 768,
  metric: 'cosine'
});

// 3. Insert vectors with metadata
await db.insert('documents', {
  id: 'doc-1',
  vector: new Float32Array(768).fill(0.1),
  payload: { title: 'Hello World', category: 'greeting' }
});

// 4. Batch insert for better throughput
await db.insertBatch('documents', [
  { id: 'doc-2', vector: new Float32Array(768).fill(0.2), payload: { title: 'Second doc' } },
  { id: 'doc-3', vector: new Float32Array(768).fill(0.3), payload: { title: 'Third doc' } },
]);

// 5. Search for similar vectors
const results = await db.search('documents', queryVector, { k: 5 });
console.log(results);
// [{ id: 'doc-1', score: 0.95, payload: { title: 'Hello World', ... } }, ...]

// 6. Cleanup
await db.close();
```

### REST Backend (Server)

The REST backend connects to a running VelesDB server. Use this for production deployments, multi-client access, and persistent storage.

```typescript
import { VelesDB } from '@wiscale/velesdb-sdk';

const db = new VelesDB({
  backend: 'rest',
  url: 'http://localhost:8080',
  apiKey: 'your-api-key'  // optional
});

await db.init();

// Same API as WASM backend
await db.createCollection('products', { dimension: 1536 });
await db.insert('products', { id: 1, vector: embedding });
const results = await db.search('products', queryVector, { k: 10 });
```

> **REST backend note:** Document IDs must be numeric integers in the range `0..Number.MAX_SAFE_INTEGER`. String IDs are only supported with the WASM backend.

## API Reference

### Client

#### `new VelesDB(config)`

Create a new VelesDB client.

| Option | Type | Required | Description |
|--------|------|----------|-------------|
| `backend` | `'wasm' \| 'rest'` | Yes | Backend type |
| `url` | `string` | REST only | Server URL |
| `apiKey` | `string` | No | API key for authentication |
| `timeout` | `number` | No | Request timeout in ms (default: 30000) |

#### `db.init()`

Initialize the client. **Must be called before any operations.** For the REST backend, this verifies connectivity to the server.

#### `db.close()`

Close the client and release resources.

---

### Collection Management

#### `db.createCollection(name, config)`

Create a vector collection.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `dimension` | `number` | Required | Vector dimension |
| `metric` | `'cosine' \| 'euclidean' \| 'dot' \| 'hamming' \| 'jaccard'` | `'cosine'` | Distance metric (aliases: `'ip'`/`'inner'`/`'dotproduct'` for dot) |
| `storageMode` | `'full' \| 'sq8' \| 'binary'` | `'full'` | Quantization mode (aliases: `'f32'` for full, `'int8'` for sq8, `'bit'` for binary) |
| `hnsw` | `{ m?: number, efConstruction?: number }` | - | HNSW index tuning |
| `description` | `string` | - | Optional description |

##### Storage Modes

| Mode | Memory (768D) | Compression | Use Case |
|------|---------------|-------------|----------|
| `full` | 3 KB/vector | 1x | Default, max precision |
| `sq8` | 776 B/vector | **4x** | Production scale, RAM-constrained |
| `binary` | 96 B/vector | **32x** | Edge devices, IoT |

```typescript
await db.createCollection('embeddings', {
  dimension: 768,
  metric: 'cosine',
  storageMode: 'sq8',
  hnsw: { m: 16, efConstruction: 200 }
});
```

#### `db.createGraphCollection(name, config?)`

Create a dedicated graph collection for knowledge graph workloads.

```typescript
await db.createGraphCollection('social', {
  dimension: 384,       // optional: embed nodes for vector+graph queries
  metric: 'cosine',
  schemaMode: 'schemaless'  // or 'strict'
});
```

#### `db.createMetadataCollection(name)`

Create a metadata-only collection (no vectors). Useful for reference tables that can be JOINed with vector collections via VelesQL.

```typescript
await db.createMetadataCollection('products');
```

#### `db.deleteCollection(name)`

Delete a collection and all its data.

#### `db.getCollection(name)`

Get collection info. Returns `null` if not found.

#### `db.listCollections()`

List all collections. Returns an array of `Collection` objects.

---

### Insert and Retrieve

#### `db.insert(collection, document)`

Insert a single vector document.

```typescript
await db.insert('docs', {
  id: 'unique-id',
  vector: [0.1, 0.2, 0.3],    // number[] or Float32Array
  payload: { key: 'value' },   // optional metadata
  sparseVector: { 42: 0.8, 99: 0.3 }  // optional sparse vector for hybrid search
});
```

#### `db.insertBatch(collection, documents)`

Insert multiple vectors in a single call. More efficient than repeated `insert()`.

```typescript
await db.insertBatch('docs', [
  { id: 'a', vector: vecA, payload: { title: 'First' } },
  { id: 'b', vector: vecB, payload: { title: 'Second' } },
]);
```

#### `db.streamInsert(collection, documents)`

Insert documents with server backpressure support. Sends documents sequentially, respecting 429 rate limits. Throws `BackpressureError` if the server pushes back.

```typescript
await db.streamInsert('docs', largeDocumentArray);
```

#### `db.get(collection, id)`

Get a document by ID. Returns `null` if not found.

#### `db.delete(collection, id)`

Delete a document by ID. Returns `true` if deleted, `false` if not found.

---

### Search

#### `db.search(collection, query, options?)`

Vector similarity search.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `k` | `number` | `10` | Number of results |
| `filter` | `object` | - | Payload filter expression |
| `includeVectors` | `boolean` | `false` | Include vectors in results |
| `sparseVector` | `Record<number, number>` | - | Sparse vector for hybrid sparse+dense search |
| `quality` | `SearchQuality` | - | Search quality mode (e.g., `'fast'`, `'balanced'`, `'custom:256'`, `'adaptive:32:512'`) |

```typescript
const results = await db.search('docs', queryVector, {
  k: 10,
  filter: { category: 'tech' },
  includeVectors: true
});
// Returns: SearchResult[] = [{ id, score, payload?, vector? }, ...]
```

#### `db.searchBatch(collection, searches)`

Execute multiple search queries in parallel.

```typescript
const batchResults = await db.searchBatch('docs', [
  { vector: queryA, k: 5 },
  { vector: queryB, k: 10, filter: { type: 'article' } },
]);
// Returns: SearchResult[][] (one result array per query)
```

#### `db.searchIds(collection, query, options?)`

Lightweight search returning only IDs and scores (no payloads).

```typescript
const hits = await db.searchIds('docs', queryVector, { k: 100 });
// Returns: Array<{ id: number, score: number }>
```

#### `db.textSearch(collection, query, options?)`

Full-text search using BM25 scoring.

```typescript
const results = await db.textSearch('docs', 'machine learning', { k: 10 });
```

#### `db.hybridSearch(collection, vector, textQuery, options?)`

Combined vector similarity + BM25 text search with RRF fusion.

```typescript
const results = await db.hybridSearch(
  'docs',
  queryVector,
  'machine learning',
  { k: 10, vectorWeight: 0.7 }  // 70% vector, 30% text
);
```

#### `db.multiQuerySearch(collection, vectors, options?)`

Multi-query fusion search for RAG pipelines using Multiple Query Generation (MQG). Combines results from several query vectors into a single ranked list.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `k` | `number` | `10` | Number of results |
| `fusion` | `'rrf' \| 'average' \| 'maximum' \| 'weighted'` | `'rrf'` | Fusion strategy |
| `fusionParams` | `object` | `{ k: 60 }` | Strategy-specific parameters |
| `filter` | `object` | - | Payload filter expression |

```typescript
// RRF fusion (default) -- best for most RAG use cases
const results = await db.multiQuerySearch('docs', [emb1, emb2, emb3], {
  k: 10,
  fusion: 'rrf',
  fusionParams: { k: 60 }
});

// Weighted fusion
const results = await db.multiQuerySearch('docs', [emb1, emb2], {
  k: 10,
  fusion: 'weighted',
  fusionParams: { avgWeight: 0.6, maxWeight: 0.3, hitWeight: 0.1 }
});
```

> **Note:** WASM supports `rrf`, `average`, `maximum`. The `weighted` strategy is REST-only.

---

### Collection Utilities

#### `db.isEmpty(collection)`

Returns `true` if the collection contains no vectors.

#### `db.flush(collection)`

Flush pending changes to disk. **REST backend only** -- the WASM backend runs in-memory and this is a no-op.

#### `db.analyzeCollection(collection)`

Compute and return collection statistics.

```typescript
const stats = await db.analyzeCollection('docs');
console.log(stats.totalPoints, stats.totalSizeBytes);
```

#### `db.getCollectionStats(collection)`

Get previously computed statistics. Returns `null` if the collection has not been analyzed yet.

#### `db.getCollectionConfig(collection)`

Get detailed collection configuration (dimension, metric, storage mode, point count, schema).

---

### VelesQL Queries

#### `db.query(collection, queryString, params?, options?)`

Execute a VelesQL query. Supports SELECT, WHERE, vector NEAR, GROUP BY, HAVING, ORDER BY, JOIN, UNION/INTERSECT/EXCEPT, and USING FUSION.

```typescript
// Vector similarity search
const result = await db.query(
  'documents',
  'SELECT * FROM documents WHERE VECTOR NEAR $query LIMIT 5',
  { query: [0.1, 0.2, 0.3] }
);

// Aggregation
const agg = await db.query(
  'products',
  `SELECT category, COUNT(*), AVG(price)
   FROM products
   GROUP BY category
   HAVING COUNT(*) > 5`
);

// Hybrid vector + text
const hybrid = await db.query(
  'docs',
  "SELECT * FROM docs WHERE VECTOR NEAR $v AND content MATCH 'rust' LIMIT 10",
  { v: queryVector }
);

// Cross-collection JOIN
const joined = await db.query(
  'orders',
  `SELECT * FROM orders
   JOIN customers AS c ON orders.customer_id = c.id
   WHERE status = $status`,
  { status: 'active' }
);

// Set operations
const combined = await db.query('users',
  'SELECT * FROM active_users UNION SELECT * FROM archived_users'
);

// Fusion strategy
const fused = await db.query('docs',
  "SELECT * FROM docs USING FUSION(strategy = 'rrf', k = 60) LIMIT 20"
);
```

Query options:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `timeoutMs` | `number` | `30000` | Query timeout in milliseconds |
| `stream` | `boolean` | `false` | Enable streaming response |

#### `db.queryExplain(queryString, params?)`

Get the execution plan for a VelesQL query without running it. Returns plan steps, estimated cost, index usage, and detected features.

```typescript
const plan = await db.queryExplain(
  'SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 10',
  { v: queryVector }
);
console.log(plan.plan);           // step-by-step execution plan
console.log(plan.estimatedCost);  // { usesIndex, selectivity, complexity }
console.log(plan.features);       // { hasVectorSearch, hasFilter, hasJoin, ... }
```

#### `db.collectionSanity(collection)`

Run diagnostic checks on a collection (dimensions, search readiness, error counts, hints).

```typescript
const report = await db.collectionSanity('docs');
console.log(report.checks);       // { hasVectors, searchReady, dimensionConfigured }
console.log(report.diagnostics);   // { searchRequestsTotal, dimensionMismatchTotal, ... }
console.log(report.hints);         // actionable suggestions
```

---

### VelesQL Query Builder

Build type-safe VelesQL queries with a fluent API instead of raw strings.

```typescript
import { velesql } from '@wiscale/velesdb-sdk';

// Vector similarity with filters
const builder = velesql()
  .match('d', 'Document')
  .nearVector('$queryVector', queryVector)
  .andWhere('d.category = $cat', { cat: 'tech' })
  .orderBy('similarity()', 'DESC')
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
// => "MATCH (p:Person)-[:KNOWS]->(f:Person) WHERE p.age > 25 RETURN p.name, f.name"
```

Builder methods: `match()`, `rel()`, `to()`, `where()`, `andWhere()`, `orWhere()`, `nearVector()`, `limit()`, `offset()`, `orderBy()`, `return()`, `returnAll()`, `fusion()`.

---

### Knowledge Graph API

#### `db.addEdge(collection, edge)`

Add a directed edge between two nodes.

```typescript
await db.addEdge('social', {
  id: 1,
  source: 100,
  target: 200,
  label: 'FOLLOWS',
  properties: { since: '2024-01-01' }
});
```

#### `db.getEdges(collection, options?)`

Query edges, optionally filtered by label.

```typescript
const edges = await db.getEdges('social', { label: 'FOLLOWS' });
```

#### `db.traverseGraph(collection, request)`

Traverse the graph using BFS or DFS from a source node.

```typescript
const result = await db.traverseGraph('social', {
  source: 100,
  strategy: 'bfs',
  maxDepth: 3,
  limit: 100,
  relTypes: ['FOLLOWS', 'KNOWS']
});

for (const node of result.results) {
  console.log(`Node ${node.targetId} at depth ${node.depth}`);
}
```

#### `db.getNodeDegree(collection, nodeId)`

Get the in-degree and out-degree of a node.

```typescript
const degree = await db.getNodeDegree('social', 100);
console.log(`In: ${degree.inDegree}, Out: ${degree.outDegree}`);
```

---

### Property Indexes

Create secondary indexes for fast lookups on payload fields.

#### `db.createIndex(collection, options)`

```typescript
// Hash index for O(1) equality lookups
await db.createIndex('users', { label: 'Person', property: 'email' });

// Range index for O(log n) range queries
await db.createIndex('events', {
  label: 'Event',
  property: 'timestamp',
  indexType: 'range'
});
```

#### `db.listIndexes(collection)`

```typescript
const indexes = await db.listIndexes('users');
// [{ label, property, indexType, cardinality, memoryBytes }, ...]
```

#### `db.hasIndex(collection, label, property)`

Returns `true` if the specified index exists.

#### `db.dropIndex(collection, label, property)`

Drop an index. Returns `true` if the index existed and was removed.

---

### Product Quantization

#### `db.trainPq(collection, options?)`

Train Product Quantization on a collection for further memory compression beyond SQ8. **REST backend only** -- delegates to velesdb-server; not available in the WASM backend.

```typescript
const result = await db.trainPq('embeddings', {
  m: 8,       // number of subquantizers
  k: 256,     // centroids per subquantizer
  opq: true   // enable Optimized PQ
});
```

---

### Agent Memory API

The Agent Memory API provides three memory types for AI agents, built on top of VelesDB's vector and graph storage.

```typescript
import { VelesDB } from '@wiscale/velesdb-sdk';

const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
await db.init();

const memory = db.agentMemory({ dimension: 384 });
```

#### Semantic Memory (facts and knowledge)

```typescript
// Store a fact
await memory.storeFact('knowledge', {
  id: 1,
  text: 'VelesDB uses HNSW for vector indexing',
  embedding: factEmbedding,
  metadata: { source: 'docs', confidence: 0.95 }
});

// Recall similar facts
const facts = await memory.searchFacts('knowledge', queryEmbedding, 5);
```

#### Episodic Memory (events and experiences)

```typescript
// Record an event
await memory.recordEvent('events', {
  eventType: 'user_query',
  data: { query: 'How does HNSW work?', response: '...' },
  embedding: eventEmbedding,
  metadata: { timestamp: Date.now() }
});

// Recall similar events
const events = await memory.recallEvents('events', queryEmbedding, 5);
```

#### Procedural Memory (learned patterns)

```typescript
// Store a procedure
await memory.learnProcedure('procedures', {
  name: 'deploy-to-prod',
  steps: ['Run tests', 'Build artifacts', 'Push to registry', 'Deploy'],
  metadata: { lastUsed: Date.now() }
});

// Find matching procedures
const procs = await memory.recallProcedures('procedures', queryEmbedding, 3);
```

---

## Error Handling

All error classes extend `VelesDBError` and include a `code` property for programmatic handling.

```typescript
import {
  VelesDBError,
  ValidationError,
  ConnectionError,
  NotFoundError,
  BackpressureError
} from '@wiscale/velesdb-sdk';

try {
  await db.search('nonexistent', queryVector);
} catch (error) {
  if (error instanceof NotFoundError) {
    console.log('Collection not found');
  } else if (error instanceof ValidationError) {
    console.log('Invalid input:', error.message);
  } else if (error instanceof ConnectionError) {
    console.log('Server unreachable:', error.message);
  } else if (error instanceof BackpressureError) {
    console.log('Server overloaded, retry later');
  }
}
```

## Exports

Everything is importable from the package root:

```typescript
import {
  // Client
  VelesDB,
  AgentMemoryClient,

  // Backends (advanced: use VelesDB client instead)
  WasmBackend,
  RestBackend,

  // Query builder
  VelesQLBuilder,
  velesql,
  type RelDirection,
  type RelOptions,
  type NearVectorOptions,
  type FusionOptions,

  // Error classes
  VelesDBError,
  ValidationError,
  ConnectionError,
  NotFoundError,
  BackpressureError,

  // Types (selected)
  type VelesDBConfig,
  type CollectionConfig,
  type VectorDocument,
  type SearchOptions,
  type SearchQuality,
  type SearchResult,
  type SparseVector,
  type MultiQuerySearchOptions,
  type GraphEdge,
  type AddEdgeRequest,
  type TraverseRequest,
  type TraverseResponse,
  type QueryApiResponse,
  type AgentMemoryConfig,
  type SemanticEntry,
  type EpisodicEvent,
  type ProceduralPattern,
} from '@wiscale/velesdb-sdk';
```

## Performance Tips

1. **Use `insertBatch()`** instead of repeated `insert()` calls
2. **Reuse `Float32Array`** buffers for query vectors when possible
3. **Use WASM backend** for browser apps (zero network latency)
4. **Use `searchIds()`** when you only need IDs and scores (skips payload transfer)
5. **Use `streamInsert()`** for high-throughput ingestion with backpressure handling
6. **Pre-initialize** the client at app startup (`await db.init()`)
7. **Tune HNSW** with `hnsw: { m: 16, efConstruction: 200 }` for higher recall

## License

MIT License -- See [LICENSE](./LICENSE) for details.

VelesDB Core and Server are licensed under VelesDB Core License 1.0 (source-available).
