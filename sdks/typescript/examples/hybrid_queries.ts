/**
 * Hybrid Query Examples for VelesDB TypeScript SDK
 *
 * Demonstrates vector similarity search combined with metadata filtering,
 * aggregations, and multi-model query patterns.
 *
 * See docs/guides/USE_CASES.md for the 10 documented use cases.
 */

import { VelesDB, Collection, SearchResult } from '../src';

/**
 * Generate a deterministic mock embedding for demo purposes.
 */
function generateEmbedding(seed: number, dim: number = 128): number[] {
  const embedding: number[] = [];
  for (let i = 0; i < dim; i++) {
    embedding.push(Math.sin(seed * 0.1 + i * 0.01));
  }
  // Normalize
  const norm = Math.sqrt(embedding.reduce((sum, x) => sum + x * x, 0));
  return embedding.map((x) => x / norm);
}

/**
 * Use Case 1: Contextual RAG
 *
 * Find documents similar to a query with metadata filtering.
 */
async function example1ContextualRag(collection: Collection): Promise<void> {
  console.log('\n=== Use Case 1: Contextual RAG ===');

  const queryEmbedding = generateEmbedding(42);

  // VelesQL query
  const velesql = `
    SELECT id, title, category 
    FROM documents 
    WHERE similarity(embedding, $query) > 0.75 
      AND category = 'ai'
    ORDER BY similarity(embedding, $query) DESC
    LIMIT 10
  `;
  console.log('VelesQL:', velesql.trim());

  // Programmatic API
  const results = await collection.search(queryEmbedding, 20);
  const filtered = results.filter(
    (r) => r.score > 0.75 && r.payload?.category === 'ai'
  );
  console.log(`Found ${filtered.length} relevant AI documents`);
}

/**
 * Use Case 5: Semantic Search with Filters
 *
 * Combine vector NEAR with multiple metadata filters.
 */
async function example2SemanticSearchWithFilters(
  collection: Collection
): Promise<void> {
  console.log('\n=== Use Case 5: Semantic Search with Filters ===');

  const velesql = `
    SELECT id, title, price 
    FROM articles 
    WHERE vector NEAR $query 
      AND category IN ('technology', 'science') 
      AND published_date >= '2024-01-01'
    LIMIT 20
    WITH (mode = 'balanced')
  `;
  console.log('VelesQL:', velesql.trim());

  const queryVec = generateEmbedding(100);
  const results = await collection.search(queryVec, 50);

  const filtered = results
    .filter((r) => {
      const cat = r.payload?.category as string;
      return ['technology', 'science'].includes(cat);
    })
    .slice(0, 20);

  console.log(`Found ${filtered.length} matching articles`);
}

/**
 * Use Case 4: Document Clustering with Aggregations
 *
 * Group similar documents by category.
 */
async function example3Aggregations(collection: Collection): Promise<void> {
  console.log('\n=== Use Case 4: Document Clustering ===');

  const velesql = `
    SELECT category, COUNT(*) 
    FROM documents 
    WHERE similarity(embedding, $query) > 0.6 
    GROUP BY category 
    ORDER BY COUNT(*) DESC
  `;
  console.log('VelesQL:', velesql.trim());

  const queryVec = generateEmbedding(50);
  const results = await collection.search(queryVec, 100);

  // Manual aggregation
  const counts = new Map<string, number>();
  for (const r of results.filter((r) => r.score > 0.6)) {
    const cat = (r.payload?.category as string) || 'unknown';
    counts.set(cat, (counts.get(cat) || 0) + 1);
  }

  console.log('Category counts:');
  for (const [cat, count] of counts.entries()) {
    console.log(`  ${cat}: ${count}`);
  }
}

/**
 * Use Case 6: Recommendation Engine
 *
 * Find similar items based on user preferences.
 */
async function example4RecommendationEngine(
  collection: Collection
): Promise<void> {
  console.log('\n=== Use Case 6: Recommendation Engine ===');

  const velesql = `
    SELECT id, name, category, price 
    FROM items 
    WHERE similarity(embedding, $user_preference) > 0.7 
      AND category = 'electronics' 
      AND price < 100
    ORDER BY similarity(embedding, $user_preference) DESC
    LIMIT 10
  `;
  console.log('VelesQL:', velesql.trim());

  const userPreference = generateEmbedding(42);
  const results = await collection.search(userPreference, 30);

  const recommendations = results
    .filter((r) => {
      const price = r.payload?.price as number;
      const cat = r.payload?.category as string;
      return price < 100 && cat === 'electronics';
    })
    .slice(0, 10);

  console.log(`Generated ${recommendations.length} recommendations`);
}

/**
 * Use Case 10: Conversational Memory for AI Agents
 *
 * Retrieve relevant context from conversation history.
 */
async function example5ConversationalMemory(
  collection: Collection,
  conversationId: string
): Promise<void> {
  console.log('\n=== Use Case 10: Conversational Memory ===');

  const velesql = `
    SELECT content, role, timestamp 
    FROM messages 
    WHERE conversation_id = $conv_id 
      AND similarity(embedding, $current_query) > 0.6 
    ORDER BY timestamp DESC 
    LIMIT 10
  `;
  console.log('VelesQL:', velesql.trim());

  const currentQueryEmb = generateEmbedding(75);
  const results = await collection.search(currentQueryEmb, 30);

  const relevantContext = results
    .filter(
      (r) => r.score > 0.6 && r.payload?.conversation_id === conversationId
    )
    .slice(0, 10);

  console.log(`Retrieved ${relevantContext.length} relevant messages for context`);
}

/**
 * Main function demonstrating all hybrid query examples.
 */
async function main(): Promise<void> {
  console.log('='.repeat(60));
  console.log('VelesDB Hybrid Query Examples - TypeScript SDK');
  console.log('='.repeat(60));

  // Note: This is a demo - in real usage, you'd open a real database
  console.log('\nNote: These examples show the query patterns.');
  console.log('Replace with real VelesDB instance for actual usage.\n');

  // Show VelesQL examples without executing (demo mode)
  const mockCollection = {
    search: async (_vec: number[], _k: number): Promise<SearchResult[]> => [],
  } as Collection;

  await example1ContextualRag(mockCollection);
  await example2SemanticSearchWithFilters(mockCollection);
  await example3Aggregations(mockCollection);
  await example4RecommendationEngine(mockCollection);
  await example5ConversationalMemory(mockCollection, 'conv_001');

  console.log('\n' + '='.repeat(60));
  console.log('See docs/guides/USE_CASES.md for all 10 use cases');
  console.log('See docs/VELESQL_SPEC.md for complete VelesQL reference');
  console.log('='.repeat(60));
}

// Run if executed directly
main().catch(console.error);

export {
  example1ContextualRag,
  example2SemanticSearchWithFilters,
  example3Aggregations,
  example4RecommendationEngine,
  example5ConversationalMemory,
};
