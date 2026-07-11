# @wiscale/velesdb-sdk

Official TypeScript SDK for [VelesDB](https://github.com/cyberlife-coder/VelesDB) -- the local-first vector database for AI and RAG. Sub-millisecond semantic search in Browser and Node.js.

**v3.9.0** | Node.js >= 18 | Browser (WASM) | VelesDB Core License 1.0

## What's New in v3.6.0

- **Memory wedge, running in-browser via WASM**: new `MemoryService` class (`remember`/`recall`/`recallWhere`/`recallFused`/`relate`/`forget`/`why`) — the same local-first agent memory as `@wiscale/velesdb-memory-node` and the Python binding, now reachable without a server. In-memory only in this release (no filesystem access under WASM); see [Memory Wedge](#memory-wedge-agent-memory) below. Requires `@wiscale/velesdb-wasm` >= 3.6.0 — fresh installs resolve it automatically; on an upgrade, refresh the dependency in your lockfile (`init()` reports the exact cause otherwise).

## What's New in v3.0.0

- **Streaming ingestion enablement (2026-06-14)** (REST backend): `enableStreaming(collection, config?)` turns on the bounded streaming-ingestion channel before `streamInsert()`. The optional `StreamingConfig` (`bufferSize`, `batchSize`, `flushIntervalMs`) is camelCase; omitted fields fall back to the server defaults. See [`db.enableStreaming`](#dbenablestreamingcollection-config) below. The WASM backend throws `NOT_SUPPORTED`.
- **Relation + durable-TTL surface** (REST backend): `relate()`, `unrelate()`, `getRelations()`, `setTtlDurable()` — now fully tested and documented (see [Knowledge Graph API](#knowledge-graph-api) and [Agent Memory API](#agent-memory-api) below). The WASM backend throws `NOT_SUPPORTED` for these methods.
- The shipped example (`examples/hybrid_queries.ts`) was rewritten against the real API and is now compile-checked in CI.

## What's New in v2.0.0

- v2.0.0: graph dimension on agent memory — `relate()` / `relations()` / `unrelate()`; durable-TTL setters; aligns with the engine 2.0 release. See the root [CHANGELOG](../../CHANGELOG.md) for the breaking VelesQL changes.
- v1.18.0: agent-memory parity wave — temporal recall facades (`recallRecent` / `recallOlderThan`), id-coercion hardening for `deleteMemory(string | number)`.

## What's New in v1.16.0

- **First-party embedding helper.** New `OpenAIEmbedder` (plus the `Embedder` interface and `OpenAIEmbedderOptions` type), exported from the package root. It calls any OpenAI-compatible `/embeddings` endpoint via the global `fetch` API — no extra runtime dependency — so you can go from text to vectors without hand-writing the request. See [Embedding helper](#embedding-helper) below. Works in Node.js ≥ 18, browsers, and Deno.

### Previous (v1.14.2)

- **No SDK source change.** v1.14.2 was a workspace patch focused on the Python Haystack `DocumentStore` (`DuplicatePolicy.SKIP` contract fix) and seven version-drift gaps in the release tooling. The TS SDK ships in lock-step with the workspace and was functionally identical to v1.14.1.

### Previous (v1.14.1)

- **Pipeline fix only.** v1.14.0 added Haystack 2.x DocumentStore source code on the Python side but the release workflow forgot to publish `haystack-velesdb` to PyPI. v1.14.1 closes that gap. No TS SDK source change.

### Previous (v1.14.0)

- **MSRV Rust 1.89** -- workspace and CI now align with the actual SIMD path (`avx512vpopcntdq` target feature). No source change for the SDK; bumps in lock-step with the workspace.
- **Dockerfile auto-sync** -- release tooling now keeps `LABEL version=` in lock-step across all Dockerfiles. Indirectly improves anyone running `docker build` against a checkout.

### Previous (v1.13.7)

- **Node.js WASM init fix** -- `new VelesDB({ backend: 'wasm' }).init()` now reads `velesdb_wasm_bg.wasm` bytes from disk via `fs.readFile` so Node 20+ no longer crashes on the broken `fetch('file://')` path. Browsers are unchanged.
- **Lifecycle hardening** -- memoised in-flight init promise + generation token make `close()` race-free.
- **Dual ESM + CJS bundles** -- TS SDK build emits both formats with `import.meta.url`/`__filename` polyfilled correctly.

### Previous (v1.13.0)

- **WASM VelesQL executor** -- full browser-side VelesQL execution: SELECT/INSERT/UPDATE/DELETE/DDL + aggregations (COUNT/SUM/AVG/MIN/MAX) + GROUP BY/HAVING/UNION/INTERSECT/EXCEPT/JOIN/FUSION/MATCH 1-2 hops + NOT De Morgan distribution
- **TS SDK coverage raised** -- per-file thresholds codified in `vitest.config.ts` (423 tests as of v1.13.0; the vitest suite has since grown past 770 cases). Note: the suite runs locally via `npm test` and is not currently executed in CI
- **SIFT1M standardized ANN benchmark** -- fvecs/ivecs loader + Criterion ef sweep on the INRIA TEXMEX dataset, feature-gated behind `--features bench-sift1m`
- **Security hardening** -- `validateCollectionName()` helper on TS SDK prevents VelesQL injection in `trainPq`
- **API consistency** -- `streamInsert` now serializes `payload: null` explicitly (matches `streamUpsertPoints`)

### Previous (v1.12.0)

- **Cross-collection MATCH queries** -- `@collection` annotation on MATCH node patterns enables cross-collection graph queries
- **MATCH via `/query` endpoint** -- MATCH queries can now be executed via `Database::execute_query`
- **BFS dedup** -- CSR and EdgeStore BFS no longer produce duplicate results for diamond graphs
- **`rrf_k` propagation** -- now properly propagated to `hybrid_search_with_filter`
- **`ComponentScores` optimization** -- changed to `&'static str` for zero-allocation score tagging

### Previous (v1.11.1)

- **Graph API parity** -- 7 new REST endpoints for complete graph operations (delete edge, edge count, list nodes, node edges, node payload, parallel BFS, graph search)
- **Bitmap pre-filter** -- adaptive strategy selection for filtered search
- **CSR graph traversal v2** -- lock-free adjacency with edge IDs and labels
- **Bulk insert v2** -- DirectVectorWriter + AsyncIndexBuilder pipeline

### Previous (v1.11.0)

- **15 new VelesQL statements** -- SHOW COLLECTIONS, DESCRIBE, EXPLAIN, CREATE/DROP INDEX, ANALYZE, TRUNCATE, ALTER COLLECTION, FLUSH, multi-row INSERT, UPSERT, SELECT EDGES, INSERT NODE
- **203 BDD E2E tests** -- comprehensive end-to-end test coverage for all VelesQL features
- **TRUNCATE on graph collections** -- clears nodes + edges in a single statement
- **Python `execute_query()`** -- full VelesQL execution from Python bindings
- **Cyclomatic complexity ≤ 8** -- refactored 6 hotspots for Codacy compliance

### Previous (v1.10.0)

- **SearchQuality type** -- `SearchQuality` type and `quality` field in `SearchOptions`
- **StorageMode in HnswParams** -- `storageMode` field in HNSW configuration
- **Relative score fusion** -- `'relative_score'` fusion strategy
- **DistanceMetric "ip" alias** -- `"ip"` accepted as alias for `"dot"`
- **StorageMode aliases** -- `"f32"`, `"int8"`, `"bit"` accepted

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

### Supported Node.js versions

| Node.js | TS SDK | Browser (WASM) | Notes |
|---------|--------|----------------|-------|
| 18 LTS  | ✅      | ✅              | minimum supported |
| 20 LTS  | ✅ (CI) | ✅              | tested matrix on every PR |
| 22 LTS  | ✅      | ✅              | latest LTS |

The Node.js WASM `init()` path was fixed in v1.13.7 to read `velesdb_wasm_bg.wasm` bytes from disk via `fs.readFile` instead of the broken `fetch('file://')` URL — Node ≥ 18 now works out-of-the-box. Browsers were never affected.

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

// 3. Upsert vectors with metadata
await db.upsert('documents', {
  id: 'doc-1',
  vector: new Float32Array(768).fill(0.1),
  payload: { title: 'Hello World', category: 'greeting' }
});

// 4. Batch upsert for better throughput
await db.upsertBatch('documents', [
  { id: 'doc-2', vector: new Float32Array(768).fill(0.2), payload: { title: 'Second doc' } },
  { id: 'doc-3', vector: new Float32Array(768).fill(0.3), payload: { title: 'Third doc' } },
]);

// 5. Search for similar vectors
const queryVector = new Float32Array(768).fill(0.1);
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
await db.upsert('products', { id: 1, vector: embedding });
const queryVector = new Float32Array(1536).fill(0.1);
const results = await db.search('products', queryVector, { k: 10 });
```

> **REST backend note:** Document IDs must be non-negative `u64` integers. Pass a JS
> number in the range `0..Number.MAX_SAFE_INTEGER`, or a decimal string for the full
> `u64` range — string ids above 2^53-1 (up to `18446744073709551615`) are kept
> verbatim on the wire, so the ids returned by `recordEvent`/`learnProcedure`
> round-trip through `get`/`delete` without precision loss. Exception: the NDJSON
> bulk endpoint (`streamUpsertPoints`) only accepts safe-range numeric ids.
> Arbitrary (non-numeric) string IDs are only supported with the WASM backend.

> **Versioned routes:** The REST backend uses `/v1/` as the canonical route prefix
> (e.g. `POST /v1/collections/{name}/search`). Legacy routes without the prefix
> are accepted for backward compatibility but are deprecated and will be removed
> in a future major version. Always target `/v1/` in custom HTTP clients.

## Embedding helper

The SDK ships an optional `OpenAIEmbedder` so you can turn text into vectors without
writing the HTTP call yourself. It targets any OpenAI-compatible `/embeddings`
endpoint (OpenAI, Azure OpenAI, vLLM, …) using the global `fetch` API, so it adds
**no extra runtime dependency** and runs in Node.js ≥ 18, browsers, and Deno.

```typescript
import { VelesDB, OpenAIEmbedder } from '@wiscale/velesdb-sdk';

const embedder = new OpenAIEmbedder({
  apiKey: process.env.OPENAI_API_KEY!,
  model: 'text-embedding-3-small', // default; pass `dimensions` to truncate
});

const db = new VelesDB({ backend: 'wasm' });
await db.init();

// Embed first so the collection dimension matches the model output.
const vectors = await embedder.embed(['hello world', 'vector search']);
await db.createCollection('docs', { dimension: embedder.dimension, metric: 'cosine' });
await db.upsertBatch('docs', vectors.map((vector, i) => ({ id: `doc-${i}`, vector })));

const [query] = await embedder.embed(['greeting']);
const results = await db.search('docs', query, { k: 5 });
```

`embedder.dimension` is `0` until the first `embed()` call (or until you pass
`dimensions` to the constructor), at which point it is inferred from the model's
output. To use a different provider, set `baseUrl`. Implement the `Embedder`
interface (`{ dimension: number; embed(texts: string[]): Promise<number[][]> }`)
to plug in any other embedding source.

## Memory Wedge (Agent Memory)

Local-first **agent memory** — the same wedge as `@wiscale/velesdb-memory-node`
and the Python binding, running entirely in-process via WebAssembly (browser
or Node.js, no server). `remember` / `recall` / `recallWhere` / `recallFused`
/ `relate` / `forget` / `why`. The differentiator is **`why()`**: it answers a
question with the best-matching memory *plus its connected subgraph* —
related facts a plain vector recall is blind to.

```typescript
import { MemoryService } from '@wiscale/velesdb-sdk';

const memory = new MemoryService({ dimension: 384 });
await memory.init();

const pr = await memory.remember('PR #42 swaps the mutex for parking_lot');
const decision = await memory.remember(
  'we chose parking_lot to avoid lock poisoning',
  { links: [{ target: pr, relation: 'decided_in' }] }
);

// recall: vector similarity.
const hits = await memory.recall('lock poisoning', 5);

// recallFused: also walks the graph from the top vector hit and promotes
// any fact it reaches — the tri-engine ranking measured on HotpotQA/TimeQA/LoCoMo.
const fused = await memory.recallFused('lock poisoning', 5);

// recallWhere: fused vector + structured filters (ranges/comparisons).
const recent = await memory.recallWhere('release notes', [
  { field: 'ts', op: 'ge', value: 20260101 },
]);

// why: the wedge — seed memory + its reachable subgraph.
const { nodes, edges } = await memory.why('why parking_lot');
```

Every method returns a `Promise`. Memory ids cross the WASM boundary as
**decimal strings** (a JS `number` loses precision above 2^53). Errors thrown
across the boundary are translated into the SDK's typed hierarchy —
`NotFoundError`, `ValidationError`, or the base `VelesDBError` — so you can
`catch (e) { if (e instanceof NotFoundError) ... }` the same way regardless
of which backend raised it.

> **In-memory only in this release.** Unlike the Node/Python bindings, the
> WASM store has no filesystem access, so `MemoryService` does not persist to
> disk — memory is lost when the page/process ends. Durable browser storage
> (IndexedDB) and `rememberExtracted` (auto-extraction, which needs a
> model/network call) are both out of scope for this release; track
> `@wiscale/velesdb-memory-node` or the Python binding if you need a
> persistent store today.

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

### Upsert and Retrieve

#### `db.upsert(collection, document)`

Upsert (insert or replace) a single vector document.

```typescript
await db.upsert('docs', {
  id: 'unique-id',
  vector: [0.1, 0.2, 0.3],    // number[] or Float32Array
  payload: { key: 'value' },   // optional metadata
  sparseVector: { 42: 0.8, 99: 0.3 }  // optional sparse vector for hybrid search
});
```

#### `db.upsertBatch(collection, documents)`

Upsert multiple vectors in a single call. More efficient than repeated `upsert()`.

```typescript
await db.upsertBatch('docs', [
  { id: 'a', vector: vecA, payload: { title: 'First' } },
  { id: 'b', vector: vecB, payload: { title: 'Second' } },
]);
```

#### `db.enableStreaming(collection, config?)`

Enable the bounded streaming-ingestion channel on a collection. Call this once before `streamInsert()`. The optional `config` is camelCase; every field is optional and omitted fields fall back to the server defaults (REST only; the WASM backend throws `NOT_SUPPORTED`).

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `bufferSize` | `number` | `10000` | Bounded ingestion channel capacity |
| `batchSize` | `number` | `128` | Points flushed to the index per batch |
| `flushIntervalMs` | `number` | `50` | Max milliseconds before a partial batch is flushed |

```typescript
await db.enableStreaming('docs', { bufferSize: 4096, batchSize: 64 });
await db.streamInsert('docs', largeDocumentArray);
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
  filter: { condition: { type: 'eq', field: 'category', value: 'tech' } },
  includeVectors: true
});
// Returns: SearchResult[] = [{ id, score, payload?, vector? }, ...]
```

#### `db.searchBatch(collection, searches)`

Execute multiple search queries in parallel.

```typescript
const batchResults = await db.searchBatch('docs', [
  { vector: queryA, k: 5 },
  { vector: queryB, k: 10, filter: { condition: { type: 'eq', field: 'type', value: 'article' } } },
]);
// Returns: SearchResult[][] (one result array per query)
```

#### `db.searchIds(collection, query, options?)`

Lightweight search returning only IDs and scores (no payloads).

```typescript
const hits = await db.searchIds('docs', queryVector, { k: 100 });
// Returns: Array<{ id: number, score: number }>
```

#### `db.scroll(collection, request)`

Iterate over all points in a collection in stable, paginated batches without a
query vector. Useful for export pipelines, re-embedding, and full-collection
inspection. Pagination is cursor-based — pass the `nextCursor` from each
`ScrollResponse` until it is `null`.

```typescript
import { ScrollRequest, ScrollResponse } from '@wiscale/velesdb-sdk';

let cursor: string | number | null = null;
let hasMore = true;
while (hasMore) {
  const req: ScrollRequest = { batchSize: 100 };
  if (cursor !== null) {
    req.cursor = cursor;
  }

  const page: ScrollResponse = await db.scroll('docs', req);

  for (const point of page.points) {
    console.log(point.id, point.payload);
  }
  cursor = page.nextCursor;
  hasMore = cursor !== null;
}
```

| Field (`ScrollRequest`) | Type | Default | Description |
|-------------------------|------|---------|-------------|
| `cursor` | `string \| number` | - | Cursor from previous `ScrollResponse.nextCursor` |
| `batchSize` | `number` | `100` | Points per page |
| `filter` | `object` | - | Optional payload filter expression |

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
| `fusion` | `'rrf' \| 'average' \| 'maximum' \| 'weighted' \| 'relative_score'` | `'rrf'` | Fusion strategy |
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

// Relative Score Fusion — linear blend of dense and sparse scores
const results = await db.multiQuerySearch('docs', [emb1, emb2], {
  k: 10,
  fusion: 'relative_score',
  fusionParams: { denseWeight: 0.7, sparseWeight: 0.3 }
});
```

> **Note:** WASM supports `rrf`, `average`, `maximum`. The `weighted` and `relative_score` strategies are REST-only.

#### Named sparse indexes — `sparseIndexName` vs `sparseSearchNamed()`

A collection can carry multiple sparse indexes (e.g. `splade_v2`, `bm25_titles`). The TypeScript SDK exposes two distinct APIs depending on whether you want a dense + sparse hybrid query or a pure sparse query against a named index.

| API | Shape | Use when |
|---|---|---|
| `db.search(coll, denseVec, { sparseVector, sparseIndexName })` | dense + sparse hybrid against the named sparse index | You already have a dense embedding and want to combine it with a specific named sparse index in a single search call. The dense vector drives the primary candidate set; the named sparse index re-ranks. |
| `db.sparseSearchNamed(coll, sparseVec, indexName, options?)` | pure sparse against a named index | You have only a sparse vector (e.g. SPLADE expansion, lexical query) and want to query a specific named sparse index directly. No dense component. |

```typescript
// Hybrid: dense + sparse via the splade_v2 named index
const hybrid = await db.search('docs', denseEmbedding, {
  k: 10,
  sparseVector: { 42: 0.8, 99: 0.3 },
  sparseIndexName: 'splade_v2',
});

// Pure sparse against the same named index
const sparseOnly = await db.sparseSearchNamed(
  'docs',
  { 42: 0.8, 99: 0.3 },
  'splade_v2',
  { k: 10 },
);
```

When the collection has only one (default) sparse index, omit `sparseIndexName` on `db.search()`; the server picks the default. For named indexes, both APIs require the explicit name.

> **Note:** Both APIs are REST-only when a named index is required.
> The WASM backend has no concept of named sparse indexes — `sparseIndexName` is silently ignored on `db.search()` (the collection's single sparse index is used), and `db.sparseSearchNamed()` throws `wasmNotSupported`. Tracked as a follow-up to either surface a `wasmNotSupported` throw on `sparseIndexName` or implement named-sparse support in WASM.

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

> **Backend support:** full VelesQL execution requires the **REST backend** (`velesdb-server`).
> The WASM backend only executes pure top-k vector queries of the form
> `SELECT * FROM <collection> WHERE vector NEAR $param [LIMIT n]` (`vector` is the
> grammar keyword, not a column name) and throws a
> `NOT_SUPPORTED` error for anything else (WHERE predicates, JOIN, GROUP BY, MATCH,
> set operations, FUSION) instead of silently ignoring clauses. Accordingly,
> `db.capabilities().velesqlQuery` is `false` on WASM.

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
  "SELECT * FROM docs LIMIT 20 USING FUSION(strategy = 'rrf', k = 60)"
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

// Vector similarity with filters — use SELECT mode via from()
const builder = velesql()
  .from('documents', 'd')
  .nearVector('$queryVector', queryVector)
  .andWhere('d.category = $cat', { cat: 'tech' })
  .orderBy('similarity()', 'DESC')
  .limit(10);

const queryString = builder.toVelesQL();
// => "SELECT * FROM documents WHERE vector NEAR $queryVector AND d.category = $cat ORDER BY similarity() DESC LIMIT 10"
const params = builder.getParams();
const results = await db.query('documents', queryString, params);

// Graph traversal with relationships (MATCH mode — RETURN is mandatory
// and precedes ORDER BY / LIMIT)
const graphQuery = velesql()
  .match('p', 'Person')
  .rel('KNOWS')
  .to('f', 'Person')
  .where('p.age > 25')
  .return(['p.name', 'f.name'])
  .toVelesQL();
// => "MATCH (p:Person)-[:KNOWS]->(f:Person) WHERE p.age > 25 RETURN p.name, f.name"

// Hybrid (vector NEAR + text MATCH) with a real USING FUSION clause
const hybrid = velesql()
  .from('docs')
  .nearVector('$q', queryVector)
  .andWhere("content MATCH 'invoice'")
  .limit(10)
  .fusion('weighted', { vectorWeight: 0.7, graphWeight: 0.3 })
  .toVelesQL();
// => "SELECT * FROM docs WHERE vector NEAR $q AND content MATCH 'invoice' LIMIT 10 USING FUSION(strategy='weighted', vector_weight=0.7, graph_weight=0.3)"

// Multi-vector fused search — strategy is typed to rrf | average | maximum
const fused = velesql()
  .from('docs')
  .nearFused(['$a', '$b'], [vectorA, vectorB], { strategy: 'average' })
  .limit(10)
  .toVelesQL();
// => "SELECT * FROM docs WHERE vector NEAR_FUSED [$a, $b] USING FUSION 'average' LIMIT 10"
```

> **Mode matters.** Call `from(collection)` for vector / hybrid / fused
> SELECT queries; call `match(alias, label)` for graph patterns. MATCH mode
> always emits a `RETURN` (defaulting to `RETURN *`) and supports no
> `OFFSET`. `nearVector({ topK })` maps to `LIMIT` (VelesQL has no `TOP`
> keyword). `nearFused()` only accepts `rrf` / `average` / `maximum` —
> `weighted` / `relative_score` are a compile error because the engine
> would silently degrade them to RRF.

Builder methods: `match()`, `from()`, `select()`, `rel()`, `to()`, `where()`, `andWhere()`, `orWhere()`, `nearVector()`, `nearFused()`, `limit()`, `offset()`, `orderBy()`, `return()`, `returnAll()`, `fusion()`.

#### Collection settings — `db.setAutoReindex()` / `db.alterCollection()`

Toggle a collection's mutable settings at runtime via typed helpers that
emit `ALTER COLLECTION ... SET(...)`:

```typescript
await db.setAutoReindex('docs', true);             // auto_reindex=true
await db.alterCollection('docs', { autoReindex: false });
```

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

#### `db.traverseParallel(collection, request)`

Multi-source parallel BFS traversal with deduplication. Starts BFS from multiple source nodes simultaneously.

```typescript
const result = await db.traverseParallel('social', {
  sources: [100, 200, 300],
  maxDepth: 3,
  limit: 50,
  relTypes: ['FOLLOWS']
});

for (const node of result.results) {
  console.log(`Node ${node.targetId} at depth ${node.depth}`);
}
```

#### `db.relate(collection, request)`

Create a typed relation edge between two existing points. Returns the
allocated edge ID (`RelateResponse = { edgeId: number | string }`). Point and
edge IDs are `number | string` — IDs above `Number.MAX_SAFE_INTEGER` travel as
decimal strings to avoid u64 precision loss. **REST backend only** (the WASM
backend throws `NOT_SUPPORTED`).

```typescript
const { edgeId } = await db.relate('social', {
  source: 100,
  target: 200,
  relType: 'FOLLOWS',
  properties: { since: '2024-01-01' }  // optional, defaults to {}
});
```

#### `db.unrelate(collection, edgeId)`

Remove a relation edge by its ID. Returns `true` if the edge was removed,
`false` if it did not exist. **REST backend only.**

```typescript
const removed = await db.unrelate('social', edgeId);
```

#### `db.getRelations(collection, pointId)`

List the outgoing relation edges of a point. Returns
`RelationsResponse = { edges: RelationEdge[]; count: number }` where each edge
carries `{ id, source, target, relType, properties? }`. **REST backend only.**

```typescript
const { edges, count } = await db.getRelations('social', 100);
for (const edge of edges) {
  console.log(`${edge.source} -[${edge.relType}]-> ${edge.target}`);
}
```

#### `db.setTtlDurable(collection, pointId, ttlSeconds)`

Durably set (or refresh) the time-to-live of a point, in **seconds**. The
expiry is persisted server-side (reserved `_veles_expires_at` payload field),
so it survives a restart. `ttlSeconds` must be a non-negative number.
**REST backend only.**

```typescript
await db.setTtlDurable('social', 100, 3600); // expire point 100 in 1 hour
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

The Agent Memory API provides three memory types for AI agents, built on top of
VelesDB's vector storage. In the TypeScript/JavaScript SDK it is accessed **over
REST** against a running `velesdb-server` (the Python and Rust bindings use the
embedded engine instead).

> **You must create the collection first.** The TS facade does **not**
> auto-create a collection for you. Create it with the dimension that matches
> your embedding model and the metric you want (`'cosine'` is the usual choice
> for normalized text embeddings), then call `storeFact` / `recordEvent` /
> `learnProcedure` against that same collection name.

> **Embeddings are caller-supplied.** There is no auto-embedding: every
> `embedding` you pass must come from your own embedding model (see
> [Embedding helper](#embedding-helper)) and its length must equal the
> collection dimension.

```typescript
import { VelesDB } from '@wiscale/velesdb-sdk';

const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
await db.init();

// 1. Create the backing collection FIRST (nothing auto-creates it).
//    The dimension must match your embedding model; cosine is typical.
await db.createCollection('knowledge', { dimension: 384, metric: 'cosine' });

// 2. Open the agent-memory facade.
const memory = db.agentMemory({ dimension: 384 });

// 3. Store and recall (embedding is your own model's output, length 384).
await memory.storeFact('knowledge', {
  id: 1,
  text: 'VelesDB uses HNSW for vector indexing',
  embedding: factEmbedding,
});
const facts = await memory.searchFacts('knowledge', queryEmbedding, 5);
```

Each similarity-recall method (`searchFacts`, `recallEvents`,
`recallProcedures`) returns `SearchResult[]`:

```typescript
// SearchResult = { id: number; score: number; payload?: Record<string, unknown>; vector?: number[] }
```

The temporal-recall methods (`recallRecent`, `recallOlderThan`) return
`EpisodicRecord[]` instead — see [Episodic Memory](#episodic-memory-events-and-experiences).

- `score` is the cosine similarity in `[0, 1]` (for a `cosine` collection);
  higher means more similar.
- `payload` carries the stored fields (`content` for semantic text,
  `event_type` / `timestamp` for episodic, `name` / `steps` for procedural,
  plus your metadata). The reserved payload keys `_memory_type` / `content` /
  `event_type` / `timestamp` / `name` / `steps` always take precedence over
  caller `metadata`/`data` of the same name, so they cannot be clobbered.
  Note: the `SemanticEntry.text` input field is stored as `content` in the
  payload — `result.payload?.content` is the fact text on recall.

#### Semantic Memory (facts and knowledge)

```typescript
// Store a fact (id is caller-assigned; reusing an id upserts)
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
// Record an event — returns the generated point id
const eventId = await memory.recordEvent('events', {
  eventType: 'user_query',
  data: { query: 'How does HNSW work?', response: '...' },
  embedding: eventEmbedding,
});

// Recall similar events
const events = await memory.recallEvents('events', queryEmbedding, 5);

// Temporal recall — no embedding needed, most-recent-first.
// recallRecent(collection, since?): events with timestamp >= since
// (inclusive, unix-seconds); recallOlderThan(collection, before): strictly older.
const nowSecs = Math.floor(Date.now() / 1000);
const allRecent = await memory.recallRecent('events');
const lastHour = await memory.recallRecent('events', nowSecs - 3600);
const stale = await memory.recallOlderThan('events', nowSecs - 86_400);

// Both return EpisodicRecord[]:
// { id: string; timestamp: number; payload: Record<string, unknown> }
```

#### Procedural Memory (learned patterns)

```typescript
// Store a procedure — embedding is required so the pattern is recallable,
// and the generated point id is returned
const procId = await memory.learnProcedure('procedures', {
  name: 'deploy-to-prod',
  steps: ['Run tests', 'Build artifacts', 'Push to registry', 'Deploy'],
  embedding: procedureEmbedding,
  metadata: { lastUsed: Date.now() }
});

// Find matching procedures
const procs = await memory.recallProcedures('procedures', queryEmbedding, 3);
```

#### Deleting memories

```typescript
// Works for facts, events, and procedures — returns true if a point was removed
await memory.deleteMemory('procedures', procId);
```

> **`dimension` is advisory.** `db.agentMemory({ dimension })` records a hint you
> can read back via `memory.dimension`, but it does **not** create or size any
> collection — the collection's own dimension (set at `createCollection`)
> governs storage and search, and your embeddings must match it.

> **TTL & snapshots.** Durable per-point TTL (in **seconds**) **is** available
> from this SDK: memory entries are regular points, so
> [`db.setTtlDurable(collection, pointId, ttlSeconds)`](#dbsetttldurablecollection-pointid-ttlseconds)
> expires a fact/event/procedure durably over REST. The subsystem-namespaced
> TTL helpers (`set_semantic_ttl`, `store_with_ttl`, `auto_expire`) and
> versioned snapshots remain embedded-only (Rust **and** Python bindings) and
> are not exposed over REST. See the API-availability table in the
> [Agent Memory guide](../../docs/guides/AGENT_MEMORY.md#api-availability-by-binding).

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

  // Embedding helper
  OpenAIEmbedder,
  type Embedder,
  type OpenAIEmbedderOptions,

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
  type RelateRequest,
  type RelateResponse,
  type RelationsResponse,
  type QueryApiResponse,
  type AgentMemoryConfig,
  type SemanticEntry,
  type EpisodicEvent,
  type EpisodicRecord,
  type ProceduralPattern,
} from '@wiscale/velesdb-sdk';
```

## Performance Tips

1. **Use `upsertBatch()`** instead of repeated `upsert()` calls
2. **Reuse `Float32Array`** buffers for query vectors when possible
3. **Use WASM backend** for browser apps (zero network latency)
4. **Use `searchIds()`** when you only need IDs and scores (skips payload transfer)
5. **Use `streamInsert()`** for high-throughput ingestion with backpressure handling
6. **Pre-initialize** the client at app startup (`await db.init()`)
7. **Tune HNSW** with `hnsw: { m: 16, efConstruction: 200 }` for higher recall

## License

Licensed under the [VelesDB Core License 1.0](./LICENSE) (source-available). The SDK bundles the VelesDB WASM engine and is governed by the Core License.

VelesDB Core and Server are licensed under VelesDB Core License 1.0 (source-available).
