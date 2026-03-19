/**
 * WASM Backend Integration Tests
 * 
 * Tests the WasmBackend class with mock WASM module
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { WasmBackend } from '../src/backends/wasm';
import { VelesDBError, NotFoundError, ConnectionError } from '../src/types';

// Mock WASM module with class-based VectorStore
class MockVectorStore {
  insert = vi.fn();
  insert_with_payload = vi.fn();
  insert_batch = vi.fn();
  search = vi.fn(() => [[BigInt(1), 0.95], [BigInt(2), 0.85]]);
  search_with_filter = vi.fn(() => [
    { id: BigInt(1), score: 0.95, payload: { title: 'filtered' } },
  ]);
  get = vi.fn((id: bigint) => ({
    id,
    vector: [1, 0, 0, 0],
    payload: { title: 'Stored' },
  }));
  text_search = vi.fn(() => [{ id: BigInt(1), score: 0.88, payload: { title: 'Text' } }]);
  hybrid_search = vi.fn(() => [{ id: BigInt(1), score: 0.91, payload: { title: 'Hybrid' } }]);
  query = vi.fn(() => [
    { id: 1, a: 1 },
  ]);
  multi_query_search = vi.fn(() => [[BigInt(1), 0.95]]);
  remove = vi.fn(() => true);
  clear = vi.fn();
  reserve = vi.fn();
  free = vi.fn();
  len = 0;
  is_empty = true;
  dimension: number;

  constructor(dimension: number, _metric: string) {
    this.dimension = dimension;
  }
}

const mockWasmModule = {
  default: vi.fn(() => Promise.resolve()),
  VectorStore: MockVectorStore,
};

// Mock the dynamic import - must match the import path in wasm.ts
vi.mock('@wiscale/velesdb-wasm', () => mockWasmModule);

describe('WasmBackend', () => {
  let backend: WasmBackend;

  beforeEach(() => {
    vi.clearAllMocks();
    backend = new WasmBackend();
  });

  describe('initialization', () => {
    it('should initialize successfully', async () => {
      await backend.init();
      expect(backend.isInitialized()).toBe(true);
    });

    it('should be idempotent', async () => {
      await backend.init();
      await backend.init(); // Should not throw
      expect(backend.isInitialized()).toBe(true);
    });
  });

  describe('collection operations', () => {
    beforeEach(async () => {
      await backend.init();
    });

    it('should create a collection', async () => {
      await backend.createCollection('test', { dimension: 128 });
      const col = await backend.getCollection('test');
      expect(col).not.toBeNull();
      expect(col?.name).toBe('test');
      expect(col?.dimension).toBe(128);
    });

    it('should throw on duplicate collection', async () => {
      await backend.createCollection('test', { dimension: 128 });
      await expect(backend.createCollection('test', { dimension: 128 }))
        .rejects.toThrow(VelesDBError);
    });

    it('should delete a collection', async () => {
      await backend.createCollection('test', { dimension: 128 });
      await backend.deleteCollection('test');
      const col = await backend.getCollection('test');
      expect(col).toBeNull();
    });

    it('should throw on deleting non-existent collection', async () => {
      await expect(backend.deleteCollection('nonexistent'))
        .rejects.toThrow(NotFoundError);
    });

    it('should list collections', async () => {
      await backend.createCollection('col1', { dimension: 128 });
      await backend.createCollection('col2', { dimension: 256, metric: 'euclidean' });
      
      const list = await backend.listCollections();
      expect(list.length).toBe(2);
      expect(list.map(c => c.name)).toContain('col1');
      expect(list.map(c => c.name)).toContain('col2');
    });
  });

  describe('vector operations', () => {
    beforeEach(async () => {
      await backend.init();
      await backend.createCollection('vectors', { dimension: 4 });
    });

    it('should insert a vector', async () => {
      await backend.insert('vectors', {
        id: '1',
        vector: [1.0, 0.0, 0.0, 0.0],
        payload: { title: 'Test' },
      });
      // No error means success
    });

    it('should get a vector by id', async () => {
      await backend.insert('vectors', {
        id: 'abc',
        vector: [1.0, 0.0, 0.0, 0.0],
        payload: { title: 'Stored' },
      });
      const doc = await backend.get('vectors', 'abc');
      expect(doc).not.toBeNull();
      expect(doc?.payload).toEqual({ title: 'Stored' });
    });

    it('should throw on dimension mismatch', async () => {
      await expect(backend.insert('vectors', {
        id: '1',
        vector: [1.0, 0.0], // Wrong dimension
      })).rejects.toThrow('dimension mismatch');
    });

    it('should throw on non-existent collection', async () => {
      await expect(backend.insert('nonexistent', {
        id: '1',
        vector: [1.0, 0.0, 0.0, 0.0],
      })).rejects.toThrow(NotFoundError);
    });

    it('should insert batch', async () => {
      await backend.insertBatch('vectors', [
        { id: '1', vector: [1.0, 0.0, 0.0, 0.0] },
        { id: '2', vector: [0.0, 1.0, 0.0, 0.0] },
      ]);
      // No error means success
    });

    it('should forward payload docs in insertBatch via insert_with_payload', async () => {
      await backend.insertBatch('vectors', [
        { id: '1', vector: [1.0, 0.0, 0.0, 0.0], payload: { category: 'A' } },
        { id: '2', vector: [0.0, 1.0, 0.0, 0.0] },
      ]);

      const collections = (backend as any).collections;
      const store = collections.get('vectors').store as MockVectorStore;

      expect(store.insert_with_payload).toHaveBeenCalledTimes(1);
      expect(store.insert_with_payload).toHaveBeenCalledWith(
        BigInt(1),
        expect.any(Float32Array),
        { category: 'A' },
      );
      expect(store.insert_batch).toHaveBeenCalledTimes(1);
      expect(store.insert_batch).toHaveBeenCalledWith([
        [BigInt(2), [0.0, 1.0, 0.0, 0.0]],
      ]);
    });

    it('should search vectors', async () => {
      const results = await backend.search('vectors', [1.0, 0.0, 0.0, 0.0], { k: 2 });
      expect(results.length).toBe(2);
      expect(results[0].score).toBe(0.95);
    });

    it('should delete a vector', async () => {
      const deleted = await backend.delete('vectors', '1');
      expect(deleted).toBe(true);
    });

    it('should not use partial numeric parsing for mixed string IDs', async () => {
      await backend.insert('vectors', {
        id: '123abc',
        vector: [1.0, 0.0, 0.0, 0.0],
      });

      await backend.delete('vectors', '123abc');

      const collections = (backend as any).collections;
      const store = collections.get('vectors').store as MockVectorStore;
      const lastRemoveArg = store.remove.mock.calls.at(-1)?.[0];
      expect(lastRemoveArg).not.toBe(BigInt(123));
    });
  });

  describe('multiQuerySearch', () => {
    beforeEach(async () => {
      await backend.init();
      await backend.createCollection('vectors', { dimension: 4, metric: 'cosine' });
    });

    it('should execute multi-query search', async () => {
      const results = await backend.multiQuerySearch('vectors', [[0.1, 0.2, 0.3, 0.4]]);
      expect(results.length).toBe(1);
      expect(results[0].score).toBe(0.95);
    });

    it('should reject weighted fusion strategy', async () => {
      await expect(
        backend.multiQuerySearch('vectors', [[0.1, 0.2, 0.3, 0.4]], {
          fusion: 'weighted',
          fusionParams: { avgWeight: 0.6, maxWeight: 0.3, hitWeight: 0.1 },
        }),
      ).rejects.toThrow("Fusion strategy 'weighted' is not supported in WASM backend.");
    });
  });

  describe('wasm feature parity', () => {
    beforeEach(async () => {
      await backend.init();
      await backend.createCollection('vectors', { dimension: 4, metric: 'cosine' });
    });

    it('supports text search', async () => {
      const results = await backend.textSearch('vectors', 'query', { k: 2 });
      expect(results.length).toBe(1);
      expect(results[0].score).toBe(0.88);
    });

    it('supports hybrid search', async () => {
      const results = await backend.hybridSearch('vectors', [0.1, 0.2, 0.3, 0.4], 'query');
      expect(results.length).toBe(1);
      expect(results[0].score).toBe(0.91);
    });

    it('supports query mapping', async () => {
      const response = await backend.query(
        'vectors',
        'SELECT * FROM vectors WHERE vector NEAR $q LIMIT 1',
        { q: [0.1, 0.2, 0.3, 0.4] }
      );
      expect('results' in response).toBe(true);
      if ('results' in response) {
        expect(response.results.length).toBe(1);
        expect(response.stats.strategy).toBe('wasm-query');
      }
    });
  });

  describe('Knowledge Graph (EPIC-016 US-041)', () => {
    beforeEach(async () => {
      await backend.init();
      await backend.createCollection('social', { dimension: 4, metric: 'cosine' });
    });

    it('should throw NOT_SUPPORTED error for addEdge', async () => {
      const edge = { id: 1, source: 100, target: 200, label: 'FOLLOWS' };
      await expect(backend.addEdge('social', edge))
        .rejects.toThrow(VelesDBError);
    });

    it('should throw NOT_SUPPORTED error for getEdges', async () => {
      await expect(backend.getEdges('social'))
        .rejects.toThrow(VelesDBError);
    });

    it('should include helpful error message for graph operations', async () => {
      await expect(backend.getEdges('social'))
        .rejects.toThrow('Knowledge Graph operations: not supported in WASM backend. Use REST backend.');
    });
  });

  describe('error handling', () => {
    it('should throw when not initialized', async () => {
      await expect(backend.createCollection('test', { dimension: 128 }))
        .rejects.toThrow(ConnectionError);
    });
  });

  describe('cleanup', () => {
    it('should close properly', async () => {
      await backend.init();
      await backend.createCollection('test', { dimension: 128 });
      await backend.close();
      expect(backend.isInitialized()).toBe(false);
    });
  });
});
