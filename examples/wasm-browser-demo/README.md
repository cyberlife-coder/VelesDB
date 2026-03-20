# VelesDB WASM Browser Demo

> **Difficulty: Beginner** | Showcases: WebAssembly vector search, client-side computation, zero-server setup

Interactive demo of VelesDB running entirely in the browser via WebAssembly.

## Features

- **100% Client-Side** - No server required
- **Real-time Performance** - See actual search latencies
- **Interactive** - Adjust vector count, dimensions, and top-k

## Quick Start

### Prerequisites

The demo loads the WASM module from npm CDN (unpkg/jsdelivr). If `velesdb-wasm` is not yet published on npm, build it locally first:

```bash
# Build the WASM package locally
cd crates/velesdb-wasm
wasm-pack build --target web --out-dir ../../examples/wasm-browser-demo/pkg

# Then update index.html to use the local path instead of CDN:
# Change: import init from 'https://unpkg.com/velesdb-wasm@1.6.0/velesdb_wasm.js'
# To:     import init from './pkg/velesdb_wasm.js'
```

### Run

```bash
cd examples/wasm-browser-demo

# Serve locally (required for WASM module loading)
python -m http.server 8080
# or: npx serve .
```

Then visit http://localhost:8080

## Live Demo

Open `index.html` directly in your browser - no build step required!

## Expected Output

When you open the page in a browser you will see an interactive panel where you can:
1. Choose the number of vectors and dimensions
2. Click "Insert" to generate and index random vectors
3. Click "Search" to run a nearest-neighbor query
4. Results appear instantly with IDs, scores, and latency in microseconds

No console output or build step is needed -- everything runs visually in the page.

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
  // If published on npm:
  // import init, { VectorStore } from 'https://unpkg.com/velesdb-wasm@1.6.0/velesdb_wasm.js';
  // If built locally with wasm-pack:
  import init, { VectorStore } from './pkg/velesdb_wasm.js';

  await init();

  const store = new VectorStore(768, 'cosine');
  store.insert(1n, new Float32Array([0.1, 0.2, ...]));  // Note: ID is BigInt

  const results = store.search(query, 10);
  // results: Array of [id: BigInt, score: number]
</script>
```

## License

MIT License
