// src/graph-and-memory.js — knowledge graph, agent memory and events.
//
// Raw `invoke` again, so the only dependency is @tauri-apps/api.
//
// Prerequisites: the capability from ../../quickstart, and the Rust command
// `seed_graph_nodes` from ../src-tauri/src/velesdb_setup.rs registered in your
// invoke_handler. See ../README.md.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const GRAPH = 'catalog_graph';
const DIM = 8;

/** Appends a line to #out and to the console. */
function log(line) {
  console.log(line);
  const out = document.getElementById('out');
  if (out) out.textContent += (out.textContent ? '\n' : '') + line;
}

/** Renders a rejection: every plugin error is a { message, code } object. */
function describe(e) {
  if (e && typeof e === 'object' && 'code' in e) return `[${e.code}] ${e.message}`;
  return String(e);
}

/**
 * Stand-in for a real embedding model.
 *
 * The plugin stores and searches vectors; producing them is your app's job.
 * A real app computes them in Rust and exposes its own command — see the RAG
 * demo. This deterministic hash keeps the example runnable unmodified.
 */
function fakeEmbedding(text, dimension = DIM) {
  const values = new Array(dimension).fill(0);
  for (let i = 0; i < text.length; i += 1) {
    values[i % dimension] += (text.charCodeAt(i) % 97) / 97;
  }
  const norm = Math.sqrt(values.reduce((acc, v) => acc + v * v, 0));
  return norm === 0 ? values : values.map((v) => v / norm);
}

// ---------------------------------------------------------------------------
// Events — subscribe BEFORE doing anything, or you miss collection-created
// ---------------------------------------------------------------------------

async function subscribeToEvents() {
  // Payload shape: { collection, operation, count? }
  // `count` is present for upsert / point deletion, absent for create / delete.
  const unlisten = await Promise.all([
    listen('velesdb://collection-created', (event) => {
      log(`event: created ${JSON.stringify(event.payload)}`);
    }),
    listen('velesdb://collection-updated', (event) => {
      log(`event: updated ${JSON.stringify(event.payload)}`);
    }),
    listen('velesdb://collection-deleted', (event) => {
      log(`event: deleted ${JSON.stringify(event.payload)}`);
    }),
  ]);

  // velesdb://operation-progress and velesdb://operation-complete are declared
  // in the plugin but no command emits them today — subscribing to them is a
  // silent no-op, not a bug in your listener.

  return () => unlisten.forEach((fn) => fn());
}

// ---------------------------------------------------------------------------
// Knowledge graph
// ---------------------------------------------------------------------------

async function knowledgeGraph() {
  log('--- knowledge graph ---');

  // A graph collection is its own family: `upsert` and `search` reject it with
  // INVALID_CONFIG ("Collection 'x' is not a vector collection").
  // Omit `dimension` for a graph with no node embeddings.
  await invoke('plugin:velesdb|create_graph_collection', {
    request: { name: GRAPH },
  });

  // Nodes must exist before any edge can reference them, and no IPC command
  // creates them. This is the app's own Rust command — see
  // ../src-tauri/src/velesdb_setup.rs.
  const seeded = await invoke('seed_graph_nodes', { collection: GRAPH });
  log(`seeded ${seeded} graph nodes from Rust`);

  // Now the frontend can drive the graph.
  await invoke('plugin:velesdb|add_edge', {
    request: {
      collection: GRAPH,
      id: 1,
      source: 100,
      target: 200,
      label: 'CONTAINS',
      properties: { position: 1 },
    },
  });

  // add_edges_batch takes the same edge objects in one round trip.
  await invoke('plugin:velesdb|add_edges_batch', {
    request: {
      collection: GRAPH,
      edges: [
        { id: 2, source: 100, target: 300, label: 'CONTAINS', properties: { position: 2 } },
        { id: 3, source: 200, target: 300, label: 'REFERENCES', properties: { weight: 0.8 } },
      ],
    },
  });

  // Filter by label, source, or target — all three fields are optional.
  const contains = await invoke('plugin:velesdb|get_edges', {
    request: { collection: GRAPH, label: 'CONTAINS' },
  });
  log(`edges labelled CONTAINS: ${JSON.stringify(contains)}`);

  // Traversal. algorithm: 'bfs' (default) or 'dfs'; relTypes filters the edge
  // labels that may be followed; maxDepth defaults to 3 and limit to 100.
  const reachable = await invoke('plugin:velesdb|traverse_graph', {
    request: {
      collection: GRAPH,
      source: 100,
      algorithm: 'bfs',
      maxDepth: 2,
      limit: 50,
    },
  });
  log(`reachable from node 100: ${JSON.stringify(reachable)}`);
  // Each entry is { targetId, depth, path }, where `path` is the accumulated
  // traversal path recorded by the engine.

  const degree = await invoke('plugin:velesdb|get_node_degree', {
    request: { collection: GRAPH, nodeId: 100 },
  });
  log(`degree of node 100: ${JSON.stringify(degree)}`);
}

// ---------------------------------------------------------------------------
// Agent memory — three independent families
// ---------------------------------------------------------------------------

async function agentMemory() {
  log('\n--- agent memory ---');

  // Semantic: facts the agent knows.
  const facts = [
    [1, 'The user prefers dark roast coffee.'],
    [2, 'The user is allergic to penicillin.'],
    [3, 'The project deadline is the end of March.'],
  ];
  for (const [id, content] of facts) {
    await invoke('plugin:velesdb|semantic_store', {
      request: { id, content, embedding: fakeEmbedding(content) },
    });
  }
  const recalled = await invoke('plugin:velesdb|semantic_query', {
    request: { embedding: fakeEmbedding('coffee preference'), topK: 2 },
  });
  log(`semantic_query -> ${JSON.stringify(recalled)}`);
  // Each result is { id, score, content }.

  // Episodic: what happened, and when. Timestamps are epoch SECONDS.
  const now = Math.floor(Date.now() / 1000);
  const episodes = [
    [10, 'User opened the project dashboard.', now - 600],
    [11, 'User exported the quarterly report.', now - 120],
  ];
  for (const [eventId, content, timestamp] of episodes) {
    await invoke('plugin:velesdb|episodic_record', {
      request: { eventId, content, timestamp, embedding: fakeEmbedding(content) },
    });
  }
  const recent = await invoke('plugin:velesdb|episodic_recent', {
    request: { limit: 5 },
  });
  log(`episodic_recent -> ${JSON.stringify(recent)}`);
  // Each result is { id, content, timestamp }.

  // Procedural: how to do something, with a confidence that can be reinforced.
  await invoke('plugin:velesdb|procedural_learn', {
    request: {
      procedureId: 20,
      name: 'export quarterly report',
      steps: ['open dashboard', 'select quarter', 'click export', 'choose PDF'],
      embedding: fakeEmbedding('export quarterly report'),
      confidence: 0.9,
    },
  });
  const procedures = await invoke('plugin:velesdb|procedural_recall', {
    request: { embedding: fakeEmbedding('how do I export the report'), topK: 3 },
  });
  log(`procedural_recall -> ${JSON.stringify(procedures)}`);
}

// ---------------------------------------------------------------------------
// The rejections worth recognising
// ---------------------------------------------------------------------------

async function expectedFailures() {
  log('\n--- expected rejections ---');

  // A graph collection is not a vector collection.
  try {
    await invoke('plugin:velesdb|search', {
      request: { collection: GRAPH, vector: [1, 0, 0, 0], topK: 1 },
    });
    log('UNEXPECTED: searching a graph collection should have been rejected');
  } catch (e) {
    log(`searching a graph collection: ${describe(e)}`);
  }

  // `query` takes no `collection` field: the target comes from the FROM clause
  // inside the VelesQL string, and any extra field is silently dropped.
  try {
    await invoke('plugin:velesdb|query', {
      request: { query: 'SELEC * FROM whatever' },
    });
    log('UNEXPECTED: a syntax error should have been rejected');
  } catch (e) {
    log(`malformed VelesQL: ${describe(e)}`);
  }
}

// ---------------------------------------------------------------------------

async function main() {
  const stop = await subscribeToEvents();
  try {
    await knowledgeGraph();
    await agentMemory();
    await expectedFailures();
  } finally {
    stop();
  }
}

main().catch((e) => {
  log(`failed: ${describe(e)}`);
  console.error(e);
});
