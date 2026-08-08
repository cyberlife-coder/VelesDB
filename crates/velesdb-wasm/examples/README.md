# velesdb-wasm examples

Five complete, runnable pages plus one Node script. No bundler, no framework,
no build step when you use the published package.

## Setup — once, for every browser example

```bash
cd crates/velesdb-wasm/examples
npm install
./serve.sh
```

`serve.sh` roots the static server at the crate directory (the parent of
`examples/`), so the URLs carry an `/examples/` segment:

| URL | Example |
|---|---|
| <http://localhost:8080/examples/01-quickstart/> | [`01-quickstart`](./01-quickstart) |
| <http://localhost:8080/examples/02-payload-filter/> | [`02-payload-filter`](./02-payload-filter) |
| <http://localhost:8080/examples/03-indexeddb-persistence/> | [`03-indexeddb-persistence`](./03-indexeddb-persistence) |
| <http://localhost:8080/examples/04-agent-memory/> | [`04-agent-memory`](./04-agent-memory) |
| <http://localhost:8080/examples/05-velesql/> | [`05-velesql`](./05-velesql) |

WebAssembly cannot be loaded from a `file://` page — opening `index.html`
directly by double-clicking it fails with `TypeError: Failed to fetch`. The
static server is not optional.

### Running against a local build instead of the published package

```bash
cargo install wasm-pack
wasm-pack build crates/velesdb-wasm --target web --release   # writes crates/velesdb-wasm/pkg/
```

Every example loads through [`loader.js`](./loader.js), which tries the
published package first and falls back to `../pkg/` automatically. Nothing to
edit either way — and serving from the crate directory is what keeps both paths
reachable.

## Index

| Example | What it shows | API surface |
|---|---|---|
| [`01-quickstart`](./01-quickstart) | The README's "first success in 60 seconds": three vectors in, two nearest out, with the exact expected numbers. | `new VectorStore(dim, metric)`, `insert`, `search`, `len` / `dimension` / `storage_mode` getters |
| [`02-payload-filter`](./02-payload-filter) | Attaching JSON payloads and narrowing a search with a metadata filter — the supported replacement for the broken `SemanticMemory.query()`. | `insert_with_payload`, `search_with_filter`, `get` |
| [`03-indexeddb-persistence`](./03-indexeddb-persistence) | Surviving a reload: save the index to IndexedDB, load it back, export/import the same bytes by hand. | `save`, `VectorStore.load`, `export_to_bytes`, `VectorStore.import_from_bytes`, `VectorStore.delete_database` |
| [`04-agent-memory`](./04-agent-memory) | Durable, explainable agent memory in the tab: store facts, recall them by meaning, link them, then ask why. | `MemoryService` — `remember`, `recall`, `relate`, `why`, `forget` |
| [`05-velesql`](./05-velesql) | VelesQL in the browser: DDL, insert, projected select — and what the WASM build rejects. | `WasmDatabase`, `executeQuery`, `QueryResult` (`kind`, `rowCount`, `rowsJson`) |
| [`node/`](./node) | The same quickstart from Node instead of a browser, using a `--target nodejs` build. | identical `VectorStore` API, CommonJS entry point |

## Two rules that will save you an hour

**1. Ids go in as `BigInt`, come back as numbers.**

```js
store.insert(1n, new Float32Array([1, 0, 0]));   // BigInt in — a plain 1 throws
const [[id, score]] = store.search(query, 1);    // id is a plain number out
```

The input side is a `wasm-bindgen` `u64` parameter, so JavaScript must hand it
a `BigInt`. The output side goes through `serde-wasm-bindgen`, which emits
`u64` as a JS number. Keep ids below `Number.MAX_SAFE_INTEGER`.

**2. Nothing works before `await init()` resolves.**

Constructing a class first gives you `null pointer passed to rust` or
`undefined is not a function`. Every example awaits the loader before touching
the API.

## Errors

Some paths throw a real `Error` carrying a machine-readable `code`
(`VELES-004` on a dimension mismatch); others still throw a bare string. Always
coerce with `String(e)` before displaying — every example does.

## Known limits worth remembering here

- **Brute-force search only.** No HNSW graph in the WASM build; search is O(n).
  `search_with_quality()` accepts the same quality strings as the Python and
  Server SDKs, but every mode returns identical results.
- **~100 K vectors** is the practical ceiling, set by browser RAM.
- **`MemoryService` is in-memory only.** There is no filesystem; nothing
  persists unless you persist it yourself.
- **`SemanticMemory.query()` has been broken since 4.0.0 (still present)** — it throws
  `Invalid search results` on every call. Example 02 shows the supported
  alternative; example 04 uses `MemoryService`.

## Going further

- [JavaScript API](../../../docs/guides/WASM_API.md)
- [Persistence and binary format](../../../docs/guides/WASM_PERSISTENCE.md)
- [VelesQL in the browser](../../../docs/guides/WASM_VELESQL.md)
- [Bundle size optimization](../../../docs/wasm/bundle-optimization.md)
