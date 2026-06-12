/**
 * Hybrid Query Example for the VelesDB TypeScript SDK
 *
 * End-to-end tour of the REST client against a running `velesdb-server`
 * (default: http://localhost:8080):
 *
 *   1. Create a collection
 *   2. Upsert documents with payloads
 *   3. Hybrid VelesQL query — vector NEAR + full-text MATCH + payload
 *      filter + ORDER BY similarity()
 *   4. Relation edges — relate() / getRelations() / unrelate()
 *   5. Agent memory — recordEvent / recallEvents / recallRecent +
 *      durable TTL via setTtlDurable()
 *
 * Run with a live server:  npx tsx examples/hybrid_queries.ts
 * Type-checked in CI via `npm run typecheck` (tsconfig.examples.json).
 */

import { VelesDB } from '../src';

const SERVER_URL = process.env.VELESDB_URL ?? 'http://localhost:8080';
const DIM = 128;

/**
 * Deterministic mock embedding for demo purposes — replace with the output
 * of a real embedding model (the vector length must equal the collection
 * dimension).
 */
function generateEmbedding(seed: number, dim: number = DIM): number[] {
  const embedding: number[] = [];
  for (let i = 0; i < dim; i++) {
    embedding.push(Math.sin(seed * 0.1 + i * 0.01));
  }
  const norm = Math.sqrt(embedding.reduce((sum, x) => sum + x * x, 0));
  return embedding.map((x) => x / norm);
}

/** 1 + 2 — create the collection and upsert documents with payloads. */
async function seedArticles(db: VelesDB): Promise<void> {
  console.log('\n=== 1. Create collection + upsert with payload ===');

  await db.createCollection('articles', { dimension: DIM, metric: 'cosine' });

  await db.upsertBatch('articles', [
    {
      id: 1,
      vector: generateEmbedding(1),
      payload: {
        title: 'Vector database internals',
        content: 'How a vector database builds HNSW graphs for fast search',
        category: 'tech',
      },
    },
    {
      id: 2,
      vector: generateEmbedding(2),
      payload: {
        title: 'Embedding models compared',
        content: 'Choosing an embedding model for your vector database',
        category: 'tech',
      },
    },
    {
      id: 3,
      vector: generateEmbedding(3),
      payload: {
        title: 'Sourdough basics',
        content: 'A starter guide to baking bread at home',
        category: 'food',
      },
    },
  ]);

  console.log('Upserted 3 articles');
}

/** 3 — hybrid VelesQL: NEAR + MATCH + filter + ORDER BY similarity(). */
async function hybridQuery(db: VelesDB): Promise<void> {
  console.log('\n=== 2. Hybrid VelesQL query ===');

  const velesql = `
    SELECT id, title, similarity() AS score
    FROM articles
    WHERE vector NEAR $q
      AND content MATCH 'vector database'
      AND category = 'tech'
    ORDER BY similarity() DESC
    LIMIT 5
  `;
  console.log('VelesQL:', velesql.trim());

  const response = await db.query('articles', velesql, {
    q: generateEmbedding(1),
  });

  if ('results' in response) {
    for (const row of response.results) {
      console.log(`  #${String(row.id)} ${String(row.title)} (score=${String(row.score)})`);
    }
    console.log(`Strategy: ${response.stats.strategy}, ${response.stats.executionTimeMs}ms`);
  }
}

/** 4 — typed relation edges between points. */
async function relationEdges(db: VelesDB): Promise<void> {
  console.log('\n=== 3. Relations: relate / getRelations / unrelate ===');

  // relate() returns the allocated edge id (number | string — u64-safe).
  const { edgeId } = await db.relate('articles', {
    source: 1,
    target: 2,
    relType: 'CITES',
    properties: { context: 'embedding section' },
  });
  console.log(`Created edge ${String(edgeId)}: 1 -[CITES]-> 2`);

  const { edges, count } = await db.getRelations('articles', 1);
  console.log(`Point 1 has ${count} outgoing relation(s):`);
  for (const edge of edges) {
    console.log(`  ${String(edge.source)} -[${edge.relType}]-> ${String(edge.target)}`);
  }

  const removed = await db.unrelate('articles', edgeId);
  console.log(`Edge removed: ${removed}`);
}

/** 5 — agent memory: episodic record/recall + durable TTL. */
async function agentMemoryRecall(db: VelesDB): Promise<void> {
  console.log('\n=== 4. Agent memory: record, recall, durable TTL ===');

  // The backing collection must exist first — nothing auto-creates it.
  await db.createCollection('agent_events', { dimension: DIM, metric: 'cosine' });
  const memory = db.agentMemory({ dimension: DIM });

  // recordEvent returns the generated point id (string, u64-safe).
  const eventId = await memory.recordEvent('agent_events', {
    eventType: 'user_query',
    data: { query: 'compare HNSW and IVF indexes' },
    embedding: generateEmbedding(7),
  });
  console.log(`Recorded event ${eventId}`);

  // Similarity recall (SearchResult[]).
  const similar = await memory.recallEvents('agent_events', generateEmbedding(7), 3);
  console.log(`recallEvents found ${similar.length} similar event(s)`);

  // Temporal recall (EpisodicRecord[], most-recent-first, no embedding needed).
  const recent = await memory.recallRecent('agent_events');
  console.log(`recallRecent found ${recent.length} event(s), newest ts=${recent[0]?.timestamp}`);

  // Expire the event in one hour — persisted server-side, survives restarts.
  await db.setTtlDurable('agent_events', eventId, 3600);
  console.log(`Set 1h durable TTL on event ${eventId}`);
}

async function main(): Promise<void> {
  const db = new VelesDB({ backend: 'rest', url: SERVER_URL });
  await db.init();

  try {
    await seedArticles(db);
    await hybridQuery(db);
    await relationEdges(db);
    await agentMemoryRecall(db);
  } finally {
    // Drop demo collections so the example is re-runnable.
    await db.deleteCollection('articles').catch(() => undefined);
    await db.deleteCollection('agent_events').catch(() => undefined);
    await db.close();
  }
}

main().catch((error: unknown) => {
  console.error('Example failed (is velesdb-server running?):', error);
  process.exitCode = 1;
});
