// 04-agent-memory — remember / recall / relate / why / forget.
//
// Run it: from crates/velesdb-wasm/examples, `npm install && ./serve.sh`, then
// open http://localhost:8080/examples/04-agent-memory/

import { loadVelesDb, log, describeError } from '../loader.js';

const out = document.getElementById('out');

async function main() {
  const { MemoryService } = await loadVelesDb();
  out.textContent = '';

  // The constructor takes the embedding dimension. The bundled HashEmbedder
  // is deterministic and offline — no model download, no network call.
  const memory = new MemoryService(64);

  // ---- 1. remember(fact, links, metadata, ttlSeconds) ---------------------
  // `links` is an array of {target, relation} edges to memories that already
  // exist; pass `null` for none. `metadata` is a plain object or `null`.
  // `ttlSeconds` makes the fact expire; omit it (or pass 0) for a permanent
  // memory. The call resolves to the new memory's id, as a decimal string.
  const pgId = memory.remember(
    'The team chose PostgreSQL for the billing service.',
    null,
    { project: 'billing', kind: 'decision' },
  );
  const whyId = memory.remember(
    'PostgreSQL was chosen because billing needs multi-row transactions.',
    null,
    { project: 'billing', kind: 'rationale' },
  );
  const opsId = memory.remember(
    'The ops team already runs a managed PostgreSQL cluster.',
    null,
    { project: 'billing', kind: 'constraint' },
  );
  const noiseId = memory.remember(
    'The marketing site is a static bundle on a CDN.',
    null,
    { project: 'marketing', kind: 'note' },
  );

  log(out, `remembered 4 facts: ${[pgId, whyId, opsId, noiseId].join(', ')}`);
  log(out, '(ids are decimal STRINGS, not numbers — that is the id contract)');
  log(out, '');

  // ---- 2. relate(from, to, relation) --------------------------------------
  // Edges are what `why()` walks. Without them, an explanation is a single
  // node with no supporting evidence.
  memory.relate(pgId, whyId, 'because');
  memory.relate(pgId, opsId, 'supported_by');
  log(out, `linked ${pgId} -> ${whyId} (because) and ${pgId} -> ${opsId} (supported_by)`);
  log(out, '');

  // ---- 3. recall(query, k, filter) ----------------------------------------
  // Semantic recall. `k` defaults to 10 and is capped internally; `filter` is
  // an exact-match metadata object, or null.
  log(out, 'recall("which database did we pick?", 3, null):');
  for (const hit of memory.recall('which database did we pick?', 3, null)) {
    log(out, `  [${hit.id}] score ${hit.score.toFixed(3)}  ${hit.content}`);
  }
  log(out, '');

  // Same query, narrowed to one project. The marketing note cannot surface.
  log(out, 'recall(..., filter = {project: "billing"}):');
  for (const hit of memory.recall('which database did we pick?', 3, { project: 'billing' })) {
    log(out, `  [${hit.id}] ${hit.content}`);
  }
  log(out, '');

  // ---- 4. why(decision, maxHops, filter) ----------------------------------
  // Returns {nodes, edges}: the best-matching memory plus the connected
  // subgraph within `maxHops` (default 2, capped at 10). This is the
  // explainability trail — the evidence, not a generated summary.
  log(out, 'why("why PostgreSQL for billing?", 2, null):');
  const explanation = memory.why('why PostgreSQL for billing?', 2, null);
  for (const node of explanation.nodes) {
    log(out, `  hop ${node.hop}  [${node.id}]  ${node.content}`);
  }
  for (const edge of explanation.edges) {
    log(out, `  edge ${edge.from} --${edge.relation}--> ${edge.to}`);
  }
  log(out, '');

  // ---- 5. forget(id) ------------------------------------------------------
  // Returns whether a memory actually existed under that id. `false` means
  // nothing was stored there (stale id or typo), not a second success.
  log(out, `forget(${noiseId}) -> ${memory.forget(noiseId)}`);
  log(out, `forget(${noiseId}) again -> ${memory.forget(noiseId)}  (already gone)`);
  log(out, '');

  log(out, 'recall("static site", 3, null) after the deletion:');
  const afterDelete = memory.recall('static site', 3, null);
  if (afterDelete.length === 0) {
    log(out, '  (nothing — the marketing note is gone)');
  } else {
    for (const hit of afterDelete) log(out, `  [${hit.id}] ${hit.content}`);
  }
}

main().catch((e) => {
  out.textContent = `FAILED: ${describeError(e)}`;
});
