# VelesDB WASM

[![npm](https://img.shields.io/npm/v/velesdb-wasm)](https://www.npmjs.com/package/velesdb-wasm)
[![License](https://img.shields.io/badge/license-BSL--1.1-blue)](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE)

WebAssembly build of [VelesDB](https://github.com/cyberlife-coder/VelesDB) - vector search in the browser.

## Features

- **In-browser vector search** - No server required
- **SIMD optimized** - Uses WASM SIMD128 for fast distance calculations
- **Multiple metrics** - Cosine, Euclidean, Dot Product
- **Lightweight** - Minimal bundle size

## Installation

```bash
npm install velesdb-wasm
```

## Usage

```javascript
import init, { VectorStore } from 'velesdb-wasm';

async function main() {
  // Initialize WASM module
  await init();

  // Create a vector store (768 dimensions, cosine similarity)
  const store = new VectorStore(768, 'cosine');

  // Insert vectors (use BigInt for IDs)
  store.insert(1n, new Float32Array([0.1, 0.2, ...]));
  store.insert(2n, new Float32Array([0.3, 0.4, ...]));

  // Search for similar vectors
  const query = new Float32Array([0.15, 0.25, ...]);
  const results = store.search(query, 5); // Top 5 results

  // Results: [[id, score], [id, score], ...]
  console.log(results);
}

main();
```

### High-Performance Bulk Insert

For optimal performance when inserting many vectors:

```javascript
// Pre-allocate capacity (avoids repeated memory allocations)
const store = VectorStore.with_capacity(768, 'cosine', 100000);

// Batch insert (much faster than individual inserts)
const batch = [
  [1n, [0.1, 0.2, ...]],
  [2n, [0.3, 0.4, ...]],
  // ... more vectors
];
store.insert_batch(batch);

// Or reserve capacity on existing store
store.reserve(50000);
```

## API

### VectorStore

```typescript
class VectorStore {
  // Create a new store
  constructor(dimension: number, metric: 'cosine' | 'euclidean' | 'dot');
  
  // Create with pre-allocated capacity (performance optimization)
  static with_capacity(dimension: number, metric: string, capacity: number): VectorStore;

  // Properties
  readonly len: number;
  readonly is_empty: boolean;
  readonly dimension: number;

  // Methods
  insert(id: bigint, vector: Float32Array): void;
  insert_batch(batch: Array<[bigint, number[]]>): void;  // Bulk insert
  search(query: Float32Array, k: number): Array<[bigint, number]>;
  remove(id: bigint): boolean;
  clear(): void;
  reserve(additional: number): void;  // Pre-allocate memory
  memory_usage(): number;
}
```

## Distance Metrics

| Metric | Description | Best For |
|--------|-------------|----------|
| `cosine` | Cosine similarity | Text embeddings (BERT, GPT) |
| `euclidean` | L2 distance | Image features, spatial data |
| `dot` | Dot product | Pre-normalized vectors |

## IndexedDB Persistence

Save and restore your vector store for offline-first applications:

```javascript
import init, { VectorStore } from 'velesdb-wasm';

// Save to IndexedDB
async function saveToIndexedDB(store, dbName = 'velesdb') {
  const bytes = store.export_to_bytes();
  
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(dbName, 1);
    
    request.onupgradeneeded = (e) => {
      e.target.result.createObjectStore('vectors');
    };
    
    request.onsuccess = (e) => {
      const db = e.target.result;
      const tx = db.transaction('vectors', 'readwrite');
      tx.objectStore('vectors').put(bytes, 'store');
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    };
  });
}

// Load from IndexedDB
async function loadFromIndexedDB(dbName = 'velesdb') {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(dbName, 1);
    
    request.onsuccess = (e) => {
      const db = e.target.result;
      const tx = db.transaction('vectors', 'readonly');
      const getReq = tx.objectStore('vectors').get('store');
      
      getReq.onsuccess = () => {
        if (getReq.result) {
          const store = VectorStore.import_from_bytes(getReq.result);
          resolve(store);
        } else {
          resolve(null);
        }
      };
    };
  });
}

// Usage
async function main() {
  await init();
  
  // Try to load existing store
  let store = await loadFromIndexedDB();
  
  if (!store) {
    // Create new store
    store = new VectorStore(768, 'cosine');
    store.insert(1n, new Float32Array(768).fill(0.1));
  }
  
  // ... use store ...
  
  // Save before closing
  await saveToIndexedDB(store);
}
```

### Persistence API

```typescript
class VectorStore {
  // Export to binary format (for IndexedDB, localStorage, file download)
  export_to_bytes(): Uint8Array;
  
  // Import from binary format (static constructor)
  static import_from_bytes(bytes: Uint8Array): VectorStore;
}
```

### Binary Format

| Field | Size | Description |
|-------|------|-------------|
| Magic | 4 bytes | `"VELS"` |
| Version | 1 byte | Format version (1) |
| Dimension | 4 bytes | Vector dimension (u32 LE) |
| Metric | 1 byte | 0=cosine, 1=euclidean, 2=dot |
| Count | 8 bytes | Number of vectors (u64 LE) |
| Vectors | variable | id (8B) + data (dim × 4B) each |

### Performance

Ultra-fast serialization thanks to contiguous memory layout:

| Operation | 10k vectors (768D) | Throughput |
|-----------|-------------------|------------|
| Export | ~7 ms | **4479 MB/s** |
| Import | ~10 ms | **2943 MB/s** |

## Use Cases

- **Browser-based RAG** - 100% client-side semantic search
- **Offline-first apps** - Works without internet, persists to IndexedDB
- **Privacy-preserving AI** - Data never leaves the browser
- **Electron/Tauri apps** - Desktop AI without a server
- **PWA applications** - Full offline support with service workers

## Building from Source

```bash
# Install wasm-pack
cargo install wasm-pack

# Build for browser
wasm-pack build --target web

# Build for Node.js
wasm-pack build --target nodejs
```

## Performance

Typical latencies on modern browsers:

| Operation | 768D vectors | 10K vectors |
|-----------|--------------|-------------|
| Insert | ~1 µs | ~10 ms |
| Search | ~50 µs | ~5 ms |

## License

Business Source License 1.1 (BSL-1.1)

See [LICENSE](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE) for details.
