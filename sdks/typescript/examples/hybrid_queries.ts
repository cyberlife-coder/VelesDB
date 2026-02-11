/**
 * Hybrid Query Examples for VelesDB TypeScript SDK
 *
 * Demonstrates real VelesQL queries through db.query() and db.matchQuery():
 * - SELECT + NEAR + WHERE filters + ORDER BY + LIMIT
 * - MATCH graph traversal + similarity() + multi-column RETURN
 * - JOIN + vector search + aggregation
 * - FUSION hybrid + filters
 * - Agent memory patterns
 *
 * All examples use the real SDK API — no mocks.
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

// ============================================================================
// 1. SELECT + NEAR + filters + ORDER + LIMIT
// ============================================================================

async function exampleVectorWithFilters(db: VelesDB): Promise<void> {
  console.log('\n=== 1. Vector Search + Filters + ORDER + LIMIT ===');

  const vec = mockEmbedding(42);
  const result = await db.query(
    'documents',
    `SELECT id, title, category, price
     FROM documents
     WHERE vector NEAR $v
       AND category IN ('tech', 'science')
       AND price BETWEEN 10 AND 100
     ORDER BY similarity(vector, $v) DESC
     LIMIT 10 OFFSET 0`,
    { v: vec }
  );

  console.log(`Found ${result.results.length} docs in ${result.stats.executionTimeMs}ms`);
  for (const r of result.results) {
    console.log(`  [${r.nodeId}] ${r.columnData?.title} — ${r.columnData?.category} ($${r.columnData?.price})`);
  }
}

// ============================================================================
// 2. similarity() threshold + LIKE + multi-column
// ============================================================================

async function exampleSimilarityWithLike(db: VelesDB): Promise<void> {
  console.log('\n=== 2. Similarity Threshold + LIKE + Multi-Column ===');

  const vec = mockEmbedding(99);
  const result = await db.query(
    'articles',
    `SELECT id, title, author, journal, published_at
     FROM articles
     WHERE similarity(embedding, $v) > 0.7
       AND title ILIKE '%machine learning%'
       AND status = $s
     ORDER BY similarity(embedding, $v) DESC
     LIMIT 15`,
    { v: vec, s: 'published' }
  );

  console.log(`Matched ${result.results.length} articles`);
}

// ============================================================================
// 3. MATCH graph + similarity + multi-column RETURN
// ============================================================================

async function exampleMatchGraphVector(db: VelesDB): Promise<void> {
  console.log('\n=== 3. MATCH Graph + Similarity + Multi-Column ===');

  const vec = mockEmbedding(50);
  const result = await db.matchQuery(
    'knowledge',
    `MATCH (doc:Document)-[:REFERENCES]->(ref:Document)
     WHERE similarity(doc.embedding, $v) > 0.8
       AND doc.category = 'AI'
     RETURN doc.title, doc.category, doc.author, ref.title, ref.journal
     ORDER BY similarity() DESC
     LIMIT 20`,
    { v: vec },
    { vector: vec, threshold: 0.8 }
  );

  console.log(`Found ${result.count} document-reference pairs in ${result.tookMs}ms`);
  for (const r of result.results) {
    console.log(`  "${r.projected['doc.title']}" (${r.projected['doc.category']}) → "${r.projected['ref.title']}"`);
  }
}

// ============================================================================
// 4. MATCH multi-hop + vector + filters + multi-column
// ============================================================================

async function exampleMatchMultiHopVector(db: VelesDB): Promise<void> {
  console.log('\n=== 4. MATCH Multi-Hop + Vector + Filters + Multi-Column ===');

  const vec = mockEmbedding(77);
  const result = await db.matchQuery(
    'social',
    `MATCH (user:User)-[:FOLLOWS]->(friend:User)-[:POSTED]->(post:Post)
     WHERE similarity(post.embedding, $v) > 0.75
       AND post.status = 'published'
       AND friend.verified = true
     RETURN user.name, user.email, friend.name, post.title, post.category, post.created_at
     ORDER BY similarity() DESC
     LIMIT 10`,
    { v: vec },
    { vector: vec, threshold: 0.75 }
  );

  console.log(`Found ${result.count} social-vector matches`);
  for (const r of result.results) {
    console.log(`  ${r.projected['user.name']} → ${r.projected['friend.name']} → "${r.projected['post.title']}"`);
  }
}

// ============================================================================
// 5. MATCH bidirectional + vector + WHERE conditions
// ============================================================================

async function exampleMatchBidirectionalVector(db: VelesDB): Promise<void> {
  console.log('\n=== 5. MATCH Bidirectional + Vector + Conditions ===');

  const vec = mockEmbedding(33);
  const result = await db.matchQuery(
    'research',
    `MATCH (a:Researcher)-[:COLLABORATES]-(b:Researcher)-[:PUBLISHED]->(p:Paper)
     WHERE similarity(p.abstract_emb, $v) > 0.7
       AND p.year >= 2024
       AND a.institution = $inst
     RETURN a.name, a.institution, b.name, b.institution, p.title, p.journal, p.year
     ORDER BY similarity() DESC
     LIMIT 15`,
    { v: vec, inst: 'MIT' },
    { vector: vec, threshold: 0.7 }
  );

  console.log(`Found ${result.count} researcher-paper matches`);
}

// ============================================================================
// 6. MATCH + NEAR (without similarity threshold)
// ============================================================================

async function exampleMatchWithNear(db: VelesDB): Promise<void> {
  console.log('\n=== 6. MATCH + NEAR + Multi-Column ===');

  const vec = mockEmbedding(66);
  const result = await db.matchQuery(
    'docs',
    `MATCH (author:Author)-[:WROTE]->(doc:Document)
     WHERE doc.vector NEAR $v AND doc.category = $cat
     RETURN author.name, author.email, doc.title, doc.category, doc.published_at
     LIMIT 10`,
    { v: vec, cat: 'AI' }
  );

  console.log(`Found ${result.count} author-document pairs`);
}

// ============================================================================
// 7. MATCH variable-length path + similarity
// ============================================================================

async function exampleMatchVariablePath(db: VelesDB): Promise<void> {
  console.log('\n=== 7. MATCH Variable-Length Path + Similarity ===');

  const vec = mockEmbedding(88);
  const result = await db.matchQuery(
    'knowledge',
    `MATCH (concept:Topic)-[:RELATED_TO*1..3]->(related:Topic)
     WHERE similarity(concept.embedding, $v) > 0.6
     RETURN concept.name, concept.domain, related.name, related.domain
     LIMIT 50`,
    { v: vec },
    { vector: vec, threshold: 0.6 }
  );

  console.log(`Found ${result.count} topic connections within 3 hops`);
}

// ============================================================================
// 8. JOIN + vector search + aggregation + ORDER + LIMIT
// ============================================================================

async function exampleJoinVectorAggregation(db: VelesDB): Promise<void> {
  console.log('\n=== 8. JOIN + Vector + Aggregation + ORDER + LIMIT ===');

  const vec = mockEmbedding(55);
  const result = await db.query(
    'orders',
    `SELECT c.country, COUNT(*) AS total, SUM(o.amount) AS revenue
     FROM orders AS o
     JOIN customers AS c ON o.customer_id = c.id
     WHERE similarity(o.embedding, $v) > 0.6
       AND o.created_at > $since
     GROUP BY c.country
     HAVING SUM(o.amount) > 1000
     ORDER BY revenue DESC
     LIMIT 10`,
    { v: vec, since: '2025-01-01' }
  );

  console.log(`Analytics: ${result.results.length} country groups`);
}

// ============================================================================
// 9. FUSION hybrid + filters + ORDER + LIMIT
// ============================================================================

async function exampleFusionHybrid(db: VelesDB): Promise<void> {
  console.log('\n=== 9. FUSION Hybrid + Filters + ORDER + LIMIT ===');

  const result = await db.query(
    'docs',
    `SELECT id, title, category, score
     FROM docs
     USING FUSION(strategy = 'weighted', vector_weight = 0.7, graph_weight = 0.3)
     WHERE category = $cat AND status = 'published'
     ORDER BY score DESC
     LIMIT 20`,
    { cat: 'engineering' }
  );

  console.log(`Fusion results: ${result.results.length} docs`);
}

// ============================================================================
// 10. Agent memory: NEAR + scope filter + IN + ORDER + LIMIT
// ============================================================================

async function exampleAgentMemory(db: VelesDB): Promise<void> {
  console.log('\n=== 10. Agent Memory: Vector + Scope + Recency ===');

  const vec = mockEmbedding(75);
  const result = await db.query(
    'agent_memory',
    `SELECT content, role, timestamp, importance
     FROM agent_memory
     WHERE vector NEAR $v
       AND session_id = $sid
       AND role IN ('user', 'assistant')
       AND NOT archived = true
     ORDER BY timestamp DESC
     LIMIT 20`,
    { v: vec, sid: 'sess-abc-123' }
  );

  console.log(`Retrieved ${result.results.length} memory fragments`);
}

/**
 * Main: run all hybrid query examples
 */
async function main(): Promise<void> {
  console.log('='.repeat(60));
  console.log('VelesDB Hybrid Query Examples — TypeScript SDK');
  console.log('='.repeat(60));
  console.log('\nRequires velesdb-server running on localhost:3030');
  console.log('with collections: documents, articles, knowledge, social,');
  console.log('research, docs, orders, agent_memory\n');

  const db = new VelesDB({ backend: 'rest', url: 'http://localhost:3030' });
  await db.init();

  await exampleVectorWithFilters(db);
  await exampleSimilarityWithLike(db);
  await exampleMatchGraphVector(db);
  await exampleMatchMultiHopVector(db);
  await exampleMatchBidirectionalVector(db);
  await exampleMatchWithNear(db);
  await exampleMatchVariablePath(db);
  await exampleJoinVectorAggregation(db);
  await exampleFusionHybrid(db);
  await exampleAgentMemory(db);

  await db.close();
  console.log('\nDone.');
}

main().catch(console.error);

export {
  exampleVectorWithFilters,
  exampleSimilarityWithLike,
  exampleMatchGraphVector,
  exampleMatchMultiHopVector,
  exampleMatchBidirectionalVector,
  exampleMatchWithNear,
  exampleMatchVariablePath,
  exampleJoinVectorAggregation,
  exampleFusionHybrid,
  exampleAgentMemory,
};
