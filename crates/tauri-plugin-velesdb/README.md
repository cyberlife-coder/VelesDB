# tauri-plugin-velesdb

[![Crates.io](https://img.shields.io/crates/v/tauri-plugin-velesdb.svg)](https://crates.io/crates/tauri-plugin-velesdb)
[![License](https://img.shields.io/badge/license-MIT-green)](LICENSE)

A Tauri plugin for **VelesDB** — Vector search in desktop applications.

## Features

- **Fast Vector Search** — Microsecond latency similarity search (HNSW + AVX2/AVX-512)
- **Text Search** — BM25 full-text search across payloads
- **Hybrid Search** — Combined vector + text with RRF fusion
- **Multi-Query Fusion** — MQG support with RRF / Weighted / Average strategies
- **Collection Management** — Create, list, and delete vector and metadata collections
- **Knowledge Graph** — Add edges, traverse (BFS/DFS), get node degrees
- **VelesQL** — SQL-like query language for advanced searches
- **Event System** — Real-time notifications for data changes
- **Local-First** — All data stays on the user's device

## Installation

### Rust (Cargo.toml)

```toml
[dependencies]
tauri-plugin-velesdb = "1"
```

### TypeScript SDK (package.json)

A typed JS/TS wrapper ships in `guest-js/index.ts`. Build it with your bundler or import directly:

```bash
npm install @wiscale/tauri-plugin-velesdb
# pnpm add @wiscale/tauri-plugin-velesdb
# yarn add @wiscale/tauri-plugin-velesdb
```

## Usage

### Rust — Plugin Registration

```rust
fn main() {
    tauri::Builder::default()
        // Default data directory: ./velesdb_data
        .plugin(tauri_plugin_velesdb::init())
        // Or specify a custom path:
        // .plugin(tauri_plugin_velesdb::init_with_path("./my_data"))
        // Or use the platform-specific app-data directory:
        // .plugin(tauri_plugin_velesdb::init_with_app_data("MyApp"))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### TypeScript SDK (recommended)

```typescript
import {
  createCollection, upsert, search,
  hybridSearch, textSearch, multiQuerySearch,
  listCollections, deleteCollection,
  createMetadataCollection, upsertMetadata,
  addEdge, traverseGraph, getNodeDegree,
  isEmpty, flush
} from '@wiscale/tauri-plugin-velesdb';

// Create a collection
await createCollection({ name: 'documents', dimension: 768, metric: 'cosine' });

// Insert vectors
await upsert({
  collection: 'documents',
  points: [
    { id: 1, vector: [0.1, 0.2, /* ... 768 dims */], payload: { title: 'Intro to AI' } },
    { id: 2, vector: [0.4, 0.5, /* ... */],          payload: { title: 'ML Guide' } }
  ]
});

// Vector similarity search
const results = await search({
  collection: 'documents',
  vector: [0.15, 0.25, /* ... */],
  topK: 5
});
// { results: [{ id: 1, score: 0.98, payload: {...} }], timingMs: 0.5 }
```

### JavaScript (raw `invoke`)

```javascript
import { invoke } from '@tauri-apps/api/core';

// Create a collection
await invoke('plugin:velesdb|create_collection', {
  request: {
    name: 'documents',
    dimension: 768,
    metric: 'cosine',    // cosine | euclidean | dot | hamming | jaccard
    storageMode: 'full'  // full | sq8 | binary | pq
  }
});

// Insert vectors
await invoke('plugin:velesdb|upsert', {
  request: {
    collection: 'documents',
    points: [
      { id: 1, vector: [0.1, 0.2, /* ... */], payload: { title: 'Intro to AI' } }
    ]
  }
});

// Vector similarity search
const results = await invoke('plugin:velesdb|search', {
  request: { collection: 'documents', vector: [0.15, 0.25, /* ... */], topK: 5 }
});

// Text search (BM25)
const textResults = await invoke('plugin:velesdb|text_search', {
  request: { collection: 'documents', query: 'machine learning guide', topK: 10 }
});

// Hybrid search (vector + text with RRF)
const hybridResults = await invoke('plugin:velesdb|hybrid_search', {
  request: {
    collection: 'documents',
    vector: [0.1, 0.2, /* ... */],
    query: 'AI introduction',
    topK: 10,
    vectorWeight: 0.7  // 0.0–1.0, higher = more vector influence
  }
});

// Multi-query fusion search (MQG)
const mqResults = await invoke('plugin:velesdb|multi_query_search', {
  request: {
    collection: 'documents',
    vectors: [
      [0.1, 0.2, /* query 1 */],
      [0.3, 0.4, /* query 2 */]
    ],
    topK: 10,
    fusion: 'rrf',          // rrf | average | maximum | weighted
    fusionParams: { k: 60 } // RRF k parameter
  }
});

// VelesQL query
const queryResults = await invoke('plugin:velesdb|query', {
  request: {
    query: "SELECT * FROM documents WHERE content MATCH 'rust' LIMIT 10",
    params: {}
  }
});

// Delete collection
await invoke('plugin:velesdb|delete_collection', { name: 'documents' });
```

### Knowledge Graph

```javascript
// Add a directed edge
await invoke('plugin:velesdb|add_edge', {
  request: {
    collection: 'documents',
    id: 1,
    source: 100,
    target: 200,
    label: 'REFERENCES',
    properties: { weight: 0.8, created: '2026-01-01' }
  }
});

// Query edges by label / source / target
const edges = await invoke('plugin:velesdb|get_edges', {
  request: { collection: 'documents', label: 'REFERENCES' }
});

// Graph traversal (BFS or DFS)
const traversal = await invoke('plugin:velesdb|traverse_graph', {
  request: {
    collection: 'documents',
    source: 100,
    maxDepth: 3,
    relTypes: ['REFERENCES', 'CITES'],
    limit: 50,
    algorithm: 'bfs'  // bfs | dfs
  }
});

// Node degree
const degree = await invoke('plugin:velesdb|get_node_degree', {
  request: { collection: 'documents', nodeId: 100 }
});
// { nodeId: 100, inDegree: 5, outDegree: 3 }
```

### Event System

```javascript
import { listen } from '@tauri-apps/api/event';

await listen('velesdb://collection-created', (event) => {
  console.log('New collection:', event.payload.collection);
});

await listen('velesdb://collection-updated', (event) => {
  console.log(`${event.payload.operation}: ${event.payload.count} items`);
});

await listen('velesdb://collection-deleted', (event) => {
  console.log('Deleted:', event.payload.collection);
});

await listen('velesdb://operation-progress', (event) => {
  console.log(`Progress: ${event.payload.progress}%`);
});

await listen('velesdb://operation-complete', (event) => {
  console.log(`Done in ${event.payload.durationMs}ms`);
});
```

### Accessing `VelesDbState` from custom Tauri commands

```rust
use tauri::{AppHandle, Manager};
use tauri_plugin_velesdb::VelesDbState;
use velesdb_core::{DistanceMetric, Point};
use std::sync::Arc;

#[tauri::command]
async fn my_command(app: AppHandle) -> Result<usize, String> {
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

## API Reference

### Commands

| Command | Description |
|---------|-------------|
| `create_collection` | Create a vector collection |
| `create_metadata_collection` | Create a metadata-only collection (no vectors) |
| `delete_collection` | Delete a collection and all its data |
| `list_collections` | List all collections with metadata |
| `get_collection` | Get info about a specific collection |
| `upsert` | Insert or update vectors with payloads |
| `upsert_metadata` | Insert or update metadata-only points |
| `get_points` | Retrieve points by IDs |
| `delete_points` | Delete points by IDs |
| `search` | Vector similarity search |
| `batch_search` | Parallel batch vector search (multiple queries) |
| `multi_query_search` | Multi-query fusion search (RRF / Weighted / Average) |
| `text_search` | BM25 full-text search |
| `hybrid_search` | Combined vector + text search with RRF fusion |
| `query` | Execute a VelesQL query |
| `is_empty` | Check if a collection has no points |
| `flush` | Flush pending writes to disk |
| `add_edge` | Add a directed edge to the knowledge graph |
| `get_edges` | Query edges by label / source / target |
| `traverse_graph` | BFS / DFS graph traversal from a node |
| `get_node_degree` | Get in-degree and out-degree of a node |
| `semantic_store` | Store a knowledge fact (Agent Memory SDK) |
| `semantic_query` | Retrieve semantically similar facts |

### Events

| Event | Payload | Description |
|-------|---------|-------------|
| `velesdb://collection-created` | `{ collection, operation }` | Collection created |
| `velesdb://collection-deleted` | `{ collection, operation }` | Collection deleted |
| `velesdb://collection-updated` | `{ collection, operation, count }` | Data modified |
| `velesdb://operation-progress` | `{ operationId, progress, total, processed }` | Progress update |
| `velesdb://operation-complete` | `{ operationId, success, error?, durationMs? }` | Operation done |

### Storage Modes

| Mode | Compression | Best For |
|------|-------------|----------|
| `full` | 1× (f32) | Maximum accuracy |
| `sq8` | 4× | Good accuracy / memory balance |
| `binary` | 32× | Edge / IoT, massive scale |
| `pq` | Variable | Product quantization, ultra-compact |

### Distance Metrics

| Metric | Best For |
|--------|----------|
| `cosine` | Text embeddings (default) |
| `euclidean` | Spatial / geographic data |
| `dot` | Pre-normalized vectors, max inner product |
| `hamming` | Binary vectors |
| `jaccard` | Set similarity |

## Permissions

Add to your `capabilities/default.json`:

```json
{
  "permissions": [
    "velesdb:default"
  ]
}
```

Or for granular control:

```json
{
  "permissions": [
    "velesdb:allow-create-collection",
    "velesdb:allow-search",
    "velesdb:allow-upsert"
  ]
}
```

## Example App

See [`demos/tauri-rag-app`](../../../demos/tauri-rag-app) for a complete desktop RAG application using this plugin with:

- `fastembed` (AllMiniLML6V2, 384D) for local ML embeddings
- Full persistent `VectorCollection` (text stored in Point payload)
- Chunk ingestion, vector search, and statistics UI

## Performance

| Operation | Latency |
|-----------|---------|
| Vector search (10k vectors) | < 1ms |
| Text search (BM25) | < 5ms |
| Hybrid search | < 10ms |
| Insert (batch 100) | < 10ms |

## License

VelesDB Core License 1.0

See [LICENSE](../../LICENSE) for details.
