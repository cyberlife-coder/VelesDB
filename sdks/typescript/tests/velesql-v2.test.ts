/**
 * VelesQL v2.0 Tests (EPIC-016 US-051)
 * 
 * Tests for VelesQL v2.0 features:
 * - GROUP BY with aggregates
 * - HAVING with AND/OR
 * - ORDER BY multi-column + similarity()
 * - JOIN with aliases
 * - UNION/INTERSECT/EXCEPT
 * - WITH (max_groups)
 * - USING FUSION
 */

import { describe, it, expect, beforeEach, vi, Mock } from 'vitest';
import { VelesDB } from '../src/client';
import type { QueryResponse, QueryResult } from '../src/types';

describe('VelesQL v2.0', () => {
  let db: VelesDB;
  let mockQuery: Mock;

  const mockQueryResponse: QueryResponse = {
    results: [
      { nodeId: 1, payload: { category: 'tech', count: 5 }, fusedScore: 0.95 },
      { nodeId: 2, payload: { category: 'science', count: 3 }, fusedScore: 0.85 },
    ],
    stats: {
      totalResults: 2,
      executionTimeMs: 1.5,
      vectorSearchMs: 0.8,
      graphTraversalMs: 0.5,
    },
  };

  beforeEach(() => {
    db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
    (db as any).initialized = true;
    
    mockQuery = vi.fn().mockResolvedValue(mockQueryResponse);
    (db as any).backend = { query: mockQuery };
  });

  describe('GROUP BY with aggregates', () => {
    it('should execute GROUP BY with COUNT', async () => {
      const result = await db.query(
        'products',
        'SELECT category, COUNT(*) FROM products GROUP BY category'
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'products',
        'SELECT category, COUNT(*) FROM products GROUP BY category',
        undefined,
        undefined
      );
      expect(result.results).toBeDefined();
      expect(result.results.length).toBe(2);
    });

    it('should execute GROUP BY with multiple aggregates', async () => {
      const result = await db.query(
        'products',
        'SELECT category, COUNT(*), SUM(price), AVG(rating) FROM products GROUP BY category'
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'products',
        'SELECT category, COUNT(*), SUM(price), AVG(rating) FROM products GROUP BY category',
        undefined,
        undefined
      );
      expect(result.results).toBeDefined();
    });

    it('should execute GROUP BY with MIN/MAX', async () => {
      const result = await db.query(
        'products',
        'SELECT category, MIN(price), MAX(price) FROM products GROUP BY category'
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });
  });

  describe('HAVING clause', () => {
    it('should execute HAVING with single condition', async () => {
      const result = await db.query(
        'products',
        'SELECT category, COUNT(*) FROM products GROUP BY category HAVING COUNT(*) > 5'
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'products',
        'SELECT category, COUNT(*) FROM products GROUP BY category HAVING COUNT(*) > 5',
        undefined,
        undefined
      );
      expect(result.results).toBeDefined();
    });

    it('should execute HAVING with AND operator', async () => {
      const result = await db.query(
        'products',
        'SELECT category, COUNT(*), AVG(price) FROM products GROUP BY category HAVING COUNT(*) > 5 AND AVG(price) > 50'
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });

    it('should execute HAVING with OR operator', async () => {
      const result = await db.query(
        'products',
        'SELECT category, COUNT(*) FROM products GROUP BY category HAVING COUNT(*) > 10 OR COUNT(*) < 2'
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });
  });

  describe('ORDER BY enhancements', () => {
    it('should execute ORDER BY with single column', async () => {
      const result = await db.query(
        'products',
        'SELECT * FROM products ORDER BY price DESC'
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });

    it('should execute ORDER BY with multiple columns', async () => {
      const result = await db.query(
        'products',
        'SELECT * FROM products ORDER BY category ASC, price DESC'
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });

    it('should execute ORDER BY with similarity function', async () => {
      const queryVector = [0.1, 0.2, 0.3, 0.4];
      const result = await db.query(
        'docs',
        'SELECT * FROM docs ORDER BY similarity(vector, $v) DESC LIMIT 10',
        { v: queryVector }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        'SELECT * FROM docs ORDER BY similarity(vector, $v) DESC LIMIT 10',
        { v: queryVector },
        undefined
      );
      expect(result.results).toBeDefined();
    });
  });

  describe('JOIN clause', () => {
    it('should execute basic JOIN', async () => {
      const result = await db.query(
        'orders',
        'SELECT * FROM orders JOIN customers ON orders.customer_id = customers.id'
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });

    it('should execute JOIN with alias', async () => {
      const result = await db.query(
        'orders',
        'SELECT * FROM orders JOIN customers AS c ON orders.customer_id = c.id'
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });

    it('should execute JOIN with WHERE clause', async () => {
      const result = await db.query(
        'orders',
        'SELECT * FROM orders JOIN customers AS c ON orders.customer_id = c.id WHERE status = $status',
        { status: 'active' }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'orders',
        'SELECT * FROM orders JOIN customers AS c ON orders.customer_id = c.id WHERE status = $status',
        { status: 'active' },
        undefined
      );
      expect(result.results).toBeDefined();
    });
  });

  describe('Set operations', () => {
    it('should execute UNION', async () => {
      const result = await db.query(
        'users',
        'SELECT * FROM active_users UNION SELECT * FROM archived_users'
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });

    it('should execute UNION ALL', async () => {
      const result = await db.query(
        'users',
        'SELECT * FROM active_users UNION ALL SELECT * FROM archived_users'
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });

    it('should execute INTERSECT', async () => {
      const result = await db.query(
        'users',
        'SELECT id FROM premium_users INTERSECT SELECT id FROM active_users'
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });

    it('should execute EXCEPT', async () => {
      const result = await db.query(
        'users',
        'SELECT id FROM all_users EXCEPT SELECT id FROM banned_users'
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });
  });

  describe('WITH clause options', () => {
    it('should execute WITH max_groups', async () => {
      const result = await db.query(
        'products',
        'SELECT category, COUNT(*) FROM products WITH (max_groups = 100) GROUP BY category'
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });

    it('should execute WITH group_limit', async () => {
      const result = await db.query(
        'products',
        'SELECT category, COUNT(*) FROM products WITH (group_limit = 50) GROUP BY category'
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });
  });

  describe('USING FUSION hybrid search', () => {
    it('should execute with default FUSION (RRF)', async () => {
      const result = await db.query(
        'docs',
        'SELECT * FROM docs USING FUSION LIMIT 20'
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });

    it('should execute with FUSION RRF strategy', async () => {
      const result = await db.query(
        'docs',
        "SELECT * FROM docs USING FUSION(strategy = 'rrf', k = 60) LIMIT 20"
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });

    it('should execute with FUSION weighted strategy', async () => {
      const result = await db.query(
        'docs',
        "SELECT * FROM docs USING FUSION(strategy = 'weighted', vector_weight = 0.7, graph_weight = 0.3) LIMIT 20"
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });

    it('should execute with FUSION maximum strategy', async () => {
      const result = await db.query(
        'docs',
        "SELECT * FROM docs USING FUSION(strategy = 'maximum') LIMIT 20"
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });
  });

  describe('Complex queries combining v2.0 features', () => {
    it('should execute complex analytics query', async () => {
      const result = await db.query(
        'orders',
        `SELECT category, COUNT(*), AVG(amount) 
         FROM orders 
         JOIN products AS p ON orders.product_id = p.id 
         GROUP BY category 
         HAVING COUNT(*) > 10 AND AVG(amount) > 100 
         ORDER BY COUNT(*) DESC 
         LIMIT 20`
      );

      expect(mockQuery).toHaveBeenCalled();
      expect(result.results).toBeDefined();
    });

    it('should execute semantic search with aggregation', async () => {
      const queryVector = [0.1, 0.2, 0.3, 0.4];
      const result = await db.query(
        'docs',
        `SELECT category, COUNT(*) 
         FROM docs 
         WHERE vector NEAR $v 
         GROUP BY category 
         ORDER BY similarity(vector, $v) DESC 
         LIMIT 10`,
        { v: queryVector }
      );

      expect(mockQuery).toHaveBeenCalledWith(
        'docs',
        expect.stringContaining('GROUP BY category'),
        { v: queryVector },
        undefined
      );
      expect(result.results).toBeDefined();
    });
  });
});
