/**
 * SelectBuilder TDD Tests
 * 
 * Tests for the fluent SELECT query builder — companion to VelesQLBuilder (MATCH).
 */

import { describe, it, expect } from 'vitest';
import { SelectBuilder, selectql } from '../src/select-builder';

describe('SelectBuilder', () => {

  // ========================================================================
  // Basic SELECT
  // ========================================================================

  describe('basic SELECT', () => {
    it('should build SELECT * FROM collection', () => {
      const { query } = selectql().from('docs').build();
      expect(query).toBe('SELECT * FROM docs');
    });

    it('should build SELECT with specific columns', () => {
      const { query } = selectql().select('name', 'age').from('users').build();
      expect(query).toBe('SELECT name, age FROM users');
    });

    it('should build SELECT with column aliases', () => {
      const { query } = selectql()
        .selectAs('first_name', 'name')
        .selectAs('birth_year', 'year')
        .from('users')
        .build();
      expect(query).toBe('SELECT first_name AS name, birth_year AS year FROM users');
    });

    it('should combine select() and selectAs()', () => {
      const { query } = selectql()
        .select('id')
        .selectAs('first_name', 'name')
        .from('users')
        .build();
      expect(query).toBe('SELECT id, first_name AS name FROM users');
    });

    it('should use selectAll() explicitly', () => {
      const { query } = selectql().selectAll().from('docs').build();
      expect(query).toBe('SELECT * FROM docs');
    });
  });

  // ========================================================================
  // WHERE
  // ========================================================================

  describe('WHERE', () => {
    it('should add simple WHERE condition', () => {
      const { query } = selectql()
        .from('users')
        .where('age > 18')
        .build();
      expect(query).toBe('SELECT * FROM users WHERE age > 18');
    });

    it('should add WHERE with params', () => {
      const result = selectql()
        .from('users')
        .where('age > $min_age', { min_age: 18 })
        .build();
      expect(result.query).toBe('SELECT * FROM users WHERE age > $min_age');
      expect(result.params).toEqual({ min_age: 18 });
    });

    it('should chain AND WHERE', () => {
      const { query } = selectql()
        .from('users')
        .where('age > 18')
        .andWhere('status = $status', { status: 'active' })
        .build();
      expect(query).toBe("SELECT * FROM users WHERE age > 18 AND status = $status");
    });

    it('should chain OR WHERE', () => {
      const { query } = selectql()
        .from('users')
        .where('role = $r1', { r1: 'admin' })
        .orWhere('role = $r2', { r2: 'moderator' })
        .build();
      expect(query).toBe("SELECT * FROM users WHERE role = $r1 OR role = $r2");
    });

    it('should combine AND and OR WHERE', () => {
      const { query } = selectql()
        .from('users')
        .where('age > 18')
        .andWhere('status = $s', { s: 'active' })
        .orWhere('role = $r', { r: 'admin' })
        .build();
      expect(query).toBe("SELECT * FROM users WHERE age > 18 AND status = $s OR role = $r");
    });
  });

  // ========================================================================
  // Vector Search
  // ========================================================================

  describe('vector search', () => {
    it('should build NEAR vector clause', () => {
      const vec = [0.1, 0.2, 0.3];
      const result = selectql()
        .from('docs')
        .nearVector('v', vec, { topK: 5 })
        .build();
      expect(result.query).toBe('SELECT * FROM docs WHERE NEAR($v, 5)');
      expect(result.params).toEqual({ v: vec });
    });

    it('should build NEAR with default topK', () => {
      const vec = [0.1, 0.2];
      const result = selectql()
        .from('docs')
        .nearVector('v', vec)
        .build();
      expect(result.query).toBe('SELECT * FROM docs WHERE NEAR($v, 10)');
      expect(result.params).toEqual({ v: vec });
    });

    it('should build similarity() clause', () => {
      const vec = [0.1, 0.2, 0.3];
      const result = selectql()
        .from('docs')
        .similarity('embedding', 'v', vec, { threshold: 0.8 })
        .build();
      expect(result.query).toBe('SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8');
      expect(result.params).toEqual({ v: vec });
    });

    it('should build similarity() with default threshold', () => {
      const vec = [0.1, 0.2];
      const result = selectql()
        .from('docs')
        .similarity('embedding', 'v', vec)
        .build();
      expect(result.query).toBe('SELECT * FROM docs WHERE similarity(embedding, $v) > 0');
      expect(result.params).toEqual({ v: vec });
    });

    it('should accept Float32Array for vectors', () => {
      const vec = new Float32Array([0.1, 0.2]);
      const result = selectql()
        .from('docs')
        .nearVector('v', vec)
        .build();
      expect(result.query).toBe('SELECT * FROM docs WHERE NEAR($v, 10)');
      expect(result.params.v).toEqual(Array.from(vec));
    });
  });

  // ========================================================================
  // Aggregation
  // ========================================================================

  describe('aggregation', () => {
    it('should build SELECT COUNT(*)', () => {
      const { query } = selectql()
        .selectAgg('COUNT', '*')
        .from('users')
        .build();
      expect(query).toBe('SELECT COUNT(*) FROM users');
    });

    it('should build SELECT with multiple aggregations', () => {
      const { query } = selectql()
        .selectAgg('COUNT', '*', 'total')
        .selectAgg('AVG', 'price', 'avg_price')
        .from('products')
        .build();
      expect(query).toBe('SELECT COUNT(*) AS total, AVG(price) AS avg_price FROM products');
    });

    it('should build aggregation with alias', () => {
      const { query } = selectql()
        .selectAgg('SUM', 'amount', 'total_amount')
        .from('orders')
        .build();
      expect(query).toBe('SELECT SUM(amount) AS total_amount FROM orders');
    });

    it('should support MIN and MAX', () => {
      const { query } = selectql()
        .selectAgg('MIN', 'price', 'cheapest')
        .selectAgg('MAX', 'price', 'most_expensive')
        .from('products')
        .build();
      expect(query).toBe('SELECT MIN(price) AS cheapest, MAX(price) AS most_expensive FROM products');
    });
  });

  // ========================================================================
  // GROUP BY
  // ========================================================================

  describe('GROUP BY', () => {
    it('should build GROUP BY single column', () => {
      const { query } = selectql()
        .select('category')
        .selectAgg('COUNT', '*', 'total')
        .from('products')
        .groupBy('category')
        .build();
      expect(query).toBe('SELECT category, COUNT(*) AS total FROM products GROUP BY category');
    });

    it('should build GROUP BY multiple columns', () => {
      const { query } = selectql()
        .select('category', 'brand')
        .selectAgg('COUNT', '*')
        .from('products')
        .groupBy('category', 'brand')
        .build();
      expect(query).toBe('SELECT category, brand, COUNT(*) FROM products GROUP BY category, brand');
    });
  });

  // ========================================================================
  // ORDER BY, LIMIT, OFFSET
  // ========================================================================

  describe('ORDER BY, LIMIT, OFFSET', () => {
    it('should build ORDER BY single column ASC (default)', () => {
      const { query } = selectql().from('users').orderBy('name').build();
      expect(query).toBe('SELECT * FROM users ORDER BY name ASC');
    });

    it('should build ORDER BY DESC', () => {
      const { query } = selectql().from('users').orderBy('created_at', 'DESC').build();
      expect(query).toBe('SELECT * FROM users ORDER BY created_at DESC');
    });

    it('should build ORDER BY multiple columns', () => {
      const { query } = selectql()
        .from('users')
        .orderBy('name', 'ASC')
        .orderBy('age', 'DESC')
        .build();
      expect(query).toBe('SELECT * FROM users ORDER BY name ASC, age DESC');
    });

    it('should build LIMIT', () => {
      const { query } = selectql().from('users').limit(10).build();
      expect(query).toBe('SELECT * FROM users LIMIT 10');
    });

    it('should build OFFSET', () => {
      const { query } = selectql().from('users').offset(20).build();
      expect(query).toBe('SELECT * FROM users OFFSET 20');
    });

    it('should build LIMIT + OFFSET', () => {
      const { query } = selectql().from('users').limit(10).offset(20).build();
      expect(query).toBe('SELECT * FROM users LIMIT 10 OFFSET 20');
    });
  });

  // ========================================================================
  // JOIN
  // ========================================================================

  describe('JOIN', () => {
    it('should build INNER JOIN', () => {
      const { query } = selectql()
        .from('orders')
        .join('products', 'orders.product_id = products.id')
        .build();
      expect(query).toBe('SELECT * FROM orders INNER JOIN products ON orders.product_id = products.id');
    });

    it('should build LEFT JOIN', () => {
      const { query } = selectql()
        .from('users')
        .join('orders', 'users.id = orders.user_id', 'LEFT')
        .build();
      expect(query).toBe('SELECT * FROM users LEFT JOIN orders ON users.id = orders.user_id');
    });

    it('should build RIGHT JOIN', () => {
      const { query } = selectql()
        .from('orders')
        .join('users', 'orders.user_id = users.id', 'RIGHT')
        .build();
      expect(query).toBe('SELECT * FROM orders RIGHT JOIN users ON orders.user_id = users.id');
    });
  });

  // ========================================================================
  // Complex queries
  // ========================================================================

  describe('complex queries', () => {
    it('should build full query with WHERE + ORDER BY + LIMIT', () => {
      const result = selectql()
        .select('name', 'age')
        .from('users')
        .where('age > $min', { min: 18 })
        .andWhere('status = $s', { s: 'active' })
        .orderBy('name', 'ASC')
        .limit(50)
        .offset(10)
        .build();
      expect(result.query).toBe(
        'SELECT name, age FROM users WHERE age > $min AND status = $s ORDER BY name ASC LIMIT 50 OFFSET 10'
      );
      expect(result.params).toEqual({ min: 18, s: 'active' });
    });

    it('should build aggregation with GROUP BY + WHERE', () => {
      const result = selectql()
        .select('category')
        .selectAgg('AVG', 'price', 'avg_price')
        .selectAgg('COUNT', '*', 'total')
        .from('products')
        .where('price > $min', { min: 0 })
        .groupBy('category')
        .orderBy('avg_price', 'DESC')
        .limit(10)
        .build();
      expect(result.query).toBe(
        'SELECT category, AVG(price) AS avg_price, COUNT(*) AS total FROM products WHERE price > $min GROUP BY category ORDER BY avg_price DESC LIMIT 10'
      );
      expect(result.params).toEqual({ min: 0 });
    });

    it('should build vector search with filter + LIMIT', () => {
      const vec = [0.1, 0.2, 0.3];
      const result = selectql()
        .from('docs')
        .similarity('embedding', 'v', vec, { threshold: 0.8 })
        .andWhere('category = $cat', { cat: 'science' })
        .limit(10)
        .build();
      expect(result.query).toBe(
        'SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8 AND category = $cat LIMIT 10'
      );
      expect(result.params).toEqual({ v: vec, cat: 'science' });
    });
  });

  // ========================================================================
  // Params
  // ========================================================================

  describe('params', () => {
    it('should return all bound params via getParams()', () => {
      const builder = selectql()
        .from('users')
        .where('age > $min', { min: 18 })
        .andWhere('status = $s', { s: 'active' });
      expect(builder.getParams()).toEqual({ min: 18, s: 'active' });
    });

    it('should return empty params when none set', () => {
      const builder = selectql().from('docs');
      expect(builder.getParams()).toEqual({});
    });

    it('should merge vector params with WHERE params', () => {
      const vec = [0.1, 0.2];
      const builder = selectql()
        .from('docs')
        .nearVector('v', vec)
        .andWhere('type = $t', { t: 'article' });
      expect(builder.getParams()).toEqual({ v: vec, t: 'article' });
    });
  });

  // ========================================================================
  // Immutability
  // ========================================================================

  describe('immutability', () => {
    it('should not mutate the original builder', () => {
      const base = selectql().from('docs');
      const withLimit = base.limit(10);
      const withOrder = base.orderBy('name');

      expect(base.build().query).toBe('SELECT * FROM docs');
      expect(withLimit.build().query).toBe('SELECT * FROM docs LIMIT 10');
      expect(withOrder.build().query).toBe('SELECT * FROM docs ORDER BY name ASC');
    });
  });

  // ========================================================================
  // Errors
  // ========================================================================

  describe('errors', () => {
    it('should throw when build() is called without FROM', () => {
      expect(() => selectql().select('name').build()).toThrow();
    });

    it('should throw when FROM is empty string', () => {
      expect(() => selectql().from('').build()).toThrow();
    });
  });
});
