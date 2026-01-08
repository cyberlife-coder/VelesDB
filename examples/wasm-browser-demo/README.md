# VelesDB WASM Browser Demo

Interactive demo of VelesDB running entirely in the browser via WebAssembly.

## Features

- **100% Client-Side** - No server required
- **Real-time Performance** - See actual search latencies
- **Interactive** - Adjust vector count, dimensions, and top-k

## Quick Start

### Option 1: Open directly

Simply open `index.html` in a modern browser (Chrome, Firefox, Edge, Safari).

### Option 2: Local server

```bash
# Python
python -m http.server 8080

# Node.js
npx serve .
```

Then visit http://localhost:8080

## Live Demo

Open `index.html` directly in your browser - no build step required!

## How it Works

1. The page loads the `velesdb-wasm` npm package from unpkg CDN
2. WASM module is initialized in the browser
3. You can insert random vectors and search them
4. All computation happens locally in your browser

## Performance

Typical results on modern hardware:

| Operation | 10K vectors (128D) | 100K vectors (128D) |
|-----------|-------------------|---------------------|
| Insert    | ~50ms             | ~500ms              |
| Search    | ~1ms              | ~10ms               |

## Use Cases

- **Offline-first AI apps** - Semantic search without internet
- **Privacy-preserving** - Data never leaves the browser
- **Edge computing** - Run ML inference locally
- **Prototyping** - Quick vector search experiments

## Integration

```html
<script type="module">
  import init, { VectorStore } from 'https://unpkg.com/velesdb-wasm@latest/velesdb_wasm.js';

  await init();
  
  const store = new VectorStore(768, 'cosine');
  store.insert(1n, new Float32Array([0.1, 0.2, ...]));  // Note: ID is BigInt
  
  const results = store.search(query, 10);
  // results: Array of [id: BigInt, score: number]
</script>
```

## License

ELv2 (Elastic License 2.0)
