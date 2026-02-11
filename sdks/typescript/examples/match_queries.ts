/**
 * MATCH Query Examples for VelesDB TypeScript SDK
 *
 * Demonstrates Cypher-like graph pattern matching via db.matchQuery().
 * Requires the REST backend (server endpoint POST /collections/{name}/match).
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
 * Example 1: Simple pattern matching
 */
async function exampleSimpleMatch(db: VelesDB): Promise<void> {
  console.log('\n=== Example 1: Simple MATCH ===');

  const result = await db.matchQuery(
    'knowledge',
    'MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name'
  );

  console.log(`Found ${result.count} matches in ${result.tookMs}ms`);
  for (const r of result.results) {
    console.log(`  ${r.bindings.a} → ${r.bindings.b} (depth: ${r.depth})`);
  }
}

/**
 * Example 2: MATCH with similarity scoring
 */
async function exampleMatchWithSimilarity(db: VelesDB): Promise<void> {
  console.log('\n=== Example 2: MATCH + Similarity ===');

  const queryVector = mockEmbedding(42);

  const result = await db.matchQuery(
    'knowledge',
    'MATCH (doc:Document)-[:REFERENCES]->(ref) WHERE similarity(doc.embedding, $v) > 0.7 RETURN doc.title, ref.title',
    { v: queryVector },
    { vector: queryVector, threshold: 0.7 }
  );

  console.log(`Matched ${result.count} document-reference pairs`);
  for (const r of result.results) {
    console.log(`  "${r.projected['doc.title']}" references "${r.projected['ref.title']}" (score: ${r.score})`);
  }
}

/**
 * Example 3: Multi-hop traversal via MATCH
 */
async function exampleMultiHopMatch(db: VelesDB): Promise<void> {
  console.log('\n=== Example 3: Multi-Hop MATCH ===');

  const result = await db.matchQuery(
    'knowledge',
    'MATCH (a:Person)-[:KNOWS*1..3]->(b:Person) RETURN a.name, b.name'
  );

  console.log(`Reachable within 3 hops: ${result.count} pairs`);
  for (const r of result.results) {
    console.log(`  ${r.bindings.a} can reach ${r.bindings.b} (depth: ${r.depth})`);
  }
}

/**
 * Example 4: MATCH with projected fields
 */
async function exampleMatchWithProjection(db: VelesDB): Promise<void> {
  console.log('\n=== Example 4: MATCH with Projection ===');

  const result = await db.matchQuery(
    'knowledge',
    `MATCH (incident:Ticket)-[:IMPACTS]->(service:Microservice)
     WHERE incident.status = 'RESOLVED'
     RETURN incident.solution, service.name`
  );

  console.log(`Resolved incidents impacting services: ${result.count}`);
  for (const r of result.results) {
    console.log(`  Service: ${r.projected['service.name']}, Solution: ${r.projected['incident.solution']}`);
  }
}

/**
 * Main: run all MATCH examples
 */
async function main(): Promise<void> {
  console.log('='.repeat(60));
  console.log('VelesDB MATCH Query Examples — TypeScript SDK');
  console.log('='.repeat(60));

  const db = new VelesDB({ backend: 'rest', url: 'http://localhost:3030' });
  await db.init();

  // Note: These examples assume collections and graph data are already set up.
  // In production, you would create collections, insert vectors, and add edges first.
  console.log('\nNote: Ensure collections exist with graph data before running.\n');

  await exampleSimpleMatch(db);
  await exampleMatchWithSimilarity(db);
  await exampleMultiHopMatch(db);
  await exampleMatchWithProjection(db);

  await db.close();
  console.log('\nDone.');
}

main().catch(console.error);

export { exampleSimpleMatch, exampleMatchWithSimilarity, exampleMultiHopMatch, exampleMatchWithProjection };
