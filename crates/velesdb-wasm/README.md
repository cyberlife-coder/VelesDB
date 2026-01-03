# VelesDB WASM

[![npm](https://img.shields.io/npm/v/@wiscale/velesdb-wasm)](https://www.npmjs.com/package/@wiscale/velesdb-wasm)
[![License](https://img.shields.io/badge/license-ELv2-blue)](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE)

WebAssembly build of [VelesDB](https://github.com/cyberlife-coder/VelesDB) - vector search in the browser.

## Features

- **In-browser vector search** - No server required
- **SIMD optimized** - Uses WASM SIMD128 for fast distance calculations
- **Multiple metrics** - Cosine, Euclidean, Dot Product, Hamming, Jaccard
- **Memory optimization** - SQ8 (4x) and Binary (32x) quantization
- **Lightweight** - Minimal bundle size

## Installation

```bash
npm install @wiscale/velesdb-wasm
```

## Usage

```javascript
import init, { VectorStore } from '@wiscale/velesdb-wasm';

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
  constructor(dimension: number, metric: 'cosine' | 'euclidean' | 'dot' | 'hamming' | 'jaccard');
  
  // Create with storage mode (sq8/binary for memory optimization)
  static new_with_mode(dimension: number, metric: string, mode: 'full' | 'sq8' | 'binary'): VectorStore;
  
  // Create with pre-allocated capacity (performance optimization)
  static with_capacity(dimension: number, metric: string, capacity: number): VectorStore;

  // Properties
  readonly len: number;
  readonly is_empty: boolean;
  readonly dimension: number;
  readonly storage_mode: string;  // "full", "sq8", or "binary"

  // Methods
  insert(id: bigint, vector: Float32Array): void;
  insert_with_payload(id: bigint, vector: Float32Array, payload: object): void;  // v0.8.5+
  insert_batch(batch: Array<[bigint, number[]]>): void;  // Bulk insert
  search(query: Float32Array, k: number): Array<[bigint, number]>;
  search_with_filter(query: Float32Array, k: number, filter: object): Array<{id, score, payload}>;  // v0.8.5+
  text_search(query: string, k: number, field?: string): Array<{id, score, payload}>;  // v0.8.5+
  get(id: bigint): {id, vector, payload} | null;  // v0.8.5+
  remove(id: bigint): boolean;
  clear(): void;
  reserve(additional: number): void;  // Pre-allocate memory
  memory_usage(): number;  // Accurate for each storage mode
}
```

### Filter Format (v0.8.5+)

```javascript
// Equality filter
const filter = {
  condition: { type: "eq", field: "category", value: "tech" }
};

// Comparison filters
const filter = {
  condition: { type: "gt", field: "price", value: 100 }
};  // Also: gte, lt, lte, neq

// Logical operators
const filter = {
  condition: {
    type: "and",
    conditions: [
      { type: "eq", field: "category", value: "tech" },
      { type: "gt", field: "views", value: 1000 }
    ]
  }
};  // Also: or, not
```

## Distance Metrics

| Metric | Description | Best For |
|--------|-------------|----------|
| `cosine` | Cosine similarity | Text embeddings (BERT, GPT) |
| `euclidean` | L2 distance | Image features, spatial data |
| `dot` | Dot product | Pre-normalized vectors |
| `hamming` | Hamming distance | Binary vectors, fingerprints |
| `jaccard` | Jaccard similarity | Set similarity, sparse vectors |

## Storage Modes (Memory Optimization)

Reduce memory usage with quantization:

```javascript
// Full precision (default) - best recall
const full = new VectorStore(768, 'cosine');

// SQ8: 4x memory reduction (~1% recall loss)
const sq8 = VectorStore.new_with_mode(768, 'cosine', 'sq8');

// Binary: 32x memory reduction (~5-10% recall loss)
const binary = VectorStore.new_with_mode(768, 'hamming', 'binary');

console.log(sq8.storage_mode);  // "sq8"
```

| Mode | Memory (768D) | Compression | Use Case |
|------|---------------|-------------|----------|
| `full` | 3080 bytes | 1x | Default, max precision |
| `sq8` | 784 bytes | **4x** | Scale, RAM-constrained |
| `binary` | 104 bytes | **32x** | Edge, IoT, mobile PWA |

## IndexedDB Persistence

Save and restore your vector store for offline-first applications with built-in async methods:

```javascript
import init, { VectorStore } from '@wiscale/velesdb-wasm';

async function main() {
  await init();

  // Create and populate a store
  const store = new VectorStore(768, 'cosine');
  store.insert(1n, new Float32Array(768).fill(0.1));
  store.insert(2n, new Float32Array(768).fill(0.2));

  // Save to IndexedDB (single async call)
  await store.save('my-vectors-db');
  console.log('Saved!', store.len, 'vectors');

  // Later: Load from IndexedDB
  const restored = await VectorStore.load('my-vectors-db');
  console.log('Restored!', restored.len, 'vectors');

  // Clean up: Delete database
  await VectorStore.delete_database('my-vectors-db');
}

main();
```

### Persistence API

```typescript
class VectorStore {
  // Save to IndexedDB (async)
  save(db_name: string): Promise<void>;
  
  // Load from IndexedDB (async, static)
  static load(db_name: string): Promise<VectorStore>;
  
  // Delete IndexedDB database (async, static)
  static delete_database(db_name: string): Promise<void>;
  
  // Manual binary export/import (for localStorage, file download, etc.)
  export_to_bytes(): Uint8Array;
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

Elastic License 2.0 (ELv2)

See [LICENSE](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE) for details.
