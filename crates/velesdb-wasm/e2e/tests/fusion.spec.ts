import { test, expect } from '@playwright/test';

/**
 * Multi-Query Fusion E2E Tests for WASM SDK
 * 
 * Tests multi-query search with different fusion strategies.
 * EPIC-060: Complete E2E test coverage
 */
test.describe('VelesDB WASM Multi-Query Fusion', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => window['VelesDB']?.ready === true, { timeout: 10000 });
  });

  test('should perform multi-query search with RRF fusion', async ({ page }) => {
    const results = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(4, 'cosine');

      // Insert test vectors
      store.insert(1, new Float32Array([1.0, 0.0, 0.0, 0.0]));
      store.insert(2, new Float32Array([0.9, 0.1, 0.0, 0.0]));
      store.insert(3, new Float32Array([0.5, 0.5, 0.0, 0.0]));
      store.insert(4, new Float32Array([0.0, 1.0, 0.0, 0.0]));
      store.insert(5, new Float32Array([0.1, 0.9, 0.0, 0.0]));

      // Multi-query search with 2 queries
      const query1 = new Float32Array([1.0, 0.0, 0.0, 0.0]);
      const query2 = new Float32Array([0.0, 1.0, 0.0, 0.0]);
      
      const searchResults = store.multi_query_search([query1, query2], 5, 'rrf');
      return searchResults;
    });

    expect(results).toBeDefined();
    expect(results.length).toBeGreaterThan(0);
  });

  test('should perform multi-query search with average fusion', async ({ page }) => {
    const results = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(4, 'cosine');

      store.insert(1, new Float32Array([1.0, 0.0, 0.0, 0.0]));
      store.insert(2, new Float32Array([0.0, 1.0, 0.0, 0.0]));

      const query1 = new Float32Array([1.0, 0.0, 0.0, 0.0]);
      const query2 = new Float32Array([0.5, 0.5, 0.0, 0.0]);
      
      return store.multi_query_search([query1, query2], 2, 'average');
    });

    expect(results).toBeDefined();
  });

  test('should perform multi-query search with maximum fusion', async ({ page }) => {
    const results = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(4, 'cosine');

      store.insert(1, new Float32Array([1.0, 0.0, 0.0, 0.0]));
      store.insert(2, new Float32Array([0.0, 1.0, 0.0, 0.0]));

      const query1 = new Float32Array([1.0, 0.0, 0.0, 0.0]);
      const query2 = new Float32Array([0.0, 1.0, 0.0, 0.0]);
      
      return store.multi_query_search([query1, query2], 2, 'maximum');
    });

    expect(results).toBeDefined();
  });

  test('should handle batch search', async ({ page }) => {
    const results = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(4, 'cosine');

      // Insert vectors
      for (let i = 0; i < 20; i++) {
        store.insert(i, new Float32Array([i / 20, (20 - i) / 20, 0, 0]));
      }

      // Batch search with multiple queries
      const queries = [
        new Float32Array([1.0, 0.0, 0.0, 0.0]),
        new Float32Array([0.5, 0.5, 0.0, 0.0]),
        new Float32Array([0.0, 1.0, 0.0, 0.0]),
      ];

      return store.batch_search(queries, 3);
    });

    expect(results).toBeDefined();
    expect(results.length).toBe(3); // One result set per query
  });
});
