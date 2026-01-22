/**
 * VelesDB Client Tests (TDD)
 */

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { VelesDB } from '../src/client';
import { ValidationError } from '../src/types';

describe('VelesDB Client', () => {
  describe('constructor', () => {
    it('should create client with wasm backend', () => {
      const db = new VelesDB({ backend: 'wasm' });
      expect(db.backendType).toBe('wasm');
    });

    it('should create client with rest backend', () => {
      const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
      expect(db.backendType).toBe('rest');
    });

    it('should throw ValidationError for missing backend', () => {
      expect(() => new VelesDB({} as any)).toThrow(ValidationError);
    });

    it('should throw ValidationError for invalid backend type', () => {
      expect(() => new VelesDB({ backend: 'invalid' as any })).toThrow(ValidationError);
    });

    it('should throw ValidationError for rest backend without url', () => {
      expect(() => new VelesDB({ backend: 'rest' })).toThrow(ValidationError);
    });
  });

  describe('initialization', () => {
    it('should start uninitialized', () => {
      const db = new VelesDB({ backend: 'wasm' });
      expect(db.isInitialized()).toBe(false);
    });
  });

  describe('validation', () => {
    let db: VelesDB;

    beforeEach(() => {
      db = new VelesDB({ backend: 'wasm' });
    });

    it('should throw when calling methods before init', async () => {
      await expect(db.createCollection('test', { dimension: 128 }))
        .rejects.toThrow('Client not initialized');
    });

    it('should validate collection name', async () => {
      // Mock initialization
      (db as any).initialized = true;
      (db as any).backend = {
        createCollection: vi.fn(),
      };

      await expect(db.createCollection('', { dimension: 128 }))
        .rejects.toThrow(ValidationError);
    });

    it('should validate dimension', async () => {
      (db as any).initialized = true;
      (db as any).backend = {
        createCollection: vi.fn(),
      };

      await expect(db.createCollection('test', { dimension: 0 }))
        .rejects.toThrow(ValidationError);

      await expect(db.createCollection('test', { dimension: -1 }))
        .rejects.toThrow(ValidationError);
    });

    it('should validate document ID', async () => {
      (db as any).initialized = true;
      (db as any).backend = {
        insert: vi.fn(),
      };

      await expect(db.insert('test', { id: null as any, vector: [1, 2, 3] }))
        .rejects.toThrow(ValidationError);
    });

    it('should validate document vector', async () => {
      (db as any).initialized = true;
      (db as any).backend = {
        insert: vi.fn(),
      };

      await expect(db.insert('test', { id: '1', vector: null as any }))
        .rejects.toThrow(ValidationError);

      await expect(db.insert('test', { id: '1', vector: 'invalid' as any }))
        .rejects.toThrow(ValidationError);
    });

    it('should validate query vector', async () => {
      (db as any).initialized = true;
      (db as any).backend = {
        search: vi.fn(),
      };

      await expect(db.search('test', null as any))
        .rejects.toThrow(ValidationError);
    });

    it('should validate batch documents', async () => {
      (db as any).initialized = true;
      (db as any).backend = {
        insertBatch: vi.fn(),
      };

      await expect(db.insertBatch('test', 'invalid' as any))
        .rejects.toThrow(ValidationError);
    });
  });

  describe('operations', () => {
    let db: VelesDB;
    let mockBackend: any;

    beforeEach(() => {
      db = new VelesDB({ backend: 'wasm' });
      mockBackend = {
        init: vi.fn(),
        createCollection: vi.fn(),
        deleteCollection: vi.fn(),
        getCollection: vi.fn(),
        listCollections: vi.fn(),
        insert: vi.fn(),
        insertBatch: vi.fn(),
        search: vi.fn(),
        delete: vi.fn(),
        get: vi.fn(),
        close: vi.fn(),
      };
      (db as any).backend = mockBackend;
      (db as any).initialized = true;
    });

    it('should call backend createCollection', async () => {
      await db.createCollection('test', { dimension: 768, metric: 'cosine' });
      expect(mockBackend.createCollection).toHaveBeenCalledWith('test', {
        dimension: 768,
        metric: 'cosine',
      });
    });

    it('should call backend deleteCollection', async () => {
      await db.deleteCollection('test');
      expect(mockBackend.deleteCollection).toHaveBeenCalledWith('test');
    });

    it('should call backend getCollection', async () => {
      mockBackend.getCollection.mockResolvedValue({ name: 'test', dimension: 768 });
      const result = await db.getCollection('test');
      expect(result).toEqual({ name: 'test', dimension: 768 });
    });

    it('should call backend listCollections', async () => {
      mockBackend.listCollections.mockResolvedValue([{ name: 'test' }]);
      const result = await db.listCollections();
      expect(result).toEqual([{ name: 'test' }]);
    });

    it('should call backend insert', async () => {
      const doc = { id: '1', vector: [0.1, 0.2], payload: { title: 'test' } };
      await db.insert('test', doc);
      expect(mockBackend.insert).toHaveBeenCalledWith('test', doc);
    });

    it('should call backend insertBatch', async () => {
      const docs = [
        { id: '1', vector: [0.1, 0.2] },
        { id: '2', vector: [0.3, 0.4] },
      ];
      await db.insertBatch('test', docs);
      expect(mockBackend.insertBatch).toHaveBeenCalledWith('test', docs);
    });

    it('should call backend search', async () => {
      mockBackend.search.mockResolvedValue([{ id: '1', score: 0.95 }]);
      const result = await db.search('test', [0.1, 0.2], { k: 5 });
      expect(result).toEqual([{ id: '1', score: 0.95 }]);
      expect(mockBackend.search).toHaveBeenCalledWith('test', [0.1, 0.2], { k: 5 });
    });

    it('should call backend delete', async () => {
      mockBackend.delete.mockResolvedValue(true);
      const result = await db.delete('test', '1');
      expect(result).toBe(true);
    });

    it('should call backend get', async () => {
      mockBackend.get.mockResolvedValue({ id: '1', vector: [0.1] });
      const result = await db.get('test', '1');
      expect(result).toEqual({ id: '1', vector: [0.1] });
    });

    it('should call backend close', async () => {
      await db.close();
      expect(mockBackend.close).toHaveBeenCalled();
      expect(db.isInitialized()).toBe(false);
    });
  });

  describe('multiQuerySearch', () => {
    let db: VelesDB;
    let mockBackend: any;

    beforeEach(() => {
      db = new VelesDB({ backend: 'wasm' });
      mockBackend = {
        init: vi.fn(),
        multiQuerySearch: vi.fn(),
      };
      (db as any).backend = mockBackend;
      (db as any).initialized = true;
    });

    it('should validate that vectors is a non-empty array', async () => {
      await expect(db.multiQuerySearch('test', []))
        .rejects.toThrow(ValidationError);

      await expect(db.multiQuerySearch('test', null as any))
        .rejects.toThrow(ValidationError);
    });

    it('should validate each vector in the array', async () => {
      await expect(db.multiQuerySearch('test', ['invalid' as any]))
        .rejects.toThrow(ValidationError);

      await expect(db.multiQuerySearch('test', [null as any]))
        .rejects.toThrow(ValidationError);
    });

    it('should call backend multiQuerySearch with correct parameters', async () => {
      const vectors = [[0.1, 0.2], [0.3, 0.4]];
      const options = { k: 5, fusion: 'rrf' as const };
      mockBackend.multiQuerySearch.mockResolvedValue([{ id: '1', score: 0.95 }]);

      const result = await db.multiQuerySearch('test', vectors, options);

      expect(mockBackend.multiQuerySearch).toHaveBeenCalledWith('test', vectors, options);
      expect(result).toEqual([{ id: '1', score: 0.95 }]);
    });

    it('should accept Float32Array vectors', async () => {
      const vectors = [new Float32Array([0.1, 0.2]), new Float32Array([0.3, 0.4])];
      mockBackend.multiQuerySearch.mockResolvedValue([]);

      await db.multiQuerySearch('test', vectors);

      expect(mockBackend.multiQuerySearch).toHaveBeenCalled();
    });

    it('should work with default options when none provided', async () => {
      const vectors = [[0.1, 0.2]];
      mockBackend.multiQuerySearch.mockResolvedValue([]);

      await db.multiQuerySearch('test', vectors);

      expect(mockBackend.multiQuerySearch).toHaveBeenCalledWith('test', vectors, undefined);
    });

    it('should support all fusion strategies', async () => {
      const vectors = [[0.1, 0.2]];
      mockBackend.multiQuerySearch.mockResolvedValue([]);

      await db.multiQuerySearch('test', vectors, { fusion: 'rrf' });
      await db.multiQuerySearch('test', vectors, { fusion: 'average' });
      await db.multiQuerySearch('test', vectors, { fusion: 'maximum' });
      await db.multiQuerySearch('test', vectors, { fusion: 'weighted' });

      expect(mockBackend.multiQuerySearch).toHaveBeenCalledTimes(4);
    });

    it('should pass fusion params correctly', async () => {
      const vectors = [[0.1, 0.2]];
      const options = {
        k: 10,
        fusion: 'weighted' as const,
        fusionParams: { avgWeight: 0.6, maxWeight: 0.3, hitWeight: 0.1 }
      };
      mockBackend.multiQuerySearch.mockResolvedValue([]);

      await db.multiQuerySearch('test', vectors, options);

      expect(mockBackend.multiQuerySearch).toHaveBeenCalledWith('test', vectors, options);
    });
  });

  describe('Knowledge Graph (EPIC-016 US-041)', () => {
    let db: VelesDB;
    let mockBackend: any;

    beforeEach(() => {
      db = new VelesDB({ backend: 'wasm' });
      mockBackend = {
        init: vi.fn(),
        addEdge: vi.fn(),
        getEdges: vi.fn(),
      };
      (db as any).backend = mockBackend;
      (db as any).initialized = true;
    });

    describe('addEdge', () => {
      it('should validate edge label is required', async () => {
        await expect(db.addEdge('test', { id: 1, source: 100, target: 200, label: '' }))
          .rejects.toThrow(ValidationError);
      });

      it('should validate source and target are numbers', async () => {
        await expect(db.addEdge('test', { id: 1, source: 'a' as any, target: 200, label: 'KNOWS' }))
          .rejects.toThrow(ValidationError);
      });

      it('should call backend addEdge with correct parameters', async () => {
        const edge = { id: 1, source: 100, target: 200, label: 'FOLLOWS', properties: { since: '2024' } };
        mockBackend.addEdge.mockResolvedValue(undefined);

        await db.addEdge('social', edge);

        expect(mockBackend.addEdge).toHaveBeenCalledWith('social', edge);
      });
    });

    describe('getEdges', () => {
      it('should call backend getEdges without options', async () => {
        mockBackend.getEdges.mockResolvedValue([]);

        await db.getEdges('social');

        expect(mockBackend.getEdges).toHaveBeenCalledWith('social', undefined);
      });

      it('should call backend getEdges with label filter', async () => {
        mockBackend.getEdges.mockResolvedValue([{ id: 1, source: 100, target: 200, label: 'FOLLOWS' }]);

        const edges = await db.getEdges('social', { label: 'FOLLOWS' });

        expect(mockBackend.getEdges).toHaveBeenCalledWith('social', { label: 'FOLLOWS' });
        expect(edges.length).toBe(1);
      });
    });
  });
});
