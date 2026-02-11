/**
 * Knowledge Graph API Examples for VelesDB TypeScript SDK
 *
 * Demonstrates graph operations: addEdge, getEdges, traverseGraph, getNodeDegree.
 * These features require the REST backend (server-side EdgeStore).
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
 * Example 1: Build a social graph and add typed edges
 */
async function exampleAddEdges(db: VelesDB): Promise<void> {
  console.log('\n=== Example 1: Add Edges ===');

  // Insert nodes (people) as vectors
  await db.insert('social', { id: 1, vector: mockEmbedding(1), payload: { name: 'Alice', role: 'engineer' } });
  await db.insert('social', { id: 2, vector: mockEmbedding(2), payload: { name: 'Bob', role: 'designer' } });
  await db.insert('social', { id: 3, vector: mockEmbedding(3), payload: { name: 'Carol', role: 'manager' } });

  // Add typed relationships
  await db.addEdge('social', { id: 1, source: 1, target: 2, label: 'KNOWS', properties: { since: '2024-01' } });
  await db.addEdge('social', { id: 2, source: 2, target: 3, label: 'REPORTS_TO' });
  await db.addEdge('social', { id: 3, source: 1, target: 3, label: 'KNOWS', properties: { since: '2023-06' } });

  console.log('Added 3 nodes and 3 edges');
}

/**
 * Example 2: Query edges with label filter
 */
async function exampleGetEdges(db: VelesDB): Promise<void> {
  console.log('\n=== Example 2: Get Edges ===');

  // Get all edges
  const allEdges = await db.getEdges('social');
  console.log(`Total edges: ${allEdges.length}`);

  // Filter by label
  const knowsEdges = await db.getEdges('social', { label: 'KNOWS' });
  console.log(`KNOWS edges: ${knowsEdges.length}`);

  for (const edge of knowsEdges) {
    console.log(`  ${edge.source} --[${edge.label}]--> ${edge.target}`);
  }
}

/**
 * Example 3: Traverse graph with BFS and DFS
 */
async function exampleTraverseGraph(db: VelesDB): Promise<void> {
  console.log('\n=== Example 3: Graph Traversal ===');

  // BFS from Alice (node 1)
  const bfsResult = await db.traverseGraph('social', {
    source: 1,
    strategy: 'bfs',
    maxDepth: 3,
    limit: 100,
  });

  console.log(`BFS from Alice: ${bfsResult.results.length} nodes reached`);
  for (const node of bfsResult.results) {
    console.log(`  Node ${node.targetId} at depth ${node.depth} via ${node.edgeLabel}`);
  }

  // DFS with specific relationship types
  const dfsResult = await db.traverseGraph('social', {
    source: 1,
    strategy: 'dfs',
    maxDepth: 2,
    relTypes: ['KNOWS'],
  });

  console.log(`DFS (KNOWS only): ${dfsResult.results.length} nodes`);
}

/**
 * Example 4: Check node connectivity with degree
 */
async function exampleNodeDegree(db: VelesDB): Promise<void> {
  console.log('\n=== Example 4: Node Degree ===');

  const aliceDegree = await db.getNodeDegree('social', 1);
  console.log(`Alice — In: ${aliceDegree.inDegree}, Out: ${aliceDegree.outDegree}`);

  const carolDegree = await db.getNodeDegree('social', 3);
  console.log(`Carol — In: ${carolDegree.inDegree}, Out: ${carolDegree.outDegree}`);
}

/**
 * Main: run all graph examples
 */
async function main(): Promise<void> {
  console.log('='.repeat(60));
  console.log('VelesDB Knowledge Graph Examples — TypeScript SDK');
  console.log('='.repeat(60));

  const db = new VelesDB({ backend: 'rest', url: 'http://localhost:3030' });
  await db.init();

  await db.createCollection('social', { dimension: 128, metric: 'cosine' });

  await exampleAddEdges(db);
  await exampleGetEdges(db);
  await exampleTraverseGraph(db);
  await exampleNodeDegree(db);

  await db.deleteCollection('social');
  await db.close();

  console.log('\nDone.');
}

main().catch(console.error);

export { exampleAddEdges, exampleGetEdges, exampleTraverseGraph, exampleNodeDegree };
