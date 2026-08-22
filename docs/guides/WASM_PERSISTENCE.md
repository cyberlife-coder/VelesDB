# VelesDB WASM — persistence, binary format, and performance

Companion to [`crates/velesdb-wasm/README.md`](../../crates/velesdb-wasm/README.md).
Covers how a browser-side store survives a page reload: IndexedDB persistence,
the `VELS` binary format, graph persistence, and the indicative performance
figures.

## IndexedDB persistence

`VectorStore` ships three async methods that handle IndexedDB end to end — no
manual transaction plumbing:

```javascript
import init, { VectorStore } from '@wiscale/velesdb-wasm';

await init();

const store = new VectorStore(768, 'cosine');
store.insert(1n, new Float32Array(768).fill(0.1));
store.insert(2n, new Float32Array(768).fill(0.2));

await store.save('my-vectors-db');            // create or overwrite
const restored = await VectorStore.load('my-vectors-db');
console.log(restored.len);                    // 2

await VectorStore.delete_database('my-vectors-db');
```

| Method | Kind | Effect |
|---|---|---|
| `save(dbName)` | instance, async | Serializes the store and writes it to IndexedDB. |
| `VectorStore.load(dbName)` | static, async | Rebuilds a store from IndexedDB. |
| `VectorStore.delete_database(dbName)` | static, async | Drops the whole IndexedDB database. |

Internals worth knowing (`crates/velesdb-wasm/src/persistence.rs`): the whole
store is written as **one blob** under the key `data` in an object store named
`vectors`, inside a database named after the `dbName` argument. There is no
per-vector record, so `save()` cost is proportional to the whole store, not to
what changed — save at checkpoints, not on every insert.

These methods are browser APIs. In Node or a non-DOM runtime there is no
`indexedDB` global, so use the byte-level export/import below instead.

## Manual export / import

```javascript
const bytes = store.export_to_bytes();        // Uint8Array
// … persist it anywhere: localStorage (base64), a file download, OPFS, a POST …
const clone = VectorStore.import_from_bytes(bytes);
```

This path is synchronous and runtime-agnostic. It is also what `save()` uses
under the hood.

## Binary format (`VELS` v2)

Layout written by `export_to_bytes`, little-endian throughout
(`crates/velesdb-wasm/src/serialization.rs`):

| Field | Size | Description |
|---|---|---|
| Magic | 4 bytes | `"VELS"` |
| Version | 1 byte | `2` |
| Dimension | 4 bytes | `u32` |
| Metric | 1 byte | `0`=cosine, `1`=euclidean, `2`=dot, `3`=hamming, `4`=jaccard |
| Storage mode | 1 byte | `0`=full, `1`=sq8, `2`=binary, `3`=PQ, `4`=RaBitQ |
| Count | 8 bytes | `u64`, number of vectors |
| Ids | count × 8 bytes | `u64` each |
| `data` | length-prefixed blob | f32 vectors (full mode) |
| `data_sq8` | length-prefixed blob | SQ8 codes |
| `data_binary` | length-prefixed blob | binary codes |
| `sq8_mins` | length-prefixed blob | f32 dequantization minima |
| `sq8_scales` | length-prefixed blob | f32 dequantization scales |
| Payloads | count × blob | JSON bytes per vector, empty blob = no payload |

Each length-prefixed blob starts with its byte length as a `u64`.

**v1 is still readable.** The v1 header was 18 bytes (no storage-mode byte) and
carried only ids plus f32 vectors, i.e. full precision only; `import_from_bytes`
accepts it for legacy data. v2 additionally preserves the storage mode, the
quantized buffers and the payloads, and it fixed a v1 defect where exporting a
quantized store indexed the empty `data` buffer.

**The sparse index is not persisted** in either version — rebuild it after a
load with `sparse_insert`.

A truncated or foreign buffer is rejected loudly:

```javascript
VectorStore.import_from_bytes(new Uint8Array([1, 2, 3, 4, 5]));
// Error: Invalid data: wrong magic number
```

Historical reference verified against `@wiscale/velesdb-wasm@4.0.0`: a 3-dimension store
holding one vector exports to 87 bytes, magic `VELS`, version byte `2`, and
re-imports with `len === 1`.

## Graph persistence

`GraphStore` has its own IndexedDB layer, `GraphPersistence`, with an explicit
`init()` step and named graphs:

| Method | Effect |
|---|---|
| `init()` | Opens / upgrades the IndexedDB database. Call once, await it. |
| `save(graphName, store)` | Persists a `GraphStore` under a name. |
| `load(graphName)` | Rebuilds the `GraphStore`. |
| `list_graphs()` | Names currently stored. |
| `delete_graph(graphName)` | Drops one graph. |
| `get_metadata(graphName)` | Stored metadata without loading the graph. |

All of these are async.

## Performance

The figures below are **indicative** and carried over from the previous README
revision. They were not re-measured for this document; treat them as an order
of magnitude, not a guarantee, and re-run the benches on your own target
hardware before relying on them.

Serialization throughput, 10 000 vectors of 768 dimensions:

| Operation | Time | Throughput |
|---|---|---|
| `export_to_bytes` | ~7 ms | ~4479 MB/s |
| `import_from_bytes` | ~10 ms | ~2943 MB/s |

Typical in-browser latency:

| Operation | 768-D vectors | 10 000 vectors |
|---|---|---|
| Insert | ~1 µs | ~10 ms |
| Search | ~50 µs | ~5 ms |

Search is brute-force O(n) over every vector — there is no HNSW graph in the
WASM build — so search latency grows linearly with `store.len`.

### Reproducing the benches

Two harnesses live in `crates/velesdb-wasm/benches/`:

```bash
# Serialization benches (export / import), Node runner.
# Builds a nodejs-target package into crates/velesdb-wasm/pkg/ first,
# because the runner does require('../pkg/velesdb_wasm.js').
wasm-pack build crates/velesdb-wasm --target nodejs --release --out-dir pkg
node crates/velesdb-wasm/benches/run_persistence_bench.js

# IndexedDB save/load benches — need a real browser, and are #[ignore]d
# so they do not run in the normal test sweep.
wasm-pack test --headless --chrome crates/velesdb-wasm -- --ignored
```

The in-repo Node runner measures **1000 vectors at 128 dimensions**, a
different shape from the table above; expect different absolute numbers.

## Related

- [VelesDB WASM JavaScript API](WASM_API.md)
- [VelesQL in the browser](WASM_VELESQL.md)
- [Bundle size optimization](../wasm/bundle-optimization.md)

---

Last updated: 2026-08-13 · Applies to: velesdb-core 5.2.0
