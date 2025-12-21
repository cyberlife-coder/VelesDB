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

  // Insert vectors
  store.insert(1, new Float32Array([0.1, 0.2, ...]));
  store.insert(2, new Float32Array([0.3, 0.4, ...]));

  // Search for similar vectors
  const query = new Float32Array([0.15, 0.25, ...]);
  const results = store.search(query, 5); // Top 5 results

  // Results: [[id, score], [id, score], ...]
  console.log(results);
}

main();
```

## API

### VectorStore

```typescript
class VectorStore {
  // Create a new store
  constructor(dimension: number, metric: 'cosine' | 'euclidean' | 'dot');

  // Properties
  readonly len: number;
  readonly is_empty: boolean;
  readonly dimension: number;

  // Methods
  insert(id: number, vector: Float32Array): void;
  search(query: Float32Array, k: number): Array<[number, number]>;
  remove(id: number): boolean;
  clear(): void;
  memory_usage(): number;
}
```

## Distance Metrics

| Metric | Description | Best For |
|--------|-------------|----------|
| `cosine` | Cosine similarity | Text embeddings (BERT, GPT) |
| `euclidean` | L2 distance | Image features, spatial data |
| `dot` | Dot product | Pre-normalized vectors |

## Use Cases

- **Browser-based RAG** - 100% client-side semantic search
- **Offline-first apps** - Works without internet
- **Privacy-preserving AI** - Data never leaves the browser
- **Electron/Tauri apps** - Desktop AI without a server

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
