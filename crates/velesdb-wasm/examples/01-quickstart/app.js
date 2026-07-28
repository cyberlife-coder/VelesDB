// 01-quickstart — the README's "first success in 60 seconds".
//
// Run it: from crates/velesdb-wasm/examples, `npm install && ./serve.sh`, then
// open http://localhost:8080/examples/01-quickstart/

import { loadVelesDb, log, describeError } from '../loader.js';

const out = document.getElementById('out');

async function main() {
  // Nothing may touch the API before this resolves: constructing a class
  // earlier throws `null pointer passed to rust`.
  const { VectorStore } = await loadVelesDb();
  out.textContent = '';

  // 3-dimensional store, cosine similarity. Accepted metric names are
  // cosine, euclidean, l2, dot, dotproduct, inner, ip, hamming, jaccard
  // (case-insensitive).
  const store = new VectorStore(3, 'cosine');

  // Ids are u64 on the Rust side, so JavaScript must pass a BigInt: `1n`,
  // not `1`. Vectors are Float32Array of exactly `store.dimension` values.
  store.insert(1n, new Float32Array([1.0, 0.0, 0.0]));
  store.insert(2n, new Float32Array([0.0, 1.0, 0.0]));
  store.insert(3n, new Float32Array([0.9, 0.1, 0.0]));

  // len / dimension / storage_mode are getters, not methods — no parentheses.
  log(out, `${store.len} vectors, dimension ${store.dimension}, storage_mode ${store.storage_mode}`);

  // search() returns [[id, score], ...] sorted best-first. Ids come back as
  // plain numbers (serde-wasm-bindgen emits u64 as a JS number), which is the
  // asymmetry to remember: BigInt in, number out.
  for (const [id, score] of store.search(new Float32Array([1.0, 0.0, 0.0]), 2)) {
    log(out, `#${id}  score ${score.toFixed(3)}`);
  }
}

main().catch((e) => {
  // Some paths throw a real Error with a `code` (VELES-004 on a dimension
  // mismatch); others throw a bare string. String(e) handles both.
  out.textContent = `FAILED: ${describeError(e)}`;
});
