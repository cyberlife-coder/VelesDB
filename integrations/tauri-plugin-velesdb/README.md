# tauri-plugin-velesdb

[![Crates.io](https://img.shields.io/crates/v/tauri-plugin-velesdb.svg)](https://crates.io/crates/tauri-plugin-velesdb)
[![License](https://img.shields.io/crates/l/tauri-plugin-velesdb.svg)](LICENSE)

A Tauri plugin for **VelesDB** - Vector search in desktop applications.

## Features

- 🚀 **Fast Vector Search** - Microsecond latency similarity search
- 📝 **Text Search** - BM25 full-text search across payloads
- 🔀 **Hybrid Search** - Combined vector + text with RRF fusion
- 🗃️ **Collection Management** - Create, list, and delete collections
- 📊 **VelesQL** - SQL-like query language
- 🔒 **Local-First** - All data stays on the user's device

## Installation

### Rust (Cargo.toml)

```toml
[dependencies]
tauri-plugin-velesdb = "0.1"
```

### JavaScript (package.json)

```json
{
  "dependencies": {
    "tauri-plugin-velesdb": "^0.1.0"
  }
}
```

```bash
npm install tauri-plugin-velesdb
# pnpm add tauri-plugin-velesdb
# yarn add tauri-plugin-velesdb
```

## Usage

### Rust - Plugin Registration

```rust
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_velesdb::init("./velesdb_data"))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### JavaScript - Frontend API

```javascript
import { invoke } from '@tauri-apps/api/core';

// Create a collection
await invoke('plugin:velesdb|create_collection', {
  request: {
    name: 'documents',
    dimension: 768,
    metric: 'cosine'  // cosine, euclidean, dot, hamming, jaccard
  }
});

// List collections
const collections = await invoke('plugin:velesdb|list_collections');
console.log(collections);
// [{ name: 'documents', dimension: 768, metric: 'cosine', count: 0 }]

// Insert vectors
await invoke('plugin:velesdb|upsert', {
  request: {
    collection: 'documents',
    points: [
      {
        id: 1,
        vector: [0.1, 0.2, 0.3, /* ... 768 dims */],
        payload: { title: 'Introduction to AI', category: 'tech' }
      },
      {
        id: 2,
        vector: [0.4, 0.5, 0.6, /* ... */],
        payload: { title: 'Machine Learning Guide', category: 'tech' }
      }
    ]
  }
});

// Vector similarity search
const results = await invoke('plugin:velesdb|search', {
  request: {
    collection: 'documents',
    vector: [0.15, 0.25, 0.35, /* ... */],
    topK: 5
  }
});
console.log(results);
// { results: [{ id: 1, score: 0.98, payload: {...} }], timingMs: 0.5 }

// Text search (BM25)
const textResults = await invoke('plugin:velesdb|text_search', {
  request: {
    collection: 'documents',
    query: 'machine learning guide',
    topK: 10
  }
});

// Hybrid search (vector + text)
const hybridResults = await invoke('plugin:velesdb|hybrid_search', {
  request: {
    collection: 'documents',
    vector: [0.1, 0.2, /* ... */],
    query: 'AI introduction',
    topK: 10,
    vectorWeight: 0.7  // 0.0-1.0, higher = more vector influence
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

## API Reference

### Commands

| Command | Description |
|---------|-------------|
| `create_collection` | Create a new vector collection |
| `delete_collection` | Delete a collection |
| `list_collections` | List all collections |
| `get_collection` | Get info about a collection |
| `upsert` | Insert or update vectors |
| `search` | Vector similarity search |
| `text_search` | BM25 full-text search |
| `hybrid_search` | Combined vector + text search |
| `query` | Execute VelesQL query |

### Distance Metrics

| Metric | Best For |
|--------|----------|
| `cosine` | Text embeddings (default) |
| `euclidean` | Spatial/geographic data |
| `dot` | Pre-normalized vectors |
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

See the [examples/basic-app](./examples/basic-app) directory for a complete Tauri app using this plugin.

## Performance

| Operation | Latency |
|-----------|---------|
| Vector search (10k vectors) | < 1ms |
| Text search (BM25) | < 5ms |
| Hybrid search | < 10ms |
| Insert (batch 100) | < 10ms |

## License

BSL-1.1 (Business Source License)

See [LICENSE](../../LICENSE) for details.
