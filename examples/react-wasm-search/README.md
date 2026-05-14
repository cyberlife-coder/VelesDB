# React + Vite — In-Browser Vector Search with VelesDB WASM

A standalone single-page app that runs vector similarity search **entirely in
the browser** using [`@wiscale/velesdb-wasm`](https://www.npmjs.com/package/@wiscale/velesdb-wasm).
No backend, no API calls, no model download.

## Run it

```bash
cd examples/react-wasm-search
npm install
npm run dev
```

Then open the URL Vite prints (typically <http://localhost:5173>).

## What it shows

- Loading the WebAssembly build of VelesDB in a Vite + React setup.
- Building an in-memory `VectorStore` of 100 product records on startup.
- Running a cosine-similarity search on every keystroke and displaying
  results with the per-query latency in milliseconds.

The same `embed()` function (a tiny hashing-trick projection — see
[`src/embed.ts`](src/embed.ts)) is applied to both the seed corpus and the
live query, so lexical overlap drives ranking. Real applications would
swap this for a sentence-transformer embedding pipeline (server-side or
via a model running on the client through `transformers.js`).

## Files

| Path | Purpose |
|---|---|
| `src/main.tsx` | React entry point. |
| `src/App.tsx` | UI + search loop. Initializes the WASM module, builds the index, runs `store.search()` per keystroke. |
| `src/embed.ts` | Deterministic 64-D bag-of-words sketch used both at index time and at query time. |
| `src/data/products.ts` | 100 sample product records. |
| `vite.config.ts` | Wires up `vite-plugin-wasm` + `vite-plugin-top-level-await` so the wasm-bindgen bundle loads cleanly. |

## VelesDB WASM API used

```ts
import init, { VectorStore } from "@wiscale/velesdb-wasm";

await init();
const store = new VectorStore(64, "cosine");
store.insert(1n, new Float32Array([...]));   // BigInt id, Float32Array vector
const results = store.search(new Float32Array([...]), 8);
// results: Array<[bigint, number]>  -> [[id, score], ...]
```

See [`crates/velesdb-wasm/src/lib.rs`](../../crates/velesdb-wasm/src/lib.rs)
for the full API surface (graph store, `VelesQL`, persistence, etc.).

## Build for production

```bash
npm run build      # outputs to dist/
npm run preview    # serves the production build locally
```

The built bundle is fully static and can be deployed to any static host
(GitHub Pages, Netlify, Cloudflare Pages, S3 + CloudFront…).
