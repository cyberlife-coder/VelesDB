/**
 * EXPLAIN Query Plan Examples for VelesDB TypeScript SDK
 *
 * Demonstrates db.explain() for analyzing VelesQL queries without executing them.
 * Returns query plan steps, cost estimation, and feature detection.
 * Requires the REST backend (server endpoint POST /query/explain).
 */

import { VelesDB } from '../src';

/**
 * Example 1: Explain a vector search query
 */
async function exampleExplainVectorSearch(db: VelesDB): Promise<void> {
  console.log('\n=== Example 1: Explain Vector Search ===');

  const plan = await db.explain(
    'SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8 LIMIT 10'
  );

  console.log(`Query type: ${plan.queryType}`);
  console.log(`Collection: ${plan.collection}`);
  console.log(`Complexity: ${plan.estimatedCost.complexity}`);
  console.log(`Uses index: ${plan.estimatedCost.usesIndex}`);

  console.log('\nFeatures detected:');
  console.log(`  Vector search: ${plan.features.hasVectorSearch}`);
  console.log(`  Filter: ${plan.features.hasFilter}`);
  console.log(`  Order by: ${plan.features.hasOrderBy}`);
  console.log(`  Limit: ${plan.features.limit}`);

  console.log('\nPlan steps:');
  for (const step of plan.plan) {
    console.log(`  Step ${step.step}: ${step.operation} — ${step.description}`);
    if (step.estimatedRows) {
      console.log(`    Estimated rows: ${step.estimatedRows}`);
    }
  }
}

/**
 * Example 2: Explain an aggregation query
 */
async function exampleExplainAggregation(db: VelesDB): Promise<void> {
  console.log('\n=== Example 2: Explain Aggregation ===');

  const plan = await db.explain(
    'SELECT category, COUNT(*) FROM products WHERE price > $min GROUP BY category ORDER BY COUNT(*) DESC',
    { min: 10 }
  );

  console.log(`Query type: ${plan.queryType}`);
  console.log(`Has aggregation: ${plan.features.hasAggregation}`);
  console.log(`Has group by: ${plan.features.hasGroupBy}`);
  console.log(`Has order by: ${plan.features.hasOrderBy}`);
  console.log(`Has filter: ${plan.features.hasFilter}`);
}

/**
 * Example 3: Explain a JOIN query
 */
async function exampleExplainJoin(db: VelesDB): Promise<void> {
  console.log('\n=== Example 3: Explain JOIN ===');

  const plan = await db.explain(
    'SELECT * FROM orders JOIN customers ON orders.customer_id = customers.id WHERE status = $s',
    { s: 'active' }
  );

  console.log(`Query type: ${plan.queryType}`);
  console.log(`Has join: ${plan.features.hasJoin}`);
  console.log(`Has filter: ${plan.features.hasFilter}`);
  console.log(`Uses index: ${plan.estimatedCost.usesIndex}`);
  if (plan.estimatedCost.indexName) {
    console.log(`Index name: ${plan.estimatedCost.indexName}`);
  }
  console.log(`Selectivity: ${plan.estimatedCost.selectivity}`);
}

/**
 * Example 4: Compare query plans for optimization
 */
async function exampleCompareQueryPlans(db: VelesDB): Promise<void> {
  console.log('\n=== Example 4: Compare Query Plans ===');

  const queries = [
    'SELECT * FROM docs WHERE category = $cat LIMIT 10',
    'SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8 LIMIT 10',
    'SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8 AND category = $cat LIMIT 10',
  ];

  for (const q of queries) {
    const plan = await db.explain(q);
    const features: string[] = [];
    if (plan.features.hasVectorSearch) features.push('vector');
    if (plan.features.hasFilter) features.push('filter');
    if (plan.features.hasJoin) features.push('join');

    console.log(`\n  Query: ${q.substring(0, 60)}...`);
    console.log(`  Complexity: ${plan.estimatedCost.complexity}`);
    console.log(`  Features: [${features.join(', ')}]`);
    console.log(`  Steps: ${plan.plan.length}`);
  }
}

/**
 * Main: run all EXPLAIN examples
 */
async function main(): Promise<void> {
  console.log('='.repeat(60));
  console.log('VelesDB EXPLAIN Query Plan Examples — TypeScript SDK');
  console.log('='.repeat(60));

  const db = new VelesDB({ backend: 'rest', url: 'http://localhost:3030' });
  await db.init();

  await exampleExplainVectorSearch(db);
  await exampleExplainAggregation(db);
  await exampleExplainJoin(db);
  await exampleCompareQueryPlans(db);

  await db.close();
  console.log('\nDone.');
}

main().catch(console.error);

export { exampleExplainVectorSearch, exampleExplainAggregation, exampleExplainJoin, exampleCompareQueryPlans };
