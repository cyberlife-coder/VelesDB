# Tauri plugin — recipes

*Last updated: 2026-07-25 · Applies to: velesdb-core 5.1.0*

Copy-pasteable snippets for [`tauri-plugin-velesdb`](../../crates/tauri-plugin-velesdb/README.md),
beyond the 60-second first success in the crate README. Command names, payload
shapes and permissions are listed in the
[plugin reference](./TAURI_PLUGIN_REFERENCE.md).

Every snippet assumes the plugin is registered and that the calling window has
the `velesdb:default` capability.

---

## TypeScript SDK

The typed wrapper lives in `crates/tauri-plugin-velesdb/guest-js/index.ts` and
ships as `@wiscale/tauri-plugin-velesdb`. It is the recommended surface: the
request and response types are checked at compile time.

```typescript
import {
  createCollection, upsert, search,
  textSearch, hybridSearch, multiQuerySearch,
  listCollections, deleteCollection,
  isEmpty, flush
} from '@wiscale/tauri-plugin-velesdb';

await createCollection({ name: 'documents', dimension: 4, metric: 'cosine' });

await upsert({
  collection: 'documents',
  points: [
    { id: 1, vector: [1, 0, 0, 0], payload: { title: 'Intro to AI' } },
    { id: 2, vector: [0, 1, 0, 0], payload: { title: 'ML Guide' } }
  ]
});

const results = await search({
  collection: 'documents',
  vector: [1, 0, 0, 0],
  topK: 5
});
console.log(results.results[0].id, results.results[0].payload.title);
// 1 Intro to AI
```

`search` resolves to `{ results: SearchResult[], timingMs: number }`, where each
result is `{ id, score, payload? }`.

---

## Raw `invoke`

No extra dependency beyond `@tauri-apps/api`. Every command takes a single
`request` object, except `delete_collection`, which takes `name`.

```javascript
import { invoke } from '@tauri-apps/api/core';

await invoke('plugin:velesdb|create_collection', {
  request: {
    name: 'documents',
    dimension: 4,
    metric: 'cosine',    // cosine | euclidean | dot | hamming | jaccard
    storageMode: 'full'  // full | sq8 | binary | pq | rabitq
  }
});

await invoke('plugin:velesdb|upsert', {
  request: {
    collection: 'documents',
    points: [{ id: 1, vector: [1, 0, 0, 0], payload: { title: 'Intro to AI' } }]
  }
});

const results = await invoke('plugin:velesdb|search', {
  request: { collection: 'documents', vector: [1, 0, 0, 0], topK: 5 }
});

await invoke('plugin:velesdb|delete_collection', { name: 'documents' });
```

### Text, hybrid and multi-query search

```javascript
const textResults = await invoke('plugin:velesdb|text_search', {
  request: { collection: 'documents', query: 'machine learning guide', topK: 10 }
});

const hybridResults = await invoke('plugin:velesdb|hybrid_search', {
  request: {
    collection: 'documents',
    vector: [1, 0, 0, 0],
    query: 'AI introduction',
    topK: 10,
    vectorWeight: 0.7  // 0.0-1.0, higher = more vector influence
  }
});

const mqResults = await invoke('plugin:velesdb|multi_query_search', {
  request: {
    collection: 'documents',
    vectors: [[1, 0, 0, 0], [0, 1, 0, 0]],
    topK: 10,
    fusion: 'rrf',          // rrf | average | maximum | weighted | relative_score
    fusionParams: { k: 60 } // RRF k parameter
  }
});
```

### VelesQL

The `query` request carries **only** `query` and `params`. There is no
`collection` field: the target collection comes from the `FROM` clause (or the
`MATCH` pattern) inside the query itself.

```javascript
const queryResults = await invoke('plugin:velesdb|query', {
  request: {
    query: "SELECT * FROM documents WHERE content MATCH 'rust' LIMIT 10",
    params: {}
  }
});
// { results: [{ nodeId, vectorScore, graphScore, fusedScore, bindings, columnData }], timingMs }
```

Cross-collection `MATCH` — nodes annotated with `@collection` in the pattern
have their payloads looked up from the named collection after traversal:

```javascript
const crossColl = await invoke('plugin:velesdb|query', {
  request: {
    query: "MATCH (p:Product)-[:STORED_IN]->(inv:Inventory@inventory) RETURN p.name, inv.price, inv.stock LIMIT 20",
    params: {}
  }
});
```

More patterns: [Graph patterns](./GRAPH_PATTERNS.md) and
[Multi-model queries](./MULTIMODEL_QUERIES.md).

---

## Knowledge graph

Graph commands operate on a **graph collection**, not on a vector collection:
`add_edge` and friends resolve the collection through `get_graph_collection`
and reject anything else with `VELES-002`.

An edge is refused unless both endpoints already have a stored node payload
(#1442). No IPC command upserts a graph node payload today, so nodes have to be
created on the Rust side (see [From your own Tauri
commands](#from-your-own-tauri-commands) below):

```rust
state.with_db(|db| {
    let coll = db
        .get_graph_collection("catalog_graph")
        .ok_or_else(|| tauri_plugin_velesdb::Error::CollectionNotFound(
            "catalog_graph".to_string()
        ))?;
    for node_id in [100_u64, 200, 300] {
        coll.upsert_node_payload(node_id, &serde_json::json!({ "kind": "product" }))
            .map_err(tauri_plugin_velesdb::Error::Database)?;
    }
    Ok(())
})?;
```

Once the nodes exist, the frontend can drive the graph:

```javascript
await invoke('plugin:velesdb|create_graph_collection', {
  request: { name: 'catalog_graph' }
});

await invoke('plugin:velesdb|add_edge', {
  request: {
    collection: 'catalog_graph',
    id: 1,
    source: 100,
    target: 200,
    label: 'REFERENCES',
    properties: { weight: 0.8, created: '2026-01-01' }
  }
});

const edges = await invoke('plugin:velesdb|get_edges', {
  request: { collection: 'catalog_graph', label: 'REFERENCES' }
});
// [{ id: 1, source: 100, target: 200, label: 'REFERENCES', properties: {...} }]

const traversal = await invoke('plugin:velesdb|traverse_graph', {
  request: {
    collection: 'catalog_graph',
    source: 100,
    maxDepth: 3,
    relTypes: ['REFERENCES', 'CITES'],
    limit: 50,
    algorithm: 'bfs'  // bfs | dfs
  }
});
// [{ targetId: 200, depth: 1, path: [100, 200] }]

const degree = await invoke('plugin:velesdb|get_node_degree', {
  request: { collection: 'catalog_graph', nodeId: 100 }
});
// { nodeId: 100, inDegree: 5, outDegree: 3 }

const parallel = await invoke('plugin:velesdb|traverse_graph_parallel', {
  request: {
    collection: 'catalog_graph',
    sources: [100, 200, 300],
    maxDepth: 3,
    limit: 50
  }
});
```

`maxDepth` defaults to 3, `limit` to 100, `algorithm` to `bfs` when omitted.

---

## Secondary indexes

Secondary indexes apply to vector collections and speed up filtered search.

```javascript
await invoke('plugin:velesdb|create_index', {
  request: { collection: 'documents', fieldName: 'category' }
});

const indexes = await invoke('plugin:velesdb|list_indexes', {
  request: { collection: 'documents' }
});
// [{ label: "secondary", property: "category", indexType: "hash", cardinality: 42, memoryBytes: 1024 }]

const dropped = await invoke('plugin:velesdb|drop_index', {
  request: { collection: 'documents', fieldName: 'category' }
});
// true when an index was actually removed
```

---

## Sparse vectors

A sparse vector is a plain object mapping a dimension index (as a string) to a
weight.

```javascript
await invoke('plugin:velesdb|sparse_upsert', {
  request: {
    collection: 'documents',
    points: [{
      id: 1,
      vector: [1, 0, 0, 0],
      payload: { title: 'Doc' },
      sparseVector: { "42": 0.8, "7": 1.2, "100": 0.5 }
    }]
  }
});

const sparseResults = await invoke('plugin:velesdb|sparse_search', {
  request: {
    collection: 'documents',
    sparseVector: { "42": 1.0, "7": 0.5 },
    topK: 10
  }
});

const hybridSparse = await invoke('plugin:velesdb|hybrid_sparse_search', {
  request: {
    collection: 'documents',
    vector: [1, 0, 0, 0],
    sparseVector: { "42": 1.0, "7": 0.5 },
    topK: 10
  }
});
```

`sparse_search` also accepts an optional `indexName` when several sparse
indexes coexist on the collection.

---

## Reacting to data changes

```javascript
import { listen } from '@tauri-apps/api/event';

await listen('velesdb://collection-created', (event) => {
  console.log('New collection:', event.payload.collection);
});

await listen('velesdb://collection-deleted', (event) => {
  console.log('Deleted:', event.payload.collection);
});

await listen('velesdb://collection-updated', (event) => {
  console.log(`${event.payload.operation}: ${event.payload.count} items`);
});
```

`velesdb://operation-progress` and `velesdb://operation-complete` are declared
but not emitted by any command yet — see the
[reference](./TAURI_PLUGIN_REFERENCE.md#events).

---

## From your own Tauri commands

`VelesDbState` is managed by the plugin, so any command in your app can reach
the engine directly and use the full `velesdb-core` API — including everything
the IPC surface does not expose.

```rust
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tauri_plugin_velesdb::VelesDbState;

#[tauri::command]
async fn count_matches(app: AppHandle) -> Result<usize, String> {
    let state = app.state::<VelesDbState>();
    state
        .with_db(|db: Arc<velesdb_core::Database>| {
            let coll = db
                .get_vector_collection("my-collection")
                .ok_or_else(|| tauri_plugin_velesdb::Error::CollectionNotFound(
                    "my-collection".to_string()
                ))?;
            coll.search(&[0.1_f32; 384], 5)
                .map(|r| r.len())
                .map_err(tauri_plugin_velesdb::Error::Database)
        })
        .map_err(|e| format!("{e}"))
}
```

This is the pattern the [RAG demo](../../demos/tauri-rag-app) uses: embeddings
are computed in Rust, and the frontend only ever calls the app's own commands.

---

[Back to the guides index](./README.md) ·
[Plugin README](../../crates/tauri-plugin-velesdb/README.md) ·
[Reference](./TAURI_PLUGIN_REFERENCE.md)
