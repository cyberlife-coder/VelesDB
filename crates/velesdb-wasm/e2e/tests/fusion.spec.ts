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

      // Multi-query search: flatten queries into one Float32Array, pass all 5 args
      const queries = new Float32Array([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]); // query1 then query2
      const searchResults = store.multi_query_search(queries, 2, 5, 'rrf', undefined);
      return searchResults;
    });

    expect(results).toBeDefined();
    expect(Array.isArray(results)).toBe(true);
    expect(results.length).toBeGreaterThan(0);
    const ids = results.map((r: [number, number]) => r[0]);
    // query1 = [1,0,0,0] is exact match for id=1; query2 = [0,1,0,0] is exact match for id=4
    expect(ids).toContain(1);
    expect(ids).toContain(4);
    // id=1 is an exact match to the first query and must surface at the top
    expect(ids[0]).toBe(1);
  });

  test('should perform multi-query search with average fusion', async ({ page }) => {
    const results = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(4, 'cosine');

      store.insert(1, new Float32Array([1.0, 0.0, 0.0, 0.0]));
      store.insert(2, new Float32Array([0.0, 1.0, 0.0, 0.0]));

      // Flatten queries into one Float32Array; pass num_vectors=2, k=2, strategy, rrf_k
      const flat = new Float32Array([1.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.0, 0.0]);
      return store.multi_query_search(flat, 2, 2, 'average', undefined);
    });

    expect(results.length).toBe(2);
    // Under cosine + average fusion:
    // query1=[1,0,0,0]: id1 score=1.0, id2 score=0.0
    // query2=[0.5,0.5,0,0]: id1 ~0.707, id2 ~0.707
    // average: id1 ~0.85, id2 ~0.35 -> id1 ranks first
    expect(results[0][0]).toBe(1);
  });

  test('should handle batch search', async ({ page }) => {
    const results = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(4, 'cosine');

      // Insert vectors: vector i has value [i/20, (20-i)/20, 0, 0]
      for (let i = 0; i < 20; i++) {
        store.insert(i, new Float32Array([i / 20, (20 - i) / 20, 0, 0]));
      }

      // Flatten queries into one Float32Array and pass num_vectors and k explicitly
      const queries = new Float32Array([
        1.0, 0.0, 0.0, 0.0,   // query 0 -> closest to high-x vectors (large i, i.e. i=19)
        0.5, 0.5, 0.0, 0.0,   // query 1
        0.0, 1.0, 0.0, 0.0,   // query 2 -> closest to high-y vectors (small i, i.e. i=0)
      ]);

      return store.batch_search(queries, 3, 3);
    });

    expect(results).toBeDefined();
    expect(results.length).toBe(3); // One result set per query
    expect(results[0].length).toBe(3); // k=3 per query
    // query 0 = [1,0,0,0]; i=19 is [0.95, 0.05, 0, 0] -> closest under cosine
    expect(results[0][0][0]).toBe(19);
    // query 2 = [0,1,0,0]; i=0 is [0, 1, 0, 0] -> closest under cosine
    expect(results[2][0][0]).toBe(0);
  });
});
