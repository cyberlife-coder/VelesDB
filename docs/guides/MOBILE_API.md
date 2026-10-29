# Mobile API guide (Swift / Kotlin)

The complete surface of the [`velesdb-mobile`](../../crates/velesdb-mobile/README.md)
UniFFI binding. Rust signatures are generated on
[docs.rs/velesdb-mobile](https://docs.rs/velesdb-mobile); this guide gives the
foreign-language names, the argument shapes that are not obvious from the type
system (JSON filters, VelesQL), and the behavioural notes.

Naming rule: every Swift/Kotlin name is the camelCase form of the Rust name
(`create_collection` → `createCollection`). Records keep their field names in
camelCase too (`properties_json` → `propertiesJson`).

To generate the bindings themselves, see the
[Mobile build guide](./MOBILE_BUILD.md).

## Threading

**No method in this binding is `async`** — the generated Swift contains zero `async`
functions, and the Kotlin methods are plain blocking calls. Every object is
`Send + Sync` on the Rust side and is exported as `@unchecked Sendable` (Swift), so a
handle can be shared across threads, but a search executed on the main thread blocks
the UI. Dispatch to a background queue (`DispatchQueue.global()`) or a coroutine
dispatcher (`Dispatchers.IO`).

## Quick start

### Swift (iOS)

```swift
import VelesDB  // the module name is the framework you packaged

// `open` is a named constructor: UniFFI emits a static method, not an init.
let db = try VelesDatabase.open(path: documentsPath + "/velesdb")

// 384 = all-MiniLM-L6-v2; 768 = MiniLM base.
try db.createCollection(name: "documents", dimension: 384, metric: .cosine)

guard let collection = try db.getCollection(name: "documents") else {
    fatalError("Collection not found")
}

let point = VelesPoint(
    id: 1,
    vector: embedding,                          // [Float] from your embedding model
    payload: "{\"title\": \"Hello World\"}"     // JSON string, or nil
)
try collection.upsert(point: point)

let results = try collection.search(vector: queryEmbedding, limit: 10)
for result in results {
    print("ID: \(result.id), Score: \(result.score)")
}
```

### Kotlin (Android)

```kotlin
import uniffi.velesdb_mobile.*   // package emitted by uniffi-bindgen

// `open` is a companion-object factory, not a constructor.
val db = VelesDatabase.open("${context.filesDir}/velesdb")

db.createCollection("documents", 384u, DistanceMetric.COSINE)

val collection = db.getCollection("documents")
    ?: throw IllegalStateException("Collection not found")

val point = VelesPoint(
    id = 1uL,
    vector = embedding,                         // List<Float> from your embedding model
    payload = """{"title": "Hello World"}"""    // JSON string, or null
)
collection.upsert(point)

val results = withContext(Dispatchers.IO) {
    collection.search(queryEmbedding, 10u)
}
results.forEach { result ->
    println("ID: ${result.id}, Score: ${result.score}")
}
```

## VelesDatabase

| Method | Description |
|--------|-------------|
| `VelesDatabase.open(path)` | Opens or creates a database at `path` (named constructor) |
| `VelesDatabase.openWithConfig(path, configPath)` | Opens with a TOML config file, engine sections only; fails fast on a missing/invalid file |
| `VelesDatabase.openWithConfigToml(path, configToml)` | Same, from an in-memory TOML string |
| `VelesDatabase.openWithObserver(path, observer)` | Opens with a read-path `MobileObserver` attached (audit **and deny**) |
| `VelesDatabase.openWithObserverAndConfig(path, observer, configPath)` | Observer + config file |
| `VelesDatabase.openWithObserverAndConfigToml(path, observer, configToml)` | Observer + config string |
| `updateGuardrails(limits)` | Live-updates query guardrail limits (`MobileQueryLimits`); throws for a `rateLimitQps` or `circuitFailureThreshold` of `0` |
| `createCollection(name, dimension, metric)` | Creates a vector collection |
| `createCollectionWithStorage(name, dimension, metric, storageMode)` | Creates a collection with quantized storage |
| `createMetadataCollection(name)` | Creates a metadata-only collection (no vectors) |
| `createGraphCollection(name)` | Creates a schemaless graph collection |
| `createGraphCollectionWithEmbeddings(name, dimension, metric)` | Graph collection whose nodes carry embeddings |
| `getCollection(name)` | Returns the collection, or nil/null when absent |
| `getGraphStore(name)` | Returns a graph collection's `MobileGraphStore`, nil/null when absent; throws for a non-graph collection |
| `listCollections()` | All collection names |
| `deleteCollection(name)` | Deletes a collection |
| `trainPq(collectionName, config)` | Trains Product Quantization (`PqTrainConfig`) |
| `executeQuery(query, params)` | Full VelesQL pass-through; returns a `QueryResult` |

Engine configuration semantics (which TOML sections are honoured, how `VELESDB_*`
env vars layer on top): [configuration guide](./CONFIGURATION.md).

## VelesCollection

### Search

| Method | Description |
|--------|-------------|
| `search(vector, limit)` | k nearest neighbours |
| `searchWithQuality(vector, limit, quality)` | Search with a `SearchQuality` preset |
| `searchWithFilter(vector, limit, filterJson)` | Search with a metadata filter |
| `batchSearch(searches)` | Batch of `IndividualSearchRequest`, each with its own filter; returns one result list per request |
| `textSearch(query, limit)` | BM25 full-text search |
| `textSearchWithFilter(query, limit, filterJson)` | Text search with filter |
| `hybridSearch(vector, textQuery, limit, vectorWeight)` | Vector + text fused search |
| `hybridSearchWithFilter(vector, textQuery, limit, vectorWeight, filterJson)` | Hybrid search with filter |
| `multiQuerySearch(vectors, limit, strategy)` | Multi-query fusion (MQG) |
| `multiQuerySearchIds(vectors, limit, strategy)` | Same, ID-only result path |
| `multiQuerySearchWithFilter(vectors, limit, strategy, filterJson)` | Multi-query fusion with filter |
| `sparseSearch(sparseVector, limit, indexName)` | Sparse-only search over an inverted index (`indexName` optional) |
| `hybridSparseSearch(vector, sparseVector, limit, indexName)` | Dense + sparse fused with RRF (k=60) |
| `query(queryStr, paramsJson)` | VelesQL query scoped to this collection; returns `SearchResult`s |

An empty `vectors` list on any multi-query method is an error
(`multi_query_search requires at least one vector`), never an empty result.

### Write

| Method | Description |
|--------|-------------|
| `upsert(point)` | Inserts or updates one point |
| `upsertBatch(points)` | Batch insert/update — the path to use for bulk loading |
| `upsertWithSparse(point, sparseVector)` | Inserts a point together with its sparse vector |
| `delete(id)` | Deletes a point |
| `enableStreaming(config)` | Enables streaming ingestion; defaults `bufferSize=10000`, `batchSize=128`, `flushIntervalMs=50`. Calling it again replaces the runtime |
| `streamInsert(points)` | Queues a batch on the streaming channel; returns the count queued. Fails when streaming was never enabled or the buffer is full |
| `flush()` | Flushes to durable storage |
| `compactStorage()` | Compacts the payload store; returns the bytes reclaimed |

### Read / introspection

| Method | Description |
|--------|-------------|
| `get(ids)` | Points by ID (missing IDs are skipped silently) |
| `getById(id)` | One point, or nil/null |
| `allIds()` | Every point ID in the collection |
| `count()` | Number of points |
| `dimension()` | Configured vector dimension |
| `isMetadataOnly()` | True for a metadata-only collection |
| `analyze()` | Runs ANALYZE, returns fresh `MobileCollectionStats` |
| `getStats()` | Latest statistics snapshot (no recomputation) |
| `diagnostics()` | `MobileCollectionDiagnostics`: readiness + index health |
| `guardRails()` | The guardrail limits currently in force for this collection |
| `applyAdvancedConfig(config)` | Post-creation overrides (`MobileAdvancedConfig`); `None` fields are left unchanged. A deferred indexer or async index builder the collection does not run yet takes effect at once; a change to one it runs takes effect at the next open |
| `pendingReopen()` | Streaming settings persisted by `applyAdvancedConfig` but not in force until the next open (`"deferred_indexing"`, `"async_index_builder"`) |

### Indexes

| Method | Description |
|--------|-------------|
| `createIndex(fieldName)` | Secondary metadata index |
| `hasSecondaryIndex(fieldName)` | Existence check |
| `createPropertyIndex(label, property)` | Graph property index |
| `createRangeIndex(label, property)` | Graph range index |
| `hasPropertyIndex(label, property)` / `hasRangeIndex(label, property)` | Existence checks |
| `listIndexes()` | All index definitions (`MobileIndexInfo`) |
| `dropIndex(label, property)` | Drops an index |
| `indexesMemoryUsage()` | Memory used by indexes, in bytes |

## The `filterJson` shape

Every `*WithFilter` method and `IndividualSearchRequest.filter` take a JSON **string**
using the canonical filter shape shared with the core engine and the REST API:

```json
{"condition": {"type": "<op>", "field": "...", "value": "..."}}
```

Depending on the operator the payload carries `value`, `values`, `pattern`, or a
nested `conditions` array. Operators: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `in`,
`contains`, `like`, `ilike`, `is_null`, `is_not_null`, `array_contains`,
`array_contains_any`, `array_contains_all`, `geo_distance`, `geo_bbox`, plus
`and` / `or` / `not` for composition.

Swift:

```swift
let results = try collection.searchWithFilter(
    vector: queryVector,
    limit: 5,
    filterJson: #"{"condition": {"type": "eq", "field": "category", "value": "tech"}}"#
)
```

Kotlin:

```kotlin
val results = collection.searchWithFilter(
    queryVector,
    5u,
    """{"condition": {"type": "eq", "field": "category", "value": "tech"}}"""
)
```

## VelesQL from mobile

`db.executeQuery(query, params)` is the full VelesQL pass-through (SELECT / NEAR /
MATCH, including cross-collection `@collection` annotations and aggregates). It
returns a `QueryResult`:

| Field | Type | Meaning |
|---|---|---|
| `kind` | `QueryResultKind` | `Rows`, `Mutation`, `Deletion`, `Ddl`, `Train`, `Admin`, mapped from core's `Query::result_kind` (so `SELECT EDGES` is `Rows`, as on WASM) |
| `rows` | `[QueryResultRow]` | Empty for DDL/TRAIN/FLUSH |
| `rowCount` | `UInt32` | Convenience count |
| `message` | `String` | Human-readable status, e.g. `"3 rows inserted"` |

Each `QueryResultRow` carries `id`, `score`, and `dataJson` — a JSON object string
merging `id`, `score`, and every payload field at the top level.

Query language reference: [multi-model queries](./MULTIMODEL_QUERIES.md) and
[graph patterns](./GRAPH_PATTERNS.md).

## VelesSemanticMemory

Agent memory for on-device AI: knowledge facts stored as vectors in a dedicated
`_semantic_memory` collection, created on first use.

| Method | Description |
|--------|-------------|
| `VelesSemanticMemory(db, dimension)` | Constructor; binds to the database and the embedding dimension |
| `store(id, content, embedding)` | Stores a fact; `content` is kept in the point payload |
| `query(embedding, topK)` | Similarity query, returns `SemanticResult` (id, score, content) |
| `delete(id)` | Deletes a fact |
| `remove(id)` | Deprecated alias for `delete` |
| `clear()` | Removes every fact |
| `len()` / `isEmpty()` | Size helpers |
| `dimension()` | Embedding dimension |

Scope note: mobile exposes **semantic memory only**. Episodic and procedural memory,
TTL setters, and snapshots — available in Python/WASM/MCP — are not bridged here
(see [ecosystem parity](../reference/ECOSYSTEM_PARITY.md)).

## MobileGraphStore

The knowledge graph of one graph collection, stored and traversed by core: nodes
are the collection's node payloads (the `label` is core's node label, so VelesQL
`MATCH (n:Label)` finds them through `executeQuery`), edges live in its edge
store, and BFS, DFS and multi-source BFS are core's `GraphCollection`
traversals. Writes are durable as core makes them; there is no save step.

Get one with `db.getGraphStore(name)` after `createGraphCollection(name)` (or
`createGraphCollectionWithEmbeddings` for nodes carrying a `vector`). Every
method throws except `removeEdge`; every read passes the read gate below as a
`GraphTraversal`, and fails closed on a scope filter.

| Method | Description |
|--------|-------------|
| `addNode(node)` | Adds or replaces a `MobileGraphNode`; `propertiesJson` must be a JSON object without the reserved `_labels` key |
| `addEdge(edge)` | Adds a `MobileGraphEdge`; throws on a duplicate edge ID, an empty label, or an endpoint node never added |
| `getNode(id)` / `getEdge(id)` | Lookup, nil/null when absent |
| `hasNode(id)` / `hasEdge(id)` | Existence checks |
| `nodeCount()` / `edgeCount()` | Sizes |
| `getOutgoing(nodeId)` / `getIncoming(nodeId)` | Incident edges |
| `getOutgoingByLabel(nodeId, label)` | Outgoing edges filtered by label |
| `getNeighbors(nodeId)` | 1-hop neighbour IDs |
| `getNodesByLabel(label)` / `getEdgesByLabel(label)` | Label scans |
| `outDegree(nodeId)` / `inDegree(nodeId)` | Degrees |
| `bfsTraverse(sourceId, maxDepth, limit)` | Core's breadth-first traversal |
| `bfsTraverseParallel(sourceIds, maxDepth, limit)` | Core's multi-source BFS, deduplicated by path |
| `dfsTraverse(sourceId, maxDepth, limit)` | Core's depth-first traversal |
| `removeNode(nodeId)` | Removes the node and every connected edge |
| `removeEdge(edgeId)` | Removes one edge; no-op when absent |
| `clear()` | Removes every node and edge |
| `flush()` | Flushes pending graph writes to disk |
| `importSnapshot(path)` | Migrates a JSON file written by the removed `save(path)` into this store |

Traversal semantics are core's (pinned by `traversals_are_cores` in
`src/graph_tests.rs` and by `tests/coverage_native.rs`): the source node is
never emitted, each `TraversalResult.path` lists the **edge IDs** taken from the
source, a single-source traversal emits each node once, and
`bfsTraverseParallel` emits a node once per path, so once per source that
reaches it.

Migrating from the in-memory store of earlier releases (`MobileGraphStore()`,
`save(path)`, `MobileGraphStore.load(path)`, all removed): create the graph
collection, get its store, and call `importSnapshot(path)` once on the old file.
It refuses, before writing, an edge ID the store already holds and an edge whose
endpoint is in neither the file nor the store; add that node, then import again.

## Read gate: MobileObserver

`openWithObserver` attaches a Swift/Kotlin callback consulted before **every** governed
read (dense / text / hybrid / sparse / multi-query search, VelesQL `SELECT` and
`MATCH`, and every `MobileGraphStore` read).

- `onQueryRequest(context) -> MobileAccessDecision` — return `Allow`, or
  `Deny { reason }` to abort the read with zero results.
- `MobileQueryContext`: `collection`, `operation`
  (`VectorSearch` / `TextSearch` / `HybridSearch` / `GraphTraversal` / `Select`),
  plus the opaque `principal` and `tenantHint` hints forwarded untouched — the gate
  never interprets them.
- Implementations **must not throw or panic**; express refusal with `Deny`.
- Core's scope-narrowing decision (`AllowWithScope`) is intentionally not bridged yet.

Guardrails are the other half of the control plane: `updateGuardrails(limits)` sets
`maxDepth`, `maxCardinality`, `memoryLimitBytes`, `timeoutMs` (0 disables),
`rateLimitQps`, `circuitFailureThreshold`, and `circuitRecoverySeconds` on every
collection — the rate limiter and circuit breaker included — and every collection
opened afterwards starts from them. They are not persisted. It throws
`VelesError` (`VELES-009`) and changes nothing for a `rateLimitQps` or a
`circuitFailureThreshold` of `0`, which would reject every query or open the
breaker on the first failure.

## Enums

### Distance metrics

| Metric | Description | Use case |
|--------|-------------|----------|
| `Cosine` | Cosine similarity (1 − cosine distance) | Text embeddings, normalized vectors |
| `Euclidean` | L2 distance | Image features, unnormalized vectors |
| `DotProduct` | Dot product | Pre-normalized vectors, MaxSim |
| `Hamming` | Hamming distance | Binary embeddings, LSH |
| `Jaccard` | Jaccard similarity | Sparse vectors, tag sets |

### Search quality

| Preset | `ef_search` | Note |
|---|---|---|
| `Fast` | 96 | 97.4% recall@10*, lowest latency |
| `Balanced` | 160 | 99.8% recall@10*, default |
| `Accurate` | 512 | 100% recall@10*; 0.98 on SIFT1M's 1M |
| `Perfect` | — (exhaustive scan, no graph) | Exact top-k, ties aside; refused above `max_perfect_mode_vectors` |
| `Custom { ef }` | caller-set, 16–4096 | Fine-grained control; outside the range, `searchWithQuality` throws `VelesError.Database` (#2275) |
| `Adaptive { minEf, maxEf }` | max(minEf, k), then once more at min(2×, maxEf) on a hard query | Escalates only hard queries; both bounds in 16–4096, or `searchWithQuality` throws |

\* Recall@10 in `recall_benchmark` (10K random 128-D vectors, an index built with `HnswParams::max_recall`, 100 queries), measured 2026-09-10 on 6.0.0; see [BENCHMARKS.md](../BENCHMARKS.md#hnsw-recall-profiles-10k128d).

### Storage modes

| Mode | Compression | Memory/dim | Use case |
|------|-------------|------------|----------|
| `Full` | 1x | 4 bytes | Best quality |
| `Sq8` | 4x | 1 byte | **Recommended for mobile** |
| `Binary` | 32x | 1 bit | Extreme constraints (IoT) |
| `ProductQuantization` | `2 × dim / m` (m subspaces) | `2 × m / dim` bytes, plus the codebook | Train first with `trainPq` |
| `Rabitq` | 32x | 1 bit | 1-bit plus rotation and scalar correction |

Compression follows from each mode's encoding, as the enum documents it in
`crates/velesdb-mobile/src/types.rs`. No recorded run measures these modes'
recall on mobile, so the table gives none.

```swift
// iOS — SQ8: 4x less memory, small recall loss
try db.createCollectionWithStorage(
    name: "embeddings",
    dimension: 384,
    metric: .cosine,
    storageMode: .sq8
)
```

```kotlin
// Android — binary quantization for IoT devices (32x compression)
db.createCollectionWithStorage(
    "embeddings", 384u, DistanceMetric.COSINE, StorageMode.BINARY
)
```

Compression trade-offs in depth: [quantization guide](./QUANTIZATION.md).

### Fusion strategies

Used by the `multiQuerySearch*` methods.

| Strategy | Description |
|----------|-------------|
| `Average` | Average score across queries |
| `Maximum` | Best score per document |
| `Rrf { k }` | Reciprocal Rank Fusion (core default k = 60) |
| `Weighted { avgWeight, maxWeight, hitWeight }` | Weighted mix of average, max, hit ratio |
| `RelativeScore { denseWeight, sparseWeight }` | Relative Score Fusion for dense + sparse |

## Records

| Type | Fields |
|------|--------|
| `VelesPoint` | `id: UInt64`, `vector: [Float]`, `payload: String?` (JSON string) |
| `SearchResult` | `id: UInt64`, `score: Float` |
| `SemanticResult` | `id: UInt64`, `score: Float`, `content: String` |
| `VelesSparseVector` | `indices: [UInt32]`, `values: [Float]` (parallel arrays) |
| `IndividualSearchRequest` | `vector: [Float]`, `topK: UInt32`, `filter: String?` |
| `PqTrainConfig` | `m: UInt32`, `k: UInt32`, `opq: Bool` |
| `MobileStreamingConfig` | `bufferSize`, `batchSize`, `flushIntervalMs` |
| `MobileQueryLimits` | `maxDepth`, `maxCardinality`, `memoryLimitBytes`, `timeoutMs`, `rateLimitQps`, `circuitFailureThreshold`, `circuitRecoverySeconds` |
| `MobileAdvancedConfig` | `pqRescoreOversampling?`, `deferredIndexing?`, `asyncIndexBuilder?` |
| `MobileGraphNode` | `id: UInt64`, `label: String`, `propertiesJson: String?`, `vector: [Float]?` |
| `MobileGraphEdge` | `id: UInt64`, `source: UInt64`, `target: UInt64`, `label: String`, `propertiesJson: String?` |
| `TraversalResult` | `nodeId: UInt64`, `path: [UInt64]` (edge IDs), `depth: UInt32` |
| `MobileCollectionStats` | `totalPoints`, `payloadSizeBytes`, `rowCount`, `deletedCount`, `avgRowSizeBytes`, `totalSizeBytes`, `fieldStatsCount`, `columnStatsCount`, `indexStatsCount` |
| `MobileCollectionDiagnostics` | `hasVectors`, `searchReady`, `dimensionConfigured`, `pointCount`, `indexHealth`, `indexHealthDetail?` |
| `MobileIndexInfo` | `label`, `property`, `indexType`, `cardinality`, `memoryBytes` |

The Swift memberwise initializers are explicit and take **every** field: pass
`payload: nil` rather than omitting it.

## Errors

`VelesError` (Kotlin: `VelesException`) has three variants:

| Variant | Fields | Raised when |
|---|---|---|
| `Database` | `message`, `code`, `recoverable` | Any engine failure, plus binding-level failures (JSON parsing, runtime setup). `code` carries the core taxonomy code (`"VELES-006"`, …) or is **empty** for binding-level errors |
| `Collection` | `message` | Collection-level failure |
| `DimensionMismatch` | `expected`, `actual` | Vector length differs from the collection dimension |

`recoverable` mirrors core's `Error::is_recoverable`, so a retry policy can be driven
from the FFI boundary without string matching. Every call that reaches core keeps
core's code and recoverability, `executeQuery`, `query`, `trainPq` and the sparse
and multi-query searches included; a VelesQL parse error is core's query error,
`VELES-010`. A core `DimensionMismatch` or a missing or duplicate collection
surfaces as the typed `DimensionMismatch` or `Collection` variant instead.

## Performance notes

1. Prefer `Sq8` on phones, `Binary` on constrained IoT hardware.
2. Load in bulk with `upsertBatch`, or `enableStreaming` + `streamInsert` for a
   continuous feed — not `upsert` in a loop.
3. Run every call off the main thread (see [Threading](#threading)).
4. Reuse the embedding buffers you hand to the binding; each call copies the vector
   across the FFI boundary.
5. ARM64 builds use core's NEON distance kernels (`velesdb_core::simd_native`,
   dispatched at runtime) and its NEON prefetching (`simd_neon_prefetch`).

## Memory footprint

Raw vector payload only — arithmetic from `dimension × bytes-per-dimension × count`.
It excludes the HNSW graph, payloads, and index structures, so treat it as a floor,
not a measurement.

| Vectors | Dimension | Storage mode | Vector memory |
|---------|-----------|--------------|--------|
| 10,000 | 384 | Full (f32) | ~15 MB |
| 10,000 | 384 | SQ8 | ~4 MB |
| 10,000 | 384 | Binary | ~0.5 MB |
| 100,000 | 768 | Full (f32) | ~300 MB |
| 100,000 | 768 | Binary | ~10 MB |

## See also

- [velesdb-mobile README](../../crates/velesdb-mobile/README.md)
- [Mobile build guide](./MOBILE_BUILD.md)
- [Ecosystem parity matrix](../reference/ECOSYSTEM_PARITY.md)
- [docs.rs/velesdb-mobile](https://docs.rs/velesdb-mobile)

---

Last updated: 2026-10-29 · Applies to: velesdb-core 7.1.0
