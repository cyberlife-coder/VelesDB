# VelesDB WASM — JavaScript API guide

Companion to [`crates/velesdb-wasm/README.md`](../../crates/velesdb-wasm/README.md).
This guide covers the full JavaScript surface exposed by `@wiscale/velesdb-wasm`:
`VectorStore`, `GraphStore`, `MemoryService`, `SemanticMemory`, `SparseIndex`,
and `WasmDatabase`.

The authoritative, always-current signature list is the TypeScript declaration
file shipped inside the package itself:
`node_modules/@wiscale/velesdb-wasm/velesdb_wasm.d.ts`. This guide explains
behaviour and marshalling rules, it does not duplicate every signature.

## Loading the module

The npm package is a `wasm-pack --target web` build: an ES module with a
**default export** that must be awaited before any class is constructed.

```javascript
import init, { VectorStore } from '@wiscale/velesdb-wasm';

await init();               // fetches velesdb_wasm_bg.wasm next to the JS file
const store = new VectorStore(3, 'cosine');
```

In a non-browser runtime (Node, a test harness) there is no `fetch` for
`file://` URLs, so pass the bytes explicitly:

```javascript
import fs from 'node:fs';
import init, { VectorStore } from '@wiscale/velesdb-wasm';

await init({
  module_or_path: fs.readFileSync(
    './node_modules/@wiscale/velesdb-wasm/velesdb_wasm_bg.wasm',
  ),
});
```

## Type marshalling: BigInt in, plain number out

This asymmetry is the single most common source of confusion, and it is a
consequence of two different boundaries:

| Direction | Representation | Why |
|---|---|---|
| Ids **passed in** (`insert`, `get`, `remove`, `GraphNode`, …) | `BigInt` (`1n`) | `wasm-bindgen` maps a Rust `u64` parameter to a JS `BigInt`. |
| Ids **returned by search** (`search`, `batch_search`, …) | plain `number` | Those results go through `serde-wasm-bindgen`, whose default serializer emits `u64` as a JS number (`serialize_large_number_types_as_bigints` defaults to `false`, see `serde-wasm-bindgen` 0.6 `src/ser.rs`). |
| Ids returned by a **direct getter** (`ParsedQuery.limit`, `GraphStore.get_neighbors`, `GraphNode.id`) | `BigInt` / `BigUint64Array` | Direct `wasm-bindgen` return, no serde in between. |
| Memory-wedge ids (`MemoryService`) | decimal **string** | Chosen so ids survive `JSON.stringify` without precision loss. |

Concretely, verified against `@wiscale/velesdb-wasm@4.0.0`:

```javascript
store.insert(1n, new Float32Array([1, 0, 0]));   // BigInt in
store.search(new Float32Array([1, 0, 0]), 2);    // => [[1, 1], [3, 0.9938837289810181]]
                                                 //     ^ typeof "number"
```

Ids above `Number.MAX_SAFE_INTEGER` (2^53 − 1) cannot be represented and make
serialization fail with an explicit error — keep ids under that bound.

## VectorStore

### Construction

| Constructor | Purpose |
|---|---|
| `new VectorStore(dimension, metric)` | Default, full-precision store. |
| `VectorStore.new_with_mode(dimension, metric, mode)` | Quantized store — `mode` is `'full'`, `'sq8'` or `'binary'`. |
| `VectorStore.with_capacity(dimension, metric, capacity)` | Pre-allocates, avoids repeated reallocation during bulk load. |
| `VectorStore.new_metadata_only()` | Payload-only store, no vectors. |

`metric` is one of `cosine`, `euclidean` (alias `l2`), `dot`, `hamming`,
`jaccard`. An unknown metric throws immediately.

Read-only **properties** (not methods): `store.len`, `store.is_empty`,
`store.dimension`, `store.storage_mode`, `store.is_metadata_only`.
Everything else is a method call.

### Distance metrics

| Metric | Direction | Best for |
|---|---|---|
| `cosine` | higher is better | Text embeddings (BERT, GPT, sentence-transformers) |
| `euclidean` (`l2`) | lower is better | Image features, spatial data |
| `dot` | higher is better | Pre-normalized vectors |
| `hamming` | lower is better | Binary vectors, fingerprints |
| `jaccard` | higher is better | Set similarity, sparse vectors |

Sorting direction comes from `DistanceMetric::higher_is_better()` in
`velesdb-core` (`crates/velesdb-core/src/distance.rs`), so browser ranking is
identical to server ranking for the same metric.

### Insert paths

```javascript
// One vector at a time.
store.insert(1n, new Float32Array([0.1, 0.2, 0.3]));

// One vector plus a JSON payload (needed for filters and text search).
store.insert_with_payload(2n, new Float32Array([0.3, 0.4, 0.5]), {
  category: 'tech',
  title: 'Vector search in the browser',
});

// Batch of [id, vector] pairs — one boundary crossing instead of N.
store.insert_batch([
  [3n, [0.5, 0.6, 0.7]],
  [4n, [0.8, 0.9, 1.0]],
]);

// Flat raw-bulk: contiguous ids + row-major vectors, the cheapest path.
store.insertBatchRaw(
  new BigUint64Array([5n, 6n]),
  new Float32Array([0.1, 0.2, 0.3, 0.4, 0.5, 0.6]),
  3,
);

// Reserve ahead of a known load.
store.reserve(50000);
```

Inserting an id that already exists replaces the previous entry — `insert` and
`insert_batch` are upserts, not duplicates.

### Search family

| Method | Returns | Notes |
|---|---|---|
| `search(query, k)` | `[[id, score], …]` | Brute-force k-NN over every vector. |
| `search_with_quality(query, k, quality)` | `[[id, score], …]` | Accepts the Python/Server quality strings (`fast`, `balanced`, `accurate`, `perfect`, `autotune`, `custom:<ef>`, `adaptive:<min>:<max>`). **All modes return identical results in WASM** — there is no HNSW graph; the parameter exists for API parity. |
| `search_with_filter(query, k, filter)` | `[{id, score, payload}, …]` | Metadata-filtered search. |
| `text_search(query, k, field?)` | `[{id, score, payload}, …]` | Full-text over payload fields. |
| `hybrid_search(vector, textQuery, k, vectorWeight?)` | `[{id, score, payload}, …]` | Dense + text fusion. |
| `multi_query_search(vectors, numVectors, k, strategy, rrfK?, weights?)` | `[[id, score], …]` | Multi-query fusion (MQG). |
| `batch_search(vectors, numVectors, k)` | `[[[id, score], …], …]` | One result list per query vector. |
| `similarity_search(query, threshold, operator, k)` | `[[id, score], …]` | Threshold-based selection. |
| `search_sparse(indices, values, k)` | `[{doc_id, score}, …]` | Throws when no sparse index has been built (parity with core). |
| `sparse_search(indices, values, k)` | `[{doc_id, score}, …]` | Same kernel, but returns `[]` instead of throwing when empty. |
| `query(vector, k)` | `[{nodeId, vectorScore, graphScore, fusedScore, bindings, columnData}, …]` | Multi-model result shape. |

A query whose length differs from `store.dimension` throws before any scoring
happens — the error names both the expected and the received length.

### Filter format

`search_with_filter` takes a plain object with a single `condition` key:

```javascript
// Equality
store.search_with_filter(query, 5, {
  condition: { type: 'eq', field: 'category', value: 'tech' },
});

// Comparison — also: gte, lt, lte, neq
store.search_with_filter(query, 5, {
  condition: { type: 'gt', field: 'price', value: 100 },
});

// Logical composition — also: or, not
store.search_with_filter(query, 5, {
  condition: {
    type: 'and',
    conditions: [
      { type: 'eq', field: 'category', value: 'tech' },
      { type: 'gt', field: 'views', value: 1000 },
    ],
  },
});
```

### Storage modes and memory

```javascript
const full   = new VectorStore(768, 'cosine');                              // default
const sq8    = VectorStore.new_with_mode(768, 'cosine', 'sq8');
const binary = VectorStore.new_with_mode(768, 'hamming', 'binary');

console.log(sq8.storage_mode);   // "sq8"
console.log(sq8.memory_usage()); // bytes actually held, per mode
```

`memory_usage()` measured for a **single 768-dimension vector**, i.e. the
per-vector cost including its 8-byte id (values reproduced with
`@wiscale/velesdb-wasm@4.0.0`):

| Mode | Bytes per 768-D vector | Compression | Trade-off |
|---|---|---|---|
| `full` | 3080 | 1× | Maximum precision, the default. |
| `sq8` | 784 | ~4× | Scalar quantization, ~1 % recall loss. |
| `binary` | 104 | ~30× | Edge / IoT / mobile PWA, ~5–10 % recall loss. |

`ProductQuantization` and `RaBitQ` exist in the storage-mode enum but are
**not usable from WASM**: they need `rayon`/`ndarray`/`persistence`, which are
compiled out for `wasm32-unknown-unknown`.

## GraphStore — in-memory knowledge graph

```javascript
import init, { GraphStore, GraphNode, GraphEdge } from '@wiscale/velesdb-wasm';

await init();
const graph = new GraphStore();

const alice = new GraphNode(1n, 'Person');
alice.set_string_property('name', 'Alice');
const bob = new GraphNode(2n, 'Person');
bob.set_string_property('name', 'Bob');

graph.add_node(alice);
graph.add_node(bob);
graph.add_edge(new GraphEdge(1n, 1n, 2n, 'KNOWS')); // (edgeId, source, target, label)

console.log(graph.get_neighbors(1n));   // BigUint64Array(1) [ 2n ]
console.log(graph.node_count(), graph.edge_count()); // 2 1
```

Traversal and inspection: `get_outgoing`, `get_incoming`,
`get_outgoing_by_label`, `bfs_traverse(sourceId, maxDepth, limit)`,
`dfs_traverse(sourceId, maxDepth, limit)`, `get_nodes_by_label`,
`get_edges_by_label`, `get_all_node_ids`, `get_all_edge_ids`, `has_node`,
`has_edge`, `out_degree`, `in_degree`, `remove_node`, `remove_edge`, `clear`.

Node properties are typed at write time: `set_string_property`,
`set_number_property`, `set_bool_property`, and `set_vector` for an embedding.

## MemoryService — the agent-memory wedge

`MemoryService` is the browser build of the
[`velesdb-memory`](../../crates/velesdb-memory/README.md) wedge: a semantic
store plus a typed-link graph, so a fact reachable only through a relation
(not through vector similarity) still surfaces — that is what makes `why()`
able to return the evidence path behind a recall. It is **in-memory only**
under WASM: there is no filesystem, so nothing survives a page reload unless
you persist it yourself.

```javascript
import init, { MemoryService } from '@wiscale/velesdb-wasm';

await init();
const memory = new MemoryService(384);   // embedding dimension

const pr = memory.remember('PR #42 swaps the mutex for parking_lot', [], null);
const decision = memory.remember(
  'we chose parking_lot to avoid lock poisoning',
  [{ target: pr, relation: 'decided_in' }],
  null,
);

memory.recall('lock poisoning', 5, null);
// => [ { id: "7771…", score: 0.5345…, content: "we chose parking_lot to avoid lock poisoning" },
//      { id: "5225…", score: 0,       content: "PR #42 swaps the mutex for parking_lot" } ]

const { nodes, edges } = memory.why('parking_lot', 2, null);
```

Full surface, as enumerated from `MemoryService.prototype` in the published
4.0.0 package:

| Group | Methods |
|---|---|
| Write | `remember(fact, links, metadata, ttlSeconds?)`, `relate(from, to, relation)`, `forget(id)` |
| Read | `recall(query, k?, filter)`, `recallWhere`, `recallFused`, `recallFusedDated`, `why(query, depth, filter)` |
| Context compiler | `compileContext`, `compileTranscript`, `contextSavings`, `explainCompilation`, `suggestBudget`, `retrieveContextSource` |
| Working context | `saveWorkingContext`, `loadWorkingContext`, `listWorkingContexts` |

Contract details:

- Ids are **decimal strings**.
- Every method is **synchronous** — no `Promise` to await.
- Failures throw a JS `Error` carrying a `.code` field: `INVALID_INPUT`,
  `NOT_FOUND`, or `INTERNAL`.
- `ttlSeconds` makes a fact expire; omit it or pass `0` for a permanent memory.
- `metadata` marshals as a **plain JS object**, not an ES2015 `Map` — this is
  pinned by a regression test (`tests/memory_wedge_web.rs`).
- A fact larger than `velesdb_memory::limits::MAX_FACT_BYTES` is rejected with
  `INVALID_INPUT` before it reaches the store.

Most applications should prefer the higher-level `MemoryService` re-exported
from [`@wiscale/velesdb-sdk`](../../sdks/typescript), which wraps this class
with Promise-returning methods and the SDK's typed error hierarchy.

## SemanticMemory — legacy, and its `query()` is broken

`SemanticMemory` predates `MemoryService` and stores `(id, content, embedding)`
triples. `store`, `len`, `dimension`, `delete`, `remove` and `clear` work, but
**`query()` throws `Invalid search results` on every call** in 4.0.0: the
implementation calls `JsValue::as_string()` on a value that
`serde-wasm-bindgen` produced as an array, so the conversion always fails
(`crates/velesdb-wasm/src/agent.rs`, `query`).

Reproduced against `@wiscale/velesdb-wasm@4.0.0`:

```javascript
const sm = new SemanticMemory(4);
sm.store(1n, 'Paris is the capital of France', new Float32Array([1, 0, 0, 0]));
sm.query(new Float32Array([1, 0, 0, 0]), 2);   // throws: Invalid search results
```

Use `MemoryService` (above), or a plain `VectorStore` with
`insert_with_payload` + `search_with_filter`, until this is fixed.

## SparseIndex and RRF fusion

```javascript
import init, { SparseIndex, hybrid_search_fuse } from '@wiscale/velesdb-wasm';

await init();
const index = new SparseIndex();

index.insert(1n, new Uint32Array([10, 20, 30]), new Float32Array([1.0, 0.5, 0.3]));
index.insert(2n, new Uint32Array([10, 40]),     new Float32Array([0.8, 1.2]));

index.search(new Uint32Array([10, 20]), new Float32Array([1.0, 1.0]), 5);
// => [ { doc_id: 1, score: 1.5 }, { doc_id: 2, score: 0.800000011920929 } ]

hybrid_search_fuse([[1, 0.9], [2, 0.5]], [[2, 0.8], [3, 0.4]], 60, 3);
// => [ { doc_id: 2, score: 0.0325… }, { doc_id: 1, score: 0.0163… }, { doc_id: 3, score: 0.0161… } ]
```

`hybrid_search_fuse(denseResults, sparseResults, rrfK, k)` delegates to
`velesdb_core::FusionStrategy::RRF`, so browser fusion reproduces core's
ranking exactly rather than re-deriving the formula.

## WasmDatabase — multi-collection handle

`WasmDatabase` groups several stores under names and is the only entry point
that runs VelesQL (`executeQuery`). See
[WASM_VELESQL.md](WASM_VELESQL.md) for the query surface.

| Method | Purpose |
|---|---|
| `create_collection(name, dimension, metric)` | Adds a vector collection. |
| `createMetadataCollection(name)` | Adds a payload-only collection (no `vector` column required). |
| `get_collection(name)` | Returns a `WasmCollectionHandle` (`insert`, `insertBatchRaw`, `search`, `remove`, and the `len` / `is_empty` / `dimension` properties). |
| `list_collections()` / `collection_count` | Introspection — `collection_count` is a property, not a call. |
| `delete_collection(name)` | Removes a collection. |
| `executeQuery(sql, paramsJson?)` | Runs VelesQL against a single collection. `paramsJson` is a **JSON string**, not an object; the result is a `QueryResult` with `kind`, `rowCount` and `rowsJson`. |

## Related

- [VelesDB WASM persistence and binary format](WASM_PERSISTENCE.md)
- [VelesQL in the browser](WASM_VELESQL.md)
- [Bundle size optimization](../wasm/bundle-optimization.md)
- Runnable examples: [`examples/wasm-browser-demo`](../../examples/wasm-browser-demo),
  [`examples/react-wasm-search`](../../examples/react-wasm-search)

---

Last updated: 2026-07-25 · Applies to: velesdb-core 4.0.0
