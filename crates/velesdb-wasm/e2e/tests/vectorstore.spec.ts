import { test, expect } from '@playwright/test';

test.describe('VelesDB WASM VectorStore', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    // Wait for WASM to load
    await page.waitForFunction(() => window['VelesDB']?.ready === true, { timeout: 10000 });
  });

  test('should load WASM module successfully', async ({ page }) => {
    const ready = await page.evaluate(() => window['VelesDB'].ready);
    expect(ready).toBe(true);
  });

  test('should create VectorStore with cosine metric', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(128, 'cosine');
      return {
        dimension: store.dimension(),
        len: store.len(),
        isEmpty: store.is_empty(),
      };
    });

    expect(result.dimension).toBe(128);
    expect(result.len).toBe(0);
    expect(result.isEmpty).toBe(true);
  });

  test('should create VectorStore with euclidean metric', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(768, 'euclidean');
      return { dimension: store.dimension() };
    });

    expect(result.dimension).toBe(768);
  });

  test('should insert and search vectors', async ({ page }) => {
    const results = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(4, 'cosine');

      // Insert vectors
      store.insert(1, new Float32Array([1.0, 0.0, 0.0, 0.0]));
      store.insert(2, new Float32Array([0.9, 0.1, 0.0, 0.0]));
      store.insert(3, new Float32Array([0.5, 0.5, 0.0, 0.0]));
      store.insert(4, new Float32Array([0.0, 1.0, 0.0, 0.0]));

      // Search
      const query = new Float32Array([1.0, 0.0, 0.0, 0.0]);
      const searchResults = store.search(query, 3);

      return {
        storeLen: store.len(),
        results: searchResults,
      };
    });

    expect(results.storeLen).toBe(4);
    expect(results.results).toHaveLength(3);
    // First result should be exact match (id=1)
    expect(results.results[0].id).toBe(1);
  });

  test('should handle batch insert', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(4, 'cosine');

      // Batch insert 100 vectors
      for (let i = 0; i < 100; i++) {
        store.insert(i, new Float32Array([i / 100, 0.0, 0.0, 0.0]));
      }

      return { len: store.len() };
    });

    expect(result.len).toBe(100);
  });

  test('should reject invalid metric', async ({ page }) => {
    const error = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      try {
        VectorStore.new(128, 'invalid_metric');
        return null;
      } catch (e) {
        return e.message;
      }
    });

    expect(error).toBeTruthy();
    expect(error).toContain('metric');
  });

  test('should reject dimension mismatch on insert', async ({ page }) => {
    const error = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(4, 'cosine');
      try {
        store.insert(1, new Float32Array([1.0, 0.0, 0.0])); // Wrong dimension
        return null;
      } catch (e) {
        return e.message;
      }
    });

    expect(error).toBeTruthy();
  });

  test('should remove vectors', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(4, 'cosine');

      store.insert(1, new Float32Array([1.0, 0.0, 0.0, 0.0]));
      store.insert(2, new Float32Array([0.0, 1.0, 0.0, 0.0]));

      const lenBefore = store.len();
      store.remove(1);
      const lenAfter = store.len();

      return { lenBefore, lenAfter };
    });

    expect(result.lenBefore).toBe(2);
    expect(result.lenAfter).toBe(1);
  });

  test('should clear all vectors', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(4, 'cosine');

      for (let i = 0; i < 10; i++) {
        store.insert(i, new Float32Array([i, 0, 0, 0]));
      }

      const lenBefore = store.len();
      store.clear();
      const lenAfter = store.len();

      return { lenBefore, lenAfter };
    });

    expect(result.lenBefore).toBe(10);
    expect(result.lenAfter).toBe(0);
  });

  test('should support dot product metric', async ({ page }) => {
    const results = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(4, 'dot');

      store.insert(1, new Float32Array([1.0, 0.0, 0.0, 0.0]));
      store.insert(2, new Float32Array([0.5, 0.5, 0.0, 0.0]));

      return store.search(new Float32Array([1.0, 0.0, 0.0, 0.0]), 2);
    });

    expect(results).toHaveLength(2);
  });

  test('should handle large vectors (1536 dim - OpenAI embeddings)', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { VectorStore } = window['VelesDB'];
      const store = VectorStore.new(1536, 'cosine');

      // Create random 1536-dim vectors
      const vec1 = new Float32Array(1536).fill(0.1);
      const vec2 = new Float32Array(1536).fill(0.2);

      store.insert(1, vec1);
      store.insert(2, vec2);

      return {
        dimension: store.dimension(),
        len: store.len(),
      };
    });

    expect(result.dimension).toBe(1536);
    expect(result.len).toBe(2);
  });
});
