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

### `db.query(queryString, params)` (v0.8.5+)

Execute a VelesQL query.

```typescript
// Simple query
const results = await db.query(
  "SELECT * FROM documents WHERE category = 'tech' LIMIT 10"
);

// With vector parameter
const results = await db.query(
  "SELECT * FROM documents WHERE VECTOR NEAR $query LIMIT 5",
  { query: [0.1, 0.2, ...] }
);

// Hybrid query
const results = await db.query(
  "SELECT * FROM docs WHERE VECTOR NEAR $v AND content MATCH 'rust' LIMIT 10",
  { v: queryVector }
);
```

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
