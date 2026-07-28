// 02-payload-filter — payloads, metadata filters, and the error shape.
//
// This is also the supported replacement for `SemanticMemory.query()`, which
// is broken in 4.0.0 (it throws `Invalid search results` on every call):
// `insert_with_payload` + `search_with_filter` covers the same ground.
//
// Run it: from crates/velesdb-wasm/examples, `npm install && ./serve.sh`, then
// open http://localhost:8080/examples/02-payload-filter/

import { loadVelesDb, log, describeError } from '../loader.js';

const out = document.getElementById('out');

/** Pretty-prints one `{id, score, payload}` hit. */
function fmt(hit) {
  return `  #${hit.id}  score ${hit.score.toFixed(3)}  ${JSON.stringify(hit.payload)}`;
}

async function main() {
  const { VectorStore } = await loadVelesDb();
  out.textContent = '';

  const store = new VectorStore(3, 'cosine');

  // insert_with_payload(id, vector, payload). The payload is any JSON value;
  // pass `null` for none. Ids are BigInt, as everywhere else.
  store.insert_with_payload(1n, new Float32Array([1.0, 0.0, 0.0]), { title: 'Ownership', category: 'docs', year: 2021 });
  store.insert_with_payload(2n, new Float32Array([0.9, 0.1, 0.0]), { title: 'Lifetimes', category: 'blog', year: 2023 });
  store.insert_with_payload(3n, new Float32Array([0.8, 0.2, 0.0]), { title: 'Traits',    category: 'docs', year: 2024 });
  store.insert_with_payload(4n, new Float32Array([0.0, 1.0, 0.0]), { title: 'Vacuum',    category: 'docs', year: 2022 });

  const query = new Float32Array([1.0, 0.0, 0.0]);

  log(out, `${store.len} vectors indexed.`);
  log(out, '');

  // ---- 1. No filter -------------------------------------------------------
  // search() returns [[id, score], ...] — ids and scores only, no payload.
  log(out, 'search(query, 2) — nearest two, any category:');
  for (const [id, score] of store.search(query, 2)) {
    log(out, `  #${id}  score ${score.toFixed(3)}`);
  }
  log(out, '');

  // ---- 2. Equality filter -------------------------------------------------
  // search_with_filter(query, k, filter) returns [{id, score, payload}] — the
  // payload comes back, unlike plain search().
  //
  // The filter grammar is the same one the REST server accepts:
  //   { condition: { type: "eq", field: "...", value: ... } }
  // Field names support dot notation for nested payloads ("author.name").
  log(out, 'search_with_filter(query, 2, category = "docs"):');
  const docsOnly = store.search_with_filter(query, 2, {
    condition: { type: 'eq', field: 'category', value: 'docs' },
  });
  for (const hit of docsOnly) log(out, fmt(hit));
  log(out, '  (id 2 is nearer than id 3 but is a blog post, so it is excluded');
  log(out, '   and id 3 takes its place — the filter runs before the top-k cut)');
  log(out, '');

  // ---- 3. Range filter ----------------------------------------------------
  // Comparison types: eq, neq, gt, gte, lt, lte.
  log(out, 'search_with_filter(query, 3, year >= 2023):');
  for (const hit of store.search_with_filter(query, 3, {
    condition: { type: 'gte', field: 'year', value: 2023 },
  })) {
    log(out, fmt(hit));
  }
  log(out, '');

  // ---- 4. Compound filter -------------------------------------------------
  // "and" / "or" take a `conditions` array and nest arbitrarily.
  log(out, 'search_with_filter(query, 3, category = "docs" AND year >= 2022):');
  for (const hit of store.search_with_filter(query, 3, {
    condition: {
      type: 'and',
      conditions: [
        { type: 'eq', field: 'category', value: 'docs' },
        { type: 'gte', field: 'year', value: 2022 },
      ],
    },
  })) {
    log(out, fmt(hit));
  }
  log(out, '');

  // ---- 5. Point lookup ----------------------------------------------------
  // get(id) returns {id, vector, payload} or null. Still a BigInt id.
  log(out, 'get(3n):');
  log(out, `  ${JSON.stringify(store.get(3n))}`);
  log(out, '');

  // ---- 6. The failure you will actually hit -------------------------------
  // A query whose length differs from store.dimension is rejected with a
  // structured error carrying code VELES-004.
  log(out, 'search with a 2-dimensional query against a 3-dimensional store:');
  try {
    store.search(new Float32Array([1.0, 0.0]), 1);
    log(out, '  UNEXPECTED: that should have thrown.');
  } catch (e) {
    log(out, `  rejected as expected -> ${describeError(e)}`);
  }
}

main().catch((e) => {
  out.textContent = `FAILED: ${describeError(e)}`;
});
