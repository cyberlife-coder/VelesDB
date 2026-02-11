/**
 * VelesQL Query Coverage Tests
 *
 * Exhaustive tests for ALL VelesQL query patterns through db.query().
 * Covers: NEAR, similarity(), WHERE operators, ORDER BY, LIMIT, OFFSET,
 * LIKE/ILIKE, IN, BETWEEN, NOT, NEAR_FUSED, LEFT/RIGHT JOIN,
 * and all hybrid combinations.
 *
 * Complements velesql-v2.test.ts (which focuses on GROUP BY, HAVING, UNION, FUSION).
 */

import { describe, it, expect, beforeEach, vi, Mock } from 'vitest';
import { VelesDB } from '../src/client';
import type { QueryResponse } from '../src/types';

describe('VelesQL Query Coverage', () => {
  let db: VelesDB;
  let mockQuery: Mock;

  const mockResponse: QueryResponse = {
    results: [
      {
        nodeId: 1, fusedScore: 0.95,
        vectorScore: 0.95, graphScore: null,
        bindings: {}, columnData: { title: 'doc1', category: 'tech' },
      },
      {
        nodeId: 2, fusedScore: 0.82,
        vectorScore: 0.82, graphScore: null,
        bindings: {}, columnData: { title: 'doc2', category: 'science' },
      },
    ],
    stats: {
      executionTimeMs: 1.2,
      strategy: 'vector_first',
      scannedNodes: 100,
    },
  };

  beforeEach(() => {
    db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
    (db as any).initialized = true;
    mockQuery = vi.fn().mockResolvedValue(mockResponse);
    (db as any).backend = { query: mockQuery };
  });

  // ==========================================================================
  // 1. Basic SELECT patterns
  // ==========================================================================

  describe('basic SELECT', () => {
    it('SELECT * FROM collection', async () => {
      await db.query('docs', 'SELECT * FROM docs');
      expect(mockQuery).toHaveBeenCalledWith('docs', 'SELECT * FROM docs', undefined, undefined);
    });

    it('SELECT with specific columns', async () => {
      await db.query('docs', 'SELECT id, title, category FROM docs');
      expect(mockQuery).toHaveBeenCalledWith(
        'docs', 'SELECT id, title, category FROM docs', undefined, undefined
      );
    });

    it('SELECT with column alias', async () => {
      await db.query('docs', 'SELECT title AS name, category AS type FROM docs');
      expect(mockQuery).toHaveBeenCalled();
    });
  });

  // ==========================================================================
  // 2. NEAR vector search
  // ==========================================================================

  describe('NEAR vector search', () => {
    const vec = [0.1, 0.2, 0.3, 0.4];

    it('basic NEAR', async () => {
      await db.query('docs', 'SELECT * FROM docs WHERE vector NEAR $v', { v: vec });
      expect(mockQuery).toHaveBeenCalledWith(
        'docs', 'SELECT * FROM docs WHERE vector NEAR $v', { v: vec }, undefined
      );
    });

    it('NEAR with LIMIT', async () => {
      await db.query('docs', 'SELECT * FROM docs WHERE vector NEAR $v LIMIT 10', { v: vec });
      expect(mockQuery).toHaveBeenCalledWith(
        'docs', 'SELECT * FROM docs WHERE vector NEAR $v LIMIT 10', { v: vec }, undefined
      );
    });

    it('NEAR with LIMIT and OFFSET (pagination)', async () => {
      await db.query(
        'docs',
        'SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 OFFSET 20',
        { v: vec }
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        'SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 OFFSET 20',
        { v: vec },
        undefined
      );
    });

    it('NEAR with WHERE filter', async () => {
      await db.query(
        'docs',
        'SELECT * FROM docs WHERE vector NEAR $v AND category = $cat LIMIT 10',
        { v: vec, cat: 'tech' }
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        'SELECT * FROM docs WHERE vector NEAR $v AND category = $cat LIMIT 10',
        { v: vec, cat: 'tech' },
        undefined
      );
    });

    it('NEAR with multiple WHERE conditions (AND + OR)', async () => {
      await db.query(
        'docs',
        'SELECT * FROM docs WHERE vector NEAR $v AND (category = $cat OR status = $s) LIMIT 5',
        { v: vec, cat: 'tech', s: 'published' }
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('AND (category = $cat OR status = $s)'),
        { v: vec, cat: 'tech', s: 'published' },
        undefined
      );
    });

    it('NEAR with ORDER BY similarity DESC', async () => {
      await db.query(
        'docs',
        'SELECT * FROM docs WHERE vector NEAR $v ORDER BY similarity(vector, $v) DESC LIMIT 20',
        { v: vec }
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('ORDER BY similarity(vector, $v) DESC'),
        { v: vec },
        undefined
      );
    });
  });

  // ==========================================================================
  // 3. similarity() threshold
  // ==========================================================================

  describe('similarity() threshold', () => {
    const vec = [0.5, 0.5, 0.5, 0.5];

    it('basic similarity threshold', async () => {
      await db.query(
        'docs',
        'SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8',
        { v: vec }
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        'SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8',
        { v: vec },
        undefined
      );
    });

    it('similarity with >= operator', async () => {
      await db.query(
        'docs',
        'SELECT * FROM docs WHERE similarity(embedding, $v) >= 0.9 LIMIT 5',
        { v: vec }
      );
      expect(mockQuery).toHaveBeenCalled();
    });

    it('similarity + WHERE filter + ORDER BY + LIMIT', async () => {
      await db.query(
        'docs',
        `SELECT * FROM docs 
         WHERE similarity(embedding, $v) > 0.7 AND category = $cat 
         ORDER BY similarity(embedding, $v) DESC 
         LIMIT 10`,
        { v: vec, cat: 'tech' }
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('similarity(embedding, $v) > 0.7 AND category = $cat'),
        { v: vec, cat: 'tech' },
        undefined
      );
    });

    it('similarity + ORDER BY non-vector column', async () => {
      await db.query(
        'docs',
        `SELECT * FROM docs 
         WHERE similarity(embedding, $v) > 0.6 
         ORDER BY created_at DESC 
         LIMIT 20`,
        { v: vec }
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('ORDER BY created_at DESC'),
        { v: vec },
        undefined
      );
    });
  });

  // ==========================================================================
  // 4. WHERE operators: LIKE, ILIKE, IN, BETWEEN, NOT
  // ==========================================================================

  describe('WHERE operators', () => {
    it('LIKE operator', async () => {
      await db.query(
        'docs',
        "SELECT * FROM docs WHERE title LIKE '%machine learning%'"
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('LIKE'),
        undefined,
        undefined
      );
    });

    it('ILIKE operator (case-insensitive)', async () => {
      await db.query(
        'docs',
        "SELECT * FROM docs WHERE title ILIKE '%VELESDB%'"
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('ILIKE'),
        undefined,
        undefined
      );
    });

    it('IN operator', async () => {
      await db.query(
        'docs',
        "SELECT * FROM docs WHERE category IN ('tech', 'science', 'ai')"
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('IN'),
        undefined,
        undefined
      );
    });

    it('NOT IN operator', async () => {
      await db.query(
        'docs',
        "SELECT * FROM docs WHERE category NOT IN ('spam', 'draft')"
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('NOT IN'),
        undefined,
        undefined
      );
    });

    it('BETWEEN operator', async () => {
      await db.query(
        'docs',
        'SELECT * FROM docs WHERE price BETWEEN 10 AND 100'
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('BETWEEN'),
        undefined,
        undefined
      );
    });

    it('NOT operator', async () => {
      await db.query(
        'docs',
        "SELECT * FROM docs WHERE NOT status = 'deleted'"
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('NOT status'),
        undefined,
        undefined
      );
    });

    it('IS NULL', async () => {
      await db.query(
        'docs',
        'SELECT * FROM docs WHERE description IS NULL'
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('IS NULL'),
        undefined,
        undefined
      );
    });

    it('comparison operators (>, <, >=, <=, !=)', async () => {
      await db.query(
        'docs',
        'SELECT * FROM docs WHERE price > 50 AND rating >= 4.0 AND stock != 0'
      );
      expect(mockQuery).toHaveBeenCalled();
    });
  });

  // ==========================================================================
  // 5. ORDER BY patterns
  // ==========================================================================

  describe('ORDER BY patterns', () => {
    it('ORDER BY ASC (default)', async () => {
      await db.query('docs', 'SELECT * FROM docs ORDER BY title ASC');
      expect(mockQuery).toHaveBeenCalled();
    });

    it('ORDER BY DESC', async () => {
      await db.query('docs', 'SELECT * FROM docs ORDER BY created_at DESC');
      expect(mockQuery).toHaveBeenCalled();
    });

    it('ORDER BY multiple columns mixed directions', async () => {
      await db.query(
        'docs',
        'SELECT * FROM docs ORDER BY category ASC, price DESC, title ASC'
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('category ASC, price DESC, title ASC'),
        undefined,
        undefined
      );
    });

    it('ORDER BY with LIMIT only', async () => {
      await db.query('docs', 'SELECT * FROM docs ORDER BY price DESC LIMIT 5');
      expect(mockQuery).toHaveBeenCalled();
    });

    it('ORDER BY with LIMIT + OFFSET', async () => {
      await db.query(
        'docs',
        'SELECT * FROM docs ORDER BY price DESC LIMIT 10 OFFSET 30'
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('LIMIT 10 OFFSET 30'),
        undefined,
        undefined
      );
    });
  });

  // ==========================================================================
  // 6. LIMIT / OFFSET pagination
  // ==========================================================================

  describe('LIMIT and OFFSET', () => {
    it('LIMIT only', async () => {
      await db.query('docs', 'SELECT * FROM docs LIMIT 50');
      expect(mockQuery).toHaveBeenCalled();
    });

    it('LIMIT with OFFSET', async () => {
      await db.query('docs', 'SELECT * FROM docs LIMIT 20 OFFSET 40');
      expect(mockQuery).toHaveBeenCalled();
    });

    it('OFFSET without LIMIT (edge case)', async () => {
      await db.query('docs', 'SELECT * FROM docs OFFSET 10');
      expect(mockQuery).toHaveBeenCalled();
    });
  });

  // ==========================================================================
  // 7. JOIN variants
  // ==========================================================================

  describe('JOIN variants', () => {
    it('LEFT JOIN', async () => {
      await db.query(
        'orders',
        'SELECT * FROM orders LEFT JOIN customers ON orders.cid = customers.id'
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'orders',
        expect.stringContaining('LEFT JOIN'),
        undefined,
        undefined
      );
    });

    it('RIGHT JOIN', async () => {
      await db.query(
        'orders',
        'SELECT * FROM orders RIGHT JOIN products ON orders.pid = products.id'
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'orders',
        expect.stringContaining('RIGHT JOIN'),
        undefined,
        undefined
      );
    });

    it('multiple JOINs', async () => {
      await db.query(
        'orders',
        `SELECT o.id, c.name, p.title 
         FROM orders AS o 
         JOIN customers AS c ON o.customer_id = c.id 
         JOIN products AS p ON o.product_id = p.id`
      );
      expect(mockQuery).toHaveBeenCalled();
    });

    it('JOIN + WHERE + ORDER BY + LIMIT', async () => {
      await db.query(
        'orders',
        `SELECT o.id, c.name, o.total 
         FROM orders AS o 
         JOIN customers AS c ON o.customer_id = c.id 
         WHERE o.status = $s 
         ORDER BY o.total DESC 
         LIMIT 25`,
        { s: 'completed' }
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'orders',
        expect.stringContaining('JOIN customers AS c'),
        { s: 'completed' },
        undefined
      );
    });
  });

  // ==========================================================================
  // 8. NEAR_FUSED (cross-collection hybrid)
  // ==========================================================================

  describe('NEAR_FUSED', () => {
    const vec = [0.3, 0.3, 0.3, 0.3];

    it('basic NEAR_FUSED', async () => {
      await db.query(
        'docs',
        'SELECT * FROM docs WHERE vector NEAR_FUSED $v LIMIT 10',
        { v: vec }
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('NEAR_FUSED'),
        { v: vec },
        undefined
      );
    });

    it('NEAR_FUSED with filter', async () => {
      await db.query(
        'docs',
        'SELECT * FROM docs WHERE vector NEAR_FUSED $v AND status = $s LIMIT 10',
        { v: vec, s: 'published' }
      );
      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('NEAR_FUSED $v AND status = $s'),
        { v: vec, s: 'published' },
        undefined
      );
    });
  });

  // ==========================================================================
  // 9. Hybrid combinations (the real-world queries)
  // ==========================================================================

  describe('hybrid combinations', () => {
    const vec = [0.1, 0.2, 0.3, 0.4];

    it('vector search + text filter + ORDER BY + LIMIT + OFFSET', async () => {
      const result = await db.query(
        'docs',
        `SELECT id, title, category FROM docs 
         WHERE vector NEAR $v AND category = $cat 
         ORDER BY similarity(vector, $v) DESC 
         LIMIT 10 OFFSET 20`,
        { v: vec, cat: 'tech' }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('NEAR $v AND category = $cat'),
        { v: vec, cat: 'tech' },
        undefined
      );
      expect(result.results).toBeDefined();
    });

    it('similarity threshold + multiple filters + ORDER BY column + LIMIT', async () => {
      await db.query(
        'docs',
        `SELECT * FROM docs 
         WHERE similarity(embedding, $v) > 0.7 
           AND category = $cat 
           AND price > $min 
         ORDER BY price ASC 
         LIMIT 15`,
        { v: vec, cat: 'tech', min: 10 }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('similarity(embedding, $v) > 0.7'),
        expect.objectContaining({ cat: 'tech', min: 10 }),
        undefined
      );
    });

    it('NEAR + JOIN + WHERE + ORDER + LIMIT', async () => {
      await db.query(
        'docs',
        `SELECT d.title, c.name 
         FROM docs AS d 
         JOIN categories AS c ON d.category_id = c.id 
         WHERE d.vector NEAR $v AND c.active = true 
         ORDER BY similarity(d.vector, $v) DESC 
         LIMIT 10`,
        { v: vec }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('JOIN categories AS c'),
        { v: vec },
        undefined
      );
    });

    it('NEAR + GROUP BY + HAVING + ORDER + LIMIT', async () => {
      await db.query(
        'docs',
        `SELECT category, COUNT(*), AVG(similarity(vector, $v)) AS avg_sim 
         FROM docs 
         WHERE vector NEAR $v 
         GROUP BY category 
         HAVING COUNT(*) > 3 
         ORDER BY avg_sim DESC 
         LIMIT 5`,
        { v: vec }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('GROUP BY category'),
        { v: vec },
        undefined
      );
    });

    it('similarity + LIKE + IN + ORDER + LIMIT', async () => {
      await db.query(
        'docs',
        `SELECT * FROM docs 
         WHERE similarity(embedding, $v) > 0.6 
           AND title LIKE '%AI%' 
           AND category IN ('tech', 'science') 
         ORDER BY similarity(embedding, $v) DESC 
         LIMIT 20`,
        { v: vec }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining("LIKE '%AI%'"),
        { v: vec },
        undefined
      );
    });

    it('NEAR + BETWEEN + NOT + ORDER + LIMIT', async () => {
      await db.query(
        'docs',
        `SELECT * FROM docs 
         WHERE vector NEAR $v 
           AND price BETWEEN 10 AND 100 
           AND NOT status = 'draft' 
         ORDER BY price DESC 
         LIMIT 30`,
        { v: vec }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('BETWEEN 10 AND 100'),
        { v: vec },
        undefined
      );
    });

    it('FUSION + WHERE + ORDER + LIMIT (hybrid vector + graph)', async () => {
      await db.query(
        'docs',
        `SELECT * FROM docs 
         USING FUSION(strategy = 'weighted', vector_weight = 0.7, graph_weight = 0.3) 
         WHERE category = $cat 
         ORDER BY score DESC 
         LIMIT 15`,
        { cat: 'tech' }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('USING FUSION'),
        { cat: 'tech' },
        undefined
      );
    });

    it('JOIN + GROUP BY + HAVING + ORDER BY + LIMIT + OFFSET (full analytics)', async () => {
      await db.query(
        'orders',
        `SELECT c.country, COUNT(*) AS total, SUM(o.amount) AS revenue 
         FROM orders AS o 
         JOIN customers AS c ON o.customer_id = c.id 
         WHERE o.created_at > $since 
         GROUP BY c.country 
         HAVING SUM(o.amount) > 1000 
         ORDER BY revenue DESC 
         LIMIT 10 OFFSET 0`,
        { since: '2025-01-01' }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'orders',
        expect.stringContaining('HAVING SUM(o.amount) > 1000'),
        { since: '2025-01-01' },
        undefined
      );
    });

    it('NEAR + ILIKE + multiple OR conditions + LIMIT', async () => {
      await db.query(
        'docs',
        `SELECT * FROM docs 
         WHERE vector NEAR $v 
           AND (title ILIKE '%vector%' OR title ILIKE '%database%') 
           AND status = $s 
         LIMIT 10`,
        { v: vec, s: 'published' }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('ILIKE'),
        { v: vec, s: 'published' },
        undefined
      );
    });

    it('subquery-style UNION + ORDER + LIMIT', async () => {
      await db.query(
        'docs',
        `SELECT * FROM recent_docs 
         UNION 
         SELECT * FROM popular_docs 
         ORDER BY created_at DESC 
         LIMIT 20`
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('UNION'),
        undefined,
        undefined
      );
    });

    it('vector search + IS NULL check + ORDER + LIMIT', async () => {
      await db.query(
        'docs',
        `SELECT * FROM docs 
         WHERE vector NEAR $v AND summary IS NULL 
         ORDER BY similarity(vector, $v) DESC 
         LIMIT 5`,
        { v: vec }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('IS NULL'),
        { v: vec },
        undefined
      );
    });
  });

  // ==========================================================================
  // 10. Parameter binding edge cases
  // ==========================================================================

  describe('parameter binding', () => {
    it('multiple named parameters', async () => {
      await db.query(
        'docs',
        'SELECT * FROM docs WHERE category = $cat AND price > $min AND price < $max LIMIT $lim',
        { cat: 'tech', min: 10, max: 100, lim: 20 }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('$cat'),
        { cat: 'tech', min: 10, max: 100, lim: 20 },
        undefined
      );
    });

    it('vector param as Float32Array', async () => {
      const vec = new Float32Array([0.1, 0.2, 0.3, 0.4]);
      await db.query(
        'docs',
        'SELECT * FROM docs WHERE vector NEAR $v LIMIT 10',
        { v: vec }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('NEAR $v'),
        { v: vec },
        undefined
      );
    });

    it('empty params object', async () => {
      await db.query('docs', 'SELECT * FROM docs LIMIT 10', {});
      expect(mockQuery).toHaveBeenCalledWith(
        'docs', 'SELECT * FROM docs LIMIT 10', {}, undefined
      );
    });

    it('no params (undefined)', async () => {
      await db.query('docs', 'SELECT COUNT(*) FROM docs');
      expect(mockQuery).toHaveBeenCalledWith(
        'docs', 'SELECT COUNT(*) FROM docs', undefined, undefined
      );
    });
  });

  // ==========================================================================
  // 11. RAG-specific patterns (real-world use cases)
  // ==========================================================================

  describe('RAG-specific patterns', () => {
    const queryVec = [0.1, 0.2, 0.3, 0.4];

    it('contextual RAG: similarity + recency + LIMIT', async () => {
      await db.query(
        'knowledge_base',
        `SELECT * FROM knowledge_base 
         WHERE similarity(embedding, $v) > 0.75 
           AND updated_at > $since 
         ORDER BY similarity(embedding, $v) DESC 
         LIMIT 5`,
        { v: queryVec, since: '2025-06-01' }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'knowledge_base',
        expect.stringContaining('similarity(embedding, $v) > 0.75'),
        expect.objectContaining({ since: '2025-06-01' }),
        undefined
      );
    });

    it('multi-tenant RAG: NEAR + tenant filter + LIMIT', async () => {
      await db.query(
        'docs',
        `SELECT * FROM docs 
         WHERE vector NEAR $v AND tenant_id = $tid 
         LIMIT 10`,
        { v: queryVec, tid: 'org-123' }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('tenant_id = $tid'),
        { v: queryVec, tid: 'org-123' },
        undefined
      );
    });

    it('agent memory: NEAR + conversation scope + recency order', async () => {
      await db.query(
        'agent_memory',
        `SELECT * FROM agent_memory 
         WHERE vector NEAR $v 
           AND session_id = $sid 
           AND role IN ('user', 'assistant') 
         ORDER BY timestamp DESC 
         LIMIT 20`,
        { v: queryVec, sid: 'sess-abc' }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'agent_memory',
        expect.stringContaining("IN ('user', 'assistant')"),
        expect.objectContaining({ sid: 'sess-abc' }),
        undefined
      );
    });

    it('graph-augmented RAG: FUSION + filter + LIMIT', async () => {
      await db.query(
        'docs',
        `SELECT * FROM docs 
         USING FUSION(strategy = 'rrf', k = 60) 
         WHERE category = $cat 
         LIMIT 10`,
        { cat: 'engineering' }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining("FUSION(strategy = 'rrf'"),
        { cat: 'engineering' },
        undefined
      );
    });

    it('semantic dedup: similarity threshold + GROUP BY', async () => {
      await db.query(
        'docs',
        `SELECT title, COUNT(*) AS dupes 
         FROM docs 
         WHERE similarity(embedding, $v) > 0.95 
         GROUP BY title 
         HAVING COUNT(*) > 1 
         ORDER BY dupes DESC`,
        { v: queryVec }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('HAVING COUNT(*) > 1'),
        { v: queryVec },
        undefined
      );
    });
  });

  // ==========================================================================
  // 12. MATCH graph queries (hybrid: graph + vector + multi-column)
  // ==========================================================================

  describe('MATCH graph queries (hybrid)', () => {
    let mockMatchQuery: Mock;
    const vec = [0.1, 0.2, 0.3, 0.4];

    const mockMatchResponse = {
      results: [
        { bindings: { a: 1, b: 2 }, score: 0.92, depth: 1, projected: { 'a.name': 'Alice', 'b.name': 'Bob' } },
        { bindings: { a: 1, b: 3 }, score: 0.85, depth: 2, projected: { 'a.name': 'Alice', 'b.name': 'Carol' } },
      ],
      tookMs: 3.2,
      count: 2,
    };

    beforeEach(() => {
      mockMatchQuery = vi.fn().mockResolvedValue(mockMatchResponse);
      (db as any).backend.matchQuery = mockMatchQuery;
    });

    it('MATCH + similarity + multi-column RETURN', async () => {
      const result = await db.matchQuery(
        'knowledge',
        `MATCH (doc:Document)-[:REFERENCES]->(ref:Document)
         WHERE similarity(doc.embedding, $v) > 0.8
         RETURN doc.title, doc.category, ref.title, ref.author`,
        { v: vec },
        { vector: vec, threshold: 0.8 }
      );

      expect(mockMatchQuery).toHaveBeenCalledWith(
        'knowledge',
        expect.stringContaining('RETURN doc.title, doc.category, ref.title, ref.author'),
        { v: vec },
        { vector: vec, threshold: 0.8 }
      );
      expect(result.results).toHaveLength(2);
      expect(result.results[0].projected).toBeDefined();
    });

    it('MATCH + similarity + WHERE filter + ORDER BY + LIMIT', async () => {
      await db.matchQuery(
        'docs',
        `MATCH (a:Person)-[:AUTHORED]->(d:Document)
         WHERE similarity(d.embedding, $v) > 0.7 AND d.status = 'published'
         RETURN a.name, d.title, d.category
         ORDER BY similarity() DESC
         LIMIT 20`,
        { v: vec },
        { vector: vec, threshold: 0.7 }
      );

      expect(mockMatchQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining("d.status = 'published'"),
        { v: vec },
        { vector: vec, threshold: 0.7 }
      );
    });

    it('MATCH multi-hop + similarity + multi-column projection', async () => {
      await db.matchQuery(
        'social',
        `MATCH (u:User)-[:FOLLOWS]->(f:User)-[:POSTED]->(p:Post)
         WHERE similarity(p.embedding, $v) > 0.75
         RETURN u.name, f.name, p.title, p.category, p.created_at
         ORDER BY similarity() DESC
         LIMIT 10`,
        { v: vec },
        { vector: vec, threshold: 0.75 }
      );

      expect(mockMatchQuery).toHaveBeenCalledWith(
        'social',
        expect.stringContaining('[:FOLLOWS]->(f:User)-[:POSTED]->(p:Post)'),
        { v: vec },
        { vector: vec, threshold: 0.75 }
      );
    });

    it('MATCH with multiple WHERE conditions + vector + RETURN fields', async () => {
      await db.matchQuery(
        'incidents',
        `MATCH (incident:Ticket)-[:IMPACTS]->(service:Microservice)
         WHERE similarity(incident.log_embedding, $v) > 0.85
           AND incident.status = 'RESOLVED'
           AND incident.severity > 3
         RETURN incident.solution, incident.severity, service.name, service.region
         ORDER BY similarity() DESC
         LIMIT 5`,
        { v: vec },
        { vector: vec, threshold: 0.85 }
      );

      expect(mockMatchQuery).toHaveBeenCalledWith(
        'incidents',
        expect.stringContaining("incident.status = 'RESOLVED'"),
        { v: vec },
        { vector: vec, threshold: 0.85 }
      );
    });

    it('MATCH variable-length path + similarity + multi-column', async () => {
      await db.matchQuery(
        'knowledge',
        `MATCH (concept:Topic)-[:RELATED_TO*1..3]->(related:Topic)
         WHERE similarity(concept.embedding, $v) > 0.6
         RETURN concept.name, concept.domain, related.name, related.domain
         LIMIT 50`,
        { v: vec },
        { vector: vec, threshold: 0.6 }
      );

      expect(mockMatchQuery).toHaveBeenCalledWith(
        'knowledge',
        expect.stringContaining('[:RELATED_TO*1..3]'),
        { v: vec },
        { vector: vec, threshold: 0.6 }
      );
    });

    it('MATCH bidirectional + vector + filters + multi-column', async () => {
      await db.matchQuery(
        'collab',
        `MATCH (a:Researcher)-[:COLLABORATES]-(b:Researcher)-[:PUBLISHED]->(p:Paper)
         WHERE similarity(p.abstract_emb, $v) > 0.7
           AND p.year >= 2024
           AND a.institution = $inst
         RETURN a.name, a.institution, b.name, p.title, p.journal, p.year
         ORDER BY similarity() DESC
         LIMIT 15`,
        { v: vec, inst: 'MIT' },
        { vector: vec, threshold: 0.7 }
      );

      expect(mockMatchQuery).toHaveBeenCalledWith(
        'collab',
        expect.stringContaining('[:COLLABORATES]-(b:Researcher)-[:PUBLISHED]'),
        { v: vec, inst: 'MIT' },
        { vector: vec, threshold: 0.7 }
      );
    });

    it('MATCH + NEAR vector (without similarity threshold)', async () => {
      await db.matchQuery(
        'docs',
        `MATCH (a:Author)-[:WROTE]->(d:Document)
         WHERE d.vector NEAR $v AND d.category = $cat
         RETURN a.name, a.email, d.title, d.category, d.published_at
         LIMIT 10`,
        { v: vec, cat: 'AI' }
      );

      expect(mockMatchQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('NEAR $v AND d.category = $cat'),
        { v: vec, cat: 'AI' },
        undefined
      );
    });

    it('MATCH response has correct structure with multi-column projected', async () => {
      const result = await db.matchQuery(
        'knowledge',
        `MATCH (a)-[:KNOWS]->(b) RETURN a.name, b.name`,
      );

      expect(result.results).toHaveLength(2);
      expect(result.results[0].projected).toHaveProperty('a.name');
      expect(result.results[0].projected).toHaveProperty('b.name');
      expect(result.results[0].score).toBe(0.92);
      expect(result.results[0].depth).toBe(1);
      expect(result.tookMs).toBeGreaterThan(0);
      expect(result.count).toBe(2);
    });
  });

  // ==========================================================================
  // 13. Query response handling
  // ==========================================================================

  describe('response structure', () => {
    it('should return results with columnData', async () => {
      const result = await db.query('docs', 'SELECT * FROM docs LIMIT 5');
      expect(result.results).toHaveLength(2);
      expect(result.results[0].nodeId).toBe(1);
      expect(result.results[0].columnData).toEqual({ title: 'doc1', category: 'tech' });
      expect(result.results[0].vectorScore).toBe(0.95);
      expect(result.results[0].bindings).toEqual({});
    });

    it('should return stats with strategy and scannedNodes', async () => {
      const result = await db.query('docs', 'SELECT * FROM docs LIMIT 5');
      expect(result.stats.executionTimeMs).toBeGreaterThan(0);
      expect(result.stats.strategy).toBe('vector_first');
      expect(result.stats.scannedNodes).toBe(100);
    });

    it('should return fusedScore', async () => {
      const result = await db.query('docs', 'SELECT * FROM docs LIMIT 5');
      expect(result.results[0].fusedScore).toBe(0.95);
      expect(result.results[1].fusedScore).toBe(0.82);
    });
  });
});
