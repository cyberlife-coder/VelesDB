/**
 * Multi-Query Fusion Examples for VelesDB TypeScript SDK
 *
 * Demonstrates multiQuerySearch() for RAG pipelines using
 * Multiple Query Generation (MQG) with different fusion strategies.
 * Requires the REST backend (server endpoint POST /collections/{name}/search/multi).
 */

import { VelesDB } from '../src';

/** Helper: generate a deterministic mock embedding */
function mockEmbedding(seed: number, dim = 128): number[] {
  const emb: number[] = [];
  for (let i = 0; i < dim; i++) {
    emb.push(Math.sin(seed * 0.1 + i * 0.01));
  }
  const norm = Math.sqrt(emb.reduce((s, x) => s + x * x, 0));
  return emb.map((x) => x / norm);
}

/**
 * Example 1: RRF fusion (Reciprocal Rank Fusion)
 *
 * Best for combining results from semantically different queries.
 * Default strategy — works well in most cases.
 */
async function exampleRrfFusion(db: VelesDB): Promise<void> {
  console.log('\n=== Example 1: RRF Fusion ===');

  // Simulate MQG: one user query → multiple reformulations
  const originalQuery = mockEmbedding(42);
  const reformulation1 = mockEmbedding(43);
  const reformulation2 = mockEmbedding(44);

  const results = await db.multiQuerySearch(
    'documents',
    [originalQuery, reformulation1, reformulation2],
    {
      k: 10,
      fusion: 'rrf',
      fusionParams: { k: 60 }, // RRF k parameter (default: 60)
    }
  );

  console.log(`RRF fusion: ${results.length} results from 3 query vectors`);
  for (const r of results.slice(0, 5)) {
    console.log(`  ID: ${r.id}, Score: ${r.score.toFixed(4)}`);
  }
}

/**
 * Example 2: Weighted fusion
 *
 * Combines average, max, and hit-count scores with custom weights.
 * Good when you want to control the importance of each signal.
 */
async function exampleWeightedFusion(db: VelesDB): Promise<void> {
  console.log('\n=== Example 2: Weighted Fusion ===');

  const queries = [mockEmbedding(50), mockEmbedding(51)];

  const results = await db.multiQuerySearch('documents', queries, {
    k: 10,
    fusion: 'weighted',
    fusionParams: {
      avgWeight: 0.6,  // Weight for average score across queries
      maxWeight: 0.3,  // Weight for best individual score
      hitWeight: 0.1,  // Weight for number of queries that found this result
    },
  });

  console.log(`Weighted fusion: ${results.length} results`);
  for (const r of results.slice(0, 5)) {
    console.log(`  ID: ${r.id}, Score: ${r.score.toFixed(4)}`);
  }
}

/**
 * Example 3: Average fusion (simple)
 *
 * Takes the mean score across all queries. Simple and interpretable.
 */
async function exampleAverageFusion(db: VelesDB): Promise<void> {
  console.log('\n=== Example 3: Average Fusion ===');

  const queries = [mockEmbedding(60), mockEmbedding(61), mockEmbedding(62)];

  const results = await db.multiQuerySearch('documents', queries, {
    k: 15,
    fusion: 'average',
  });

  console.log(`Average fusion: ${results.length} results from ${queries.length} queries`);
}

/**
 * Example 4: Maximum fusion
 *
 * Takes the best score from any query for each document.
 * Good when any single match is sufficient.
 */
async function exampleMaximumFusion(db: VelesDB): Promise<void> {
  console.log('\n=== Example 4: Maximum Fusion ===');

  const queries = [mockEmbedding(70), mockEmbedding(71)];

  const results = await db.multiQuerySearch('documents', queries, {
    k: 10,
    fusion: 'maximum',
  });

  console.log(`Maximum fusion: ${results.length} results`);
}

/**
 * Example 5: MQG RAG Pipeline pattern
 *
 * Shows the full pattern for using multi-query fusion in a RAG pipeline.
 */
async function exampleRagPipeline(db: VelesDB): Promise<void> {
  console.log('\n=== Example 5: RAG Pipeline with MQG ===');

  // Step 1: Generate multiple query embeddings from user question
  // In production, you'd use an LLM to generate reformulations
  const userQueryEmb = mockEmbedding(100);
  const reformulations = [
    mockEmbedding(101), // "What are the key benefits?"
    mockEmbedding(102), // "Explain the advantages"
    mockEmbedding(103), // "List the main pros"
  ];

  const allQueries = [userQueryEmb, ...reformulations];

  // Step 2: Multi-query search with RRF (best default for diverse queries)
  const results = await db.multiQuerySearch('knowledge_base', allQueries, {
    k: 5,
    fusion: 'rrf',
    fusionParams: { k: 60 },
    filter: { status: 'published' },
  });

  // Step 3: Use results as context for LLM
  console.log(`Retrieved ${results.length} context chunks for RAG`);
  for (const r of results) {
    console.log(`  [${r.id}] score=${r.score.toFixed(4)} — "${r.payload?.title ?? 'untitled'}"`);
  }

  // In production: feed results to LLM as context
}

/**
 * Main: run all multi-query fusion examples
 */
async function main(): Promise<void> {
  console.log('='.repeat(60));
  console.log('VelesDB Multi-Query Fusion Examples — TypeScript SDK');
  console.log('='.repeat(60));

  const db = new VelesDB({ backend: 'rest', url: 'http://localhost:3030' });
  await db.init();

  console.log('\nNote: Ensure "documents" and "knowledge_base" collections exist.\n');

  await exampleRrfFusion(db);
  await exampleWeightedFusion(db);
  await exampleAverageFusion(db);
  await exampleMaximumFusion(db);
  await exampleRagPipeline(db);

  await db.close();
  console.log('\nDone.');
}

main().catch(console.error);

export {
  exampleRrfFusion,
  exampleWeightedFusion,
  exampleAverageFusion,
  exampleMaximumFusion,
  exampleRagPipeline,
};
