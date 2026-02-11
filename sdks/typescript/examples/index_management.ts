/**
 * Index Management Examples for VelesDB TypeScript SDK
 *
 * Demonstrates createIndex, listIndexes, hasIndex, and dropIndex.
 * Indexes enable O(1) equality lookups and O(log n) range queries on properties.
 * Requires the REST backend (server endpoints on /collections/{name}/indexes).
 */

import { VelesDB } from '../src';

/**
 * Example 1: Create hash and range indexes
 */
async function exampleCreateIndexes(db: VelesDB): Promise<void> {
  console.log('\n=== Example 1: Create Indexes ===');

  // Hash index — O(1) equality lookups (default type)
  await db.createIndex('users', {
    label: 'Person',
    property: 'email',
  });
  console.log('Created hash index on Person.email');

  // Range index — O(log n) range queries
  await db.createIndex('events', {
    label: 'Event',
    property: 'timestamp',
    indexType: 'range',
  });
  console.log('Created range index on Event.timestamp');

  // Hash index for category filtering
  await db.createIndex('products', {
    label: 'Product',
    property: 'category',
  });
  console.log('Created hash index on Product.category');
}

/**
 * Example 2: List all indexes on a collection
 */
async function exampleListIndexes(db: VelesDB): Promise<void> {
  console.log('\n=== Example 2: List Indexes ===');

  const indexes = await db.listIndexes('users');
  console.log(`Indexes on "users": ${indexes.length}`);

  for (const idx of indexes) {
    console.log(`  ${idx.label}.${idx.property} (${idx.indexType}) — ${idx.entryCount} entries`);
  }
}

/**
 * Example 3: Check if an index exists before creating
 */
async function exampleHasIndex(db: VelesDB): Promise<void> {
  console.log('\n=== Example 3: Check Index Existence ===');

  const emailExists = await db.hasIndex('users', 'Person', 'email');
  console.log(`Person.email index exists: ${emailExists}`);

  const ageExists = await db.hasIndex('users', 'Person', 'age');
  console.log(`Person.age index exists: ${ageExists}`);

  // Create only if not exists
  if (!ageExists) {
    await db.createIndex('users', { label: 'Person', property: 'age', indexType: 'range' });
    console.log('Created missing Person.age range index');
  }
}

/**
 * Example 4: Drop an index
 */
async function exampleDropIndex(db: VelesDB): Promise<void> {
  console.log('\n=== Example 4: Drop Index ===');

  const dropped = await db.dropIndex('users', 'Person', 'email');
  console.log(`Dropped Person.email index: ${dropped}`);

  // Dropping a non-existent index returns false
  const droppedAgain = await db.dropIndex('users', 'Person', 'email');
  console.log(`Drop again (already gone): ${droppedAgain}`);
}

/**
 * Main: run all index management examples
 */
async function main(): Promise<void> {
  console.log('='.repeat(60));
  console.log('VelesDB Index Management Examples — TypeScript SDK');
  console.log('='.repeat(60));

  const db = new VelesDB({ backend: 'rest', url: 'http://localhost:3030' });
  await db.init();

  // Ensure collections exist
  await db.createCollection('users', { dimension: 128, metric: 'cosine' });
  await db.createCollection('events', { dimension: 128, metric: 'cosine' });
  await db.createCollection('products', { dimension: 128, metric: 'cosine' });

  await exampleCreateIndexes(db);
  await exampleListIndexes(db);
  await exampleHasIndex(db);
  await exampleDropIndex(db);

  // Cleanup
  await db.deleteCollection('users');
  await db.deleteCollection('events');
  await db.deleteCollection('products');
  await db.close();

  console.log('\nDone.');
}

main().catch(console.error);

export { exampleCreateIndexes, exampleListIndexes, exampleHasIndex, exampleDropIndex };
