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
  insert_batch = vi.fn();
  search = vi.fn(() => [[BigInt(1), 0.95], [BigInt(2), 0.85]]);
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

// Mock the dynamic import
vi.mock('velesdb-wasm', () => mockWasmModule);

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

    it('should search vectors', async () => {
      const results = await backend.search('vectors', [1.0, 0.0, 0.0, 0.0], { k: 2 });
      expect(results.length).toBe(2);
      expect(results[0].score).toBe(0.95);
    });

    it('should delete a vector', async () => {
      const deleted = await backend.delete('vectors', '1');
      expect(deleted).toBe(true);
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
