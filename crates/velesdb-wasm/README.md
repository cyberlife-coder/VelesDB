# velesdb-wasm

> Vector search, knowledge graph and agent memory running entirely inside the browser — no server, no network call.

[![npm](https://img.shields.io/npm/v/@wiscale/velesdb-wasm)](https://www.npmjs.com/package/@wiscale/velesdb-wasm)
[![License](https://img.shields.io/badge/license-VelesDB_Core_1.0-blue)](LICENSE)

## Objective

Semantic search normally means shipping user data to a server: an embedding
endpoint, a vector database, a round-trip per keystroke. That is a latency
budget, an infrastructure bill, and a privacy exposure — three costs for one
feature.

`velesdb-wasm` compiles the VelesDB engine to WebAssembly so the index lives in
the tab. Vectors are inserted, filtered, fused and ranked locally, persisted to
IndexedDB, and never leave the device. If your data can leave the device and
you have a backend anyway, use the REST server instead — this crate exists for
the cases where it cannot.

## Use cases

- A documentation site whose search box ranks pages semantically with no search
  backend to operate.
- An offline-first PWA that keeps working in a plane and reloads its index from
  IndexedDB on startup.
- A health or legal app whose regulator forbids the corpus from reaching a
  server, even transiently.
- An Electron or Tauri desktop app that wants retrieval without bundling and
  supervising a database process.
- An AI agent running in the browser that needs durable, explainable memory —
  `remember` / `recall` / `relate` / `why` — with no backend.

## Prerequisites

| Requirement | Minimum version | Note |
|---|---|---|
| Browser | Any with WebAssembly + ES modules + `BigInt` | IndexedDB is additionally required for `save()` / `load()`. |
| Node.js | 20+ | Only to install the package and serve files. Verified on Node 26.3.0. |
| Rust | 1.90 | Only to build from source (`rust-toolchain.toml`). |
| wasm-pack | 0.12+ | Only to build from source. |

Nothing else: no embedding model, no server, no build step if you use the
published package.

## Installation

```bash
npm install @wiscale/velesdb-wasm
```

The published package is an ES module (`wasm-pack --target web` build)
containing `velesdb_wasm.js`, `velesdb_wasm.d.ts` and `velesdb_wasm_bg.wasm`.

This crate is **not published on crates.io** (`publish = false` in
`Cargo.toml`) — npm is the only distribution channel. To build it yourself:

```bash
cargo install wasm-pack
wasm-pack build crates/velesdb-wasm --target web --release   # browser
wasm-pack build crates/velesdb-wasm --target nodejs --release # Node
```

## First success in 60 seconds

Three files, no bundler, no config.

```bash
mkdir velesdb-hello && cd velesdb-hello
npm install @wiscale/velesdb-wasm@4.0.0
```

Create `index.html`:

```html
<!doctype html>
<meta charset="utf-8">
<title>VelesDB WASM</title>
<pre id="out">loading…</pre>
<script type="module">
  import init, { VectorStore } from './node_modules/@wiscale/velesdb-wasm/velesdb_wasm.js';

  await init();

  const store = new VectorStore(3, 'cosine');
  store.insert(1n, new Float32Array([1.0, 0.0, 0.0]));
  store.insert(2n, new Float32Array([0.0, 1.0, 0.0]));
  store.insert(3n, new Float32Array([0.9, 0.1, 0.0]));

  const lines = [
    `${store.len} vectors, dimension ${store.dimension}, storage_mode ${store.storage_mode}`,
  ];
  for (const [id, score] of store.search(new Float32Array([1.0, 0.0, 0.0]), 2)) {
    lines.push(`#${id}  score ${score.toFixed(3)}`);
  }
  document.getElementById('out').textContent = lines.join('\n');
</script>
```

Serve it (WebAssembly cannot be loaded from a `file://` page):

```bash
python3 -m http.server 8080
```

Open <http://localhost:8080>. The page must show exactly:

```
3 vectors, dimension 3, storage_mode full
#1  score 1.000
#3  score 0.994
```

Vector 1 is identical to the query, so its cosine similarity is `1.000`;
vector 3 is close, at `0.994`; vector 2 is orthogonal and is cut by `k = 2`.
Anything else — an empty `<pre>`, a `loading…` that never changes, a console
error — is a failure, not a slow start; see [Troubleshooting](#troubleshooting).

## Examples

- [`examples/wasm-browser-demo`](../../examples/wasm-browser-demo) — interactive
  single-page demo, insert and search random vectors, latency shown live.
- [`examples/react-wasm-search`](../../examples/react-wasm-search) — React +
  Vite app, 100 products, search on every keystroke.

## API

The published package ships its own TypeScript declarations
(`node_modules/@wiscale/velesdb-wasm/velesdb_wasm.d.ts`); that file is the
authoritative signature list and your editor will use it automatically. This
crate is not on docs.rs, so the narrative documentation lives here:

| Guide | Contents |
|---|---|
| [JavaScript API](../../docs/guides/WASM_API.md) | `VectorStore` (construction, metrics, insert paths, the search family, payload filters, storage modes), `GraphStore`, `MemoryService`, `SparseIndex`, `WasmDatabase`, and the BigInt/number marshalling rules. |
| [Persistence and binary format](../../docs/guides/WASM_PERSISTENCE.md) | IndexedDB `save`/`load`/`delete_database`, byte-level export/import, the `VELS` v2 format, graph persistence, performance figures and how to reproduce them. |
| [VelesQL in the browser](../../docs/guides/WASM_VELESQL.md) | Parsing, `executeQuery`, `EXPLAIN`, the full WASM-vs-REST feature matrix, every rejected shape with its exact error message, and how to migrate a query to the REST server. |
| [Bundle size optimization](../../docs/wasm/bundle-optimization.md) | Keeping the `.wasm` payload small. |

One rule to internalize before writing any code: **ids go in as `BigInt`
(`1n`) and come back from `search()` as plain numbers**. The input side is a
`wasm-bindgen` `u64` parameter; the output side goes through
`serde-wasm-bindgen`, which emits `u64` as a JS number by default. Keep ids
below `Number.MAX_SAFE_INTEGER`.

## Known limits

- **Brute-force search only.** There is no HNSW graph in the WASM build; search
  is O(n) over every vector. `search_with_quality()` accepts the same quality
  strings as the Python and Server SDKs, but all modes return identical
  results — the parameter exists for API parity.
- **~100 K vectors is the practical ceiling**, set by browser RAM. Beyond that,
  move to the REST server.
- **Single collection per query.** Cross-collection `MATCH` (`@collection`)
  needs Database-level routing and is server-only. `MATCH` is limited to 1–2
  hops.
- **No filesystem.** The agent-memory wedge (`MemoryService`) is in-memory
  only; nothing persists unless you persist it yourself.
- **No quantizer training.** `TRAIN QUANTIZER`, Product Quantization and RaBitQ
  need `rayon`/`ndarray`/`persistence`, which are compiled out for
  `wasm32-unknown-unknown`. `sq8` and `binary` storage modes still work.
- **`SemanticMemory.query()` is broken in 4.0.0** — it throws
  `Invalid search results` on every call (`src/agent.rs` calls `as_string()` on
  an array-valued `JsValue`). Use `MemoryService`, or a `VectorStore` with
  `insert_with_payload` + `search_with_filter`.
- **Errors are not uniformly structured.** Some paths throw a real `Error` with
  a machine-readable `code` (`VELES-004` on a dimension mismatch); others still
  throw a bare string. Always coerce with `String(e)` before displaying.

The full list of rejected VelesQL shapes, with their exact messages, is in the
[VelesQL guide](../../docs/guides/WASM_VELESQL.md#what-is-rejected-and-how).

## Compatibility

| Environment | Status | Note |
|---|---|---|
| Chromium browsers (Chrome, Edge, Brave, Arc) | Supported | Primary target. |
| Firefox | Supported | |
| Safari / iOS WebKit | Supported | Needs a version with WebAssembly SIMD; older WebKit may reject the binary. |
| Electron / Tauri | Supported | Same engine as the browser target. |
| Node.js 20+ | Supported with one adjustment | The `--target web` build resolves the `.wasm` via `fetch`, which has no `file://` support. Pass the bytes yourself — see [Loading the module](../../docs/guides/WASM_API.md#loading-the-module) — or build with `--target nodejs`. Verified on Node 26.3.0. |
| Vite / webpack / Rollup | Supported | The React example uses `vite-plugin-wasm` + top-level-await; see [`examples/react-wasm-search/vite.config.ts`](../../examples/react-wasm-search/vite.config.ts). |
| Web Worker | Supported | Import and `init()` inside the worker; a `VectorStore` cannot be transferred across the worker boundary. |
| Deno / Bun | Untested | No CI coverage. |

The release profile runs `wasm-opt` with `--enable-simd`,
`--enable-bulk-memory` and `--enable-nontrapping-float-to-int`
(`Cargo.toml`, `[package.metadata.wasm-pack.profile.release]`), so the emitted
binary can use those WebAssembly features. Any engine predating them will fail
to instantiate the module.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `TypeError: Failed to fetch` / CORS error on `init()` | The page was opened as `file://`. | Serve over HTTP: `python3 -m http.server 8080`. |
| `null pointer passed to rust` or `undefined is not a function` on the first call | A class was constructed before `await init()` resolved. | Await `init()` first; in a bundler, enable top-level await or wrap the app entry in an async bootstrap. |
| `Error: [VELES-004] Vector dimension mismatch: expected N, got M` | The query length differs from `store.dimension`. | Embed the query with the same model and dimension used at index time. |
| `Unknown metric. Use: cosine, euclidean, l2, dot, dotproduct, inner, ip, hamming, jaccard` | Unsupported metric string in the constructor. | Use one of the listed names; they are case-insensitive. |
| `Invalid data: wrong magic number` on `import_from_bytes` | The buffer is not a `VELS` export (truncated, or another format). | Re-export from a `VectorStore`; see the [binary format](../../docs/guides/WASM_PERSISTENCE.md#binary-format-vels-v2). |
| `Invalid search results` | `SemanticMemory.query()` — a known 4.0.0 defect. | Use `MemoryService` instead. |
| Console warning about `application/wasm` MIME type | The static server does not label `.wasm`. | Harmless: the loader falls back to `WebAssembly.instantiate`. Configure the MIME type to keep streaming compilation. |

## License

Licensed under the [VelesDB Core License 1.0](LICENSE) (source-available).
`velesdb-wasm` compiles the VelesDB engine to WebAssembly, so the published
artifact embeds the engine and is governed by the Core License.

---

`velesdb-wasm v4.1.0` · Last updated: 2026-07-25 · Applies to: velesdb-core 4.1.0 · [Report a docs error](https://github.com/cyberlife-coder/velesdb/issues)
