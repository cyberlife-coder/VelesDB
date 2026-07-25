// src/main.js — the frontend half of the quickstart.
//
// Raw `invoke`, so the only dependency is @tauri-apps/api, which every Tauri 2
// project already has. The typed wrapper is optional:
//     npm install @wiscale/tauri-plugin-velesdb
//     import { createCollection, upsert, search } from '@wiscale/tauri-plugin-velesdb';
// It calls exactly these command names underneath.

import { invoke } from '@tauri-apps/api/core';

const out = document.getElementById('out');

/** Appends a line to the page and to the console. */
function log(line) {
  console.log(line);
  if (out) out.textContent += (out.textContent ? '\n' : '') + line;
}

async function velesdbSmokeTest() {
  // ---- 1. Create the collection ------------------------------------------
  // Dimension and metric are immutable after creation. If your embedding model
  // changes, create a new collection and reindex.
  //
  // Note the shape: every request is wrapped in a single `request` argument,
  // and its fields are camelCase (topK, storageMode) — the REST server uses
  // snake_case for the same engine.
  await invoke('plugin:velesdb|create_collection', {
    request: { name: 'demo', dimension: 4, metric: 'cosine' },
  });

  // ---- 2. Upsert ----------------------------------------------------------
  // Every request carries its own `collection` field: there is no URL path in
  // IPC, so the collection the REST API takes from /collections/{name}/... is
  // a body field here.
  const inserted = await invoke('plugin:velesdb|upsert', {
    request: {
      collection: 'demo',
      points: [
        { id: 1, vector: [1, 0, 0, 0], payload: { title: 'north' } },
        { id: 2, vector: [0, 1, 0, 0], payload: { title: 'east' } },
      ],
    },
  });
  log(`inserted: ${inserted}`);

  // ---- 3. Search ----------------------------------------------------------
  // `topK`, not `top_k`. The response is { results, timingMs }.
  const hits = await invoke('plugin:velesdb|search', {
    request: { collection: 'demo', vector: [1, 0, 0, 0], topK: 2 },
  });
  log(JSON.stringify(hits, null, 2));

  // With the cosine metric the score is a similarity: 1.0 identical, 0.0
  // orthogonal. Success is id 1 ranked first with a score close to 1.0.
}

velesdbSmokeTest().catch((e) => {
  // Rejections are { message, code } objects, not plain strings.
  //   VELES-002 -> collection not found (did step 1 run and succeed?)
  //   VELES-001 -> collection already exists (left over from a previous run)
  //   VELES-004 -> vector dimension mismatch
  // A permission error that fires before reaching the plugin means the
  // capability file is missing or does not cover this window.
  const code = e && typeof e === 'object' && 'code' in e ? e.code : '(no code)';
  const message = e && typeof e === 'object' && 'message' in e ? e.message : String(e);
  log(`velesdb failed [${code}]: ${message}`);
  console.error('velesdb failed:', e);
});
