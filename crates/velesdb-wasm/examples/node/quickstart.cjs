#!/usr/bin/env node
// quickstart.cjs — the browser quickstart, from Node.
//
// Requires a Node-target build (the published npm package is a web-target
// build and cannot be loaded from Node without hand-feeding the .wasm bytes):
//
//   wasm-pack build crates/velesdb-wasm --target nodejs --release --out-dir pkg-node
//   node crates/velesdb-wasm/examples/node/quickstart.cjs
//
// CommonJS on purpose: the --target nodejs build is CommonJS, and the parent
// package.json declares "type": "module", which would otherwise make a plain
// .js file an ES module. See ./README.md.

'use strict';

const path = require('node:path');

const PKG = path.join(__dirname, '..', '..', 'pkg-node', 'velesdb_wasm.js');

let VectorStore;
try {
  // No init() on this target: the module loads its .wasm synchronously.
  ({ VectorStore } = require(PKG));
} catch (e) {
  console.error(`Could not load ${PKG}`);
  console.error(String(e));
  console.error('');
  console.error('Build it first:');
  console.error('  wasm-pack build crates/velesdb-wasm --target nodejs --release --out-dir pkg-node');
  process.exit(1);
}

const store = new VectorStore(3, 'cosine');

// Ids are u64 on the Rust side: JavaScript must pass a BigInt (1n, not 1).
store.insert_with_payload(1n, new Float32Array([1.0, 0.0, 0.0]), { title: 'north', category: 'docs' });
store.insert_with_payload(2n, new Float32Array([0.0, 1.0, 0.0]), { title: 'east', category: 'blog' });
store.insert_with_payload(3n, new Float32Array([0.9, 0.1, 0.0]), { title: 'north-ish', category: 'blog' });

// len / dimension / storage_mode are getters, not methods.
console.log(`${store.len} vectors, dimension ${store.dimension}, storage_mode ${store.storage_mode}`);

// search() returns [[id, score], ...]; ids come back as plain numbers.
const query = new Float32Array([1.0, 0.0, 0.0]);
for (const [id, score] of store.search(query, 2)) {
  console.log(`#${id}  score ${score.toFixed(3)}`);
}

// search_with_filter() returns [{id, score, payload}] and keeps the payload.
console.log('filtered (category = "docs"):');
for (const hit of store.search_with_filter(query, 2, {
  condition: { type: 'eq', field: 'category', value: 'docs' },
})) {
  console.log(`  #${hit.id}  score ${hit.score.toFixed(3)}  ${JSON.stringify(hit.payload)}`);
}

// The index serialises to a self-contained VELS buffer — write it to disk,
// ship it to a browser, reload it later with VectorStore.import_from_bytes().
const bytes = store.export_to_bytes();
console.log(`exported ${bytes.length} bytes`);
