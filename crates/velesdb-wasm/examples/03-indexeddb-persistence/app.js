// 03-indexeddb-persistence — save, load, export, import, delete.
//
// Run it: from crates/velesdb-wasm/examples, `npm install && ./serve.sh`, then
// open http://localhost:8080/examples/03-indexeddb-persistence/

import { loadVelesDb, log, describeError } from '../loader.js';

const out = document.getElementById('out');
const DB_NAME = 'velesdb-example-03';

/** Builds a small, deterministic index so every run is comparable. */
function buildStore(VectorStore) {
  const store = new VectorStore(3, 'cosine');
  store.insert_with_payload(1n, new Float32Array([1.0, 0.0, 0.0]), { title: 'north' });
  store.insert_with_payload(2n, new Float32Array([0.0, 1.0, 0.0]), { title: 'east' });
  store.insert_with_payload(3n, new Float32Array([0.9, 0.1, 0.0]), { title: 'north-ish' });
  store.insert_with_payload(4n, new Float32Array([0.0, 0.0, 1.0]), { title: 'up' });
  return store;
}

async function main() {
  const { VectorStore } = await loadVelesDb();
  out.textContent = '';

  // ---- 1. Try to load a previously saved index ----------------------------
  // VectorStore.load(dbName) is a static async method. It rejects when nothing
  // was ever saved under that name — that rejection is the "first run" signal,
  // not an error to fix.
  let store = null;
  try {
    store = await VectorStore.load(DB_NAME);
    log(out, `Loaded "${DB_NAME}" from IndexedDB: ${store.len} vectors, dimension ${store.dimension}.`);
    log(out, 'Nothing was re-inserted — this index survived the reload.');
  } catch (e) {
    log(out, `No saved index under "${DB_NAME}" (${describeError(e)}).`);
    log(out, 'Building one from scratch…');
    store = buildStore(VectorStore);
    await store.save(DB_NAME);
    log(out, `Built and saved ${store.len} vectors. Reload the page to load them back.`);
  }
  log(out, '');

  // ---- 2. The index works the same either way -----------------------------
  log(out, 'search([1, 0, 0], 2):');
  for (const [id, score] of store.search(new Float32Array([1.0, 0.0, 0.0]), 2)) {
    log(out, `  #${id}  score ${score.toFixed(3)}`);
  }
  log(out, '');

  // ---- 3. The same bytes, without IndexedDB -------------------------------
  // export_to_bytes() is synchronous and returns a Uint8Array in the VELS v2
  // format. Store it wherever you like; import_from_bytes() rebuilds the store.
  const bytes = store.export_to_bytes();
  log(out, `export_to_bytes(): ${bytes.length} bytes, magic "${String.fromCharCode(...bytes.slice(0, 4))}"`);

  const clone = VectorStore.import_from_bytes(bytes);
  log(out, `import_from_bytes(): ${clone.len} vectors, dimension ${clone.dimension}, storage_mode ${clone.storage_mode}`);

  const sameTop = clone.search(new Float32Array([1.0, 0.0, 0.0]), 1)[0];
  log(out, `round-trip top hit: #${sameTop[0]} score ${sameTop[1].toFixed(3)} (identical to the original)`);
  log(out, '');

  // ---- 4. A foreign buffer is rejected, not silently accepted -------------
  // Every export starts with the ASCII magic "VELS" followed by a version
  // byte. Anything else fails with "Invalid data: wrong magic number".
  log(out, 'import_from_bytes(a buffer that is not a VELS export):');
  try {
    VectorStore.import_from_bytes(new Uint8Array([0, 1, 2, 3, 4, 5, 6, 7]));
    log(out, '  UNEXPECTED: that should have thrown.');
  } catch (e) {
    log(out, `  rejected as expected -> ${describeError(e)}`);
  }
}

document.getElementById('run').addEventListener('click', () => {
  main().catch((e) => { out.textContent = `FAILED: ${describeError(e)}`; });
});

document.getElementById('reload').addEventListener('click', () => {
  window.location.reload();
});

document.getElementById('wipe').addEventListener('click', async () => {
  try {
    const { VectorStore } = await loadVelesDb();
    // Static async method: drops the whole IndexedDB database.
    await VectorStore.delete_database(DB_NAME);
    out.textContent = `Deleted "${DB_NAME}". Reload the page to start over.`;
  } catch (e) {
    out.textContent = `FAILED: ${describeError(e)}`;
  }
});

main().catch((e) => {
  out.textContent = `FAILED: ${describeError(e)}`;
});
