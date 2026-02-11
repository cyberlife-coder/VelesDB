/**
 * Query Builder Examples for VelesDB TypeScript SDK
 *
 * Demonstrates both query builders:
 * - VelesQLBuilder (velesql) for MATCH graph queries
 * - SelectBuilder (selectql) for SELECT queries
 */

import { VelesDB, velesql, selectql } from '../src';

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
 * Example 1: VelesQLBuilder — graph pattern with vector NEAR
 */
function exampleMatchBuilder(): void {
  console.log('\n=== Example 1: VelesQLBuilder (MATCH) ===');

  const queryVector = mockEmbedding(42);

  const builder = velesql()
    .match('d', 'Document')
    .nearVector('$q', queryVector, { topK: 20 })
    .andWhere('d.category = $cat', { cat: 'tech' })
    .limit(10);

  const queryStr = builder.toVelesQL();
  const params = builder.getParams();

  console.log('Query:', queryStr);
  console.log('Params keys:', Object.keys(params));
}

/**
 * Example 2: VelesQLBuilder — graph traversal with relationships
 */
function exampleGraphTraversalBuilder(): void {
  console.log('\n=== Example 2: VelesQLBuilder (Graph Traversal) ===');

  const builder = velesql()
    .match('p', 'Person')
    .rel('KNOWS')
    .to('f', 'Person')
    .where('p.age > 25')
    .andWhere('f.city = $city', { city: 'Paris' })
    .return(['p.name', 'f.name', 'f.email'])
    .orderBy('p.name')
    .limit(50);

  console.log('Query:', builder.toVelesQL());
}

/**
 * Example 3: VelesQLBuilder — similarity with threshold
 */
function exampleSimilarityBuilder(): void {
  console.log('\n=== Example 3: VelesQLBuilder (Similarity) ===');

  const queryVector = mockEmbedding(99);

  const builder = velesql()
    .match('doc', 'Document')
    .similarity(queryVector, { threshold: 0.8, field: 'embedding' })
    .andWhere('doc.status = $s', { s: 'published' })
    .orderBySimilarity()
    .limit(15);

  console.log('Query:', builder.toVelesQL());
}

/**
 * Example 4: SelectBuilder — vector search with filters
 */
function exampleSelectBuilder(): void {
  console.log('\n=== Example 4: SelectBuilder (Vector Search) ===');

  const queryVector = mockEmbedding(77);

  const { query, params } = selectql()
    .select('id', 'title', 'category')
    .from('documents')
    .similarity('embedding', 'v', queryVector, { threshold: 0.7 })
    .andWhere('category = $cat', { cat: 'tech' })
    .orderBy('title', 'ASC')
    .limit(20)
    .build();

  console.log('Query:', query);
  console.log('Params keys:', Object.keys(params));
}

/**
 * Example 5: SelectBuilder — aggregation with GROUP BY
 */
function exampleAggregationBuilder(): void {
  console.log('\n=== Example 5: SelectBuilder (Aggregation) ===');

  const { query } = selectql()
    .selectAgg('COUNT', '*', 'total')
    .selectAgg('AVG', 'price', 'avg_price')
    .select('category')
    .from('products')
    .where('price > $min', { min: 10 })
    .groupBy('category')
    .orderBy('total', 'DESC')
    .limit(10)
    .build();

  console.log('Query:', query);
}

/**
 * Example 6: SelectBuilder — JOIN query
 */
function exampleJoinBuilder(): void {
  console.log('\n=== Example 6: SelectBuilder (JOIN) ===');

  const { query } = selectql()
    .select('o.id', 'c.name', 'o.total')
    .from('orders AS o')
    .join('customers AS c', 'o.customer_id = c.id', 'LEFT')
    .where('o.status = $s', { s: 'active' })
    .orderBy('o.total', 'DESC')
    .limit(50)
    .build();

  console.log('Query:', query);
}

/**
 * Example 7: Using builders with db.query()
 */
async function exampleExecuteWithBuilder(db: VelesDB): Promise<void> {
  console.log('\n=== Example 7: Execute Builder Output ===');

  const queryVector = mockEmbedding(42);

  // Build a SELECT query
  const { query, params } = selectql()
    .select('id', 'title')
    .from('documents')
    .nearVector('q', queryVector, { topK: 10 })
    .limit(5)
    .build();

  // Execute via db.query()
  const response = await db.query('documents', query, params);
  console.log(`Results: ${response.results.length}, took: ${response.stats.executionTimeMs}ms`);
}

/**
 * Main: run all query builder examples
 */
async function main(): Promise<void> {
  console.log('='.repeat(60));
  console.log('VelesDB Query Builder Examples — TypeScript SDK');
  console.log('='.repeat(60));

  // Pure builder examples (no server needed)
  exampleMatchBuilder();
  exampleGraphTraversalBuilder();
  exampleSimilarityBuilder();
  exampleSelectBuilder();
  exampleAggregationBuilder();
  exampleJoinBuilder();

  // Server-connected example
  console.log('\n--- Server-connected example (requires velesdb-server) ---');
  try {
    const db = new VelesDB({ backend: 'rest', url: 'http://localhost:3030' });
    await db.init();
    await exampleExecuteWithBuilder(db);
    await db.close();
  } catch {
    console.log('(Skipped — server not available)');
  }

  console.log('\nDone.');
}

main().catch(console.error);

export {
  exampleMatchBuilder,
  exampleGraphTraversalBuilder,
  exampleSimilarityBuilder,
  exampleSelectBuilder,
  exampleAggregationBuilder,
  exampleJoinBuilder,
  exampleExecuteWithBuilder,
};
