/**
 * REST Backend Integration Tests
 * 
 * Tests the RestBackend class with mocked fetch
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { RestBackend } from '../src/backends/rest';
import { VelesDBError, NotFoundError, ConnectionError } from '../src/types';

// Mock global fetch
const mockFetch = vi.fn();
global.fetch = mockFetch;

describe('RestBackend', () => {
  let backend: RestBackend;

  beforeEach(() => {
    vi.clearAllMocks();
    backend = new RestBackend('http://localhost:8080', 'test-api-key');
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('initialization', () => {
    it('should initialize with health check', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ status: 'ok' }),
      });

      await backend.init();
      expect(backend.isInitialized()).toBe(true);
      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/health',
        expect.objectContaining({
          method: 'GET',
          headers: expect.objectContaining({
            'Authorization': 'Bearer test-api-key',
          }),
        })
      );
    });

    it('should throw on connection failure', async () => {
      mockFetch.mockRejectedValueOnce(new Error('Network error'));
      await expect(backend.init()).rejects.toThrow(ConnectionError);
    });

    it('should throw on unhealthy server', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        json: () => Promise.resolve({ code: 'SERVER_ERROR', message: 'Internal error' }),
      });
      await expect(backend.init()).rejects.toThrow(ConnectionError);
    });
  });

  describe('collection operations', () => {
    beforeEach(async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ status: 'ok' }),
      });
      await backend.init();
      vi.clearAllMocks();
    });

    it('should create a collection', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ name: 'test', dimension: 128 }),
      });

      await backend.createCollection('test', { dimension: 128, metric: 'cosine' });

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections',
        expect.objectContaining({
          method: 'POST',
          body: JSON.stringify({
            name: 'test',
            dimension: 128,
            metric: 'cosine',
            storage_mode: 'full',
            collection_type: 'vector',
          }),
        })
      );
    });

    it('should get a collection', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ name: 'test', dimension: 128, metric: 'cosine', count: 100 }),
      });

      const col = await backend.getCollection('test');
      expect(col?.name).toBe('test');
      expect(col?.dimension).toBe(128);
    });

    it('should return null for non-existent collection', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        json: () => Promise.resolve({ code: 'NOT_FOUND', message: 'Not found' }),
      });

      const col = await backend.getCollection('nonexistent');
      expect(col).toBeNull();
    });

    it('should delete a collection', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({}),
      });

      await backend.deleteCollection('test');
      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/test',
        expect.objectContaining({ method: 'DELETE' })
      );
    });

    it('should list collections', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve([
          { name: 'col1', dimension: 128 },
          { name: 'col2', dimension: 256 },
        ]),
      });

      const list = await backend.listCollections();
      expect(list.length).toBe(2);
    });
  });

  describe('vector operations', () => {
    beforeEach(async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ status: 'ok' }),
      });
      await backend.init();
      vi.clearAllMocks();
    });

    it('should insert a vector', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({}),
      });

      await backend.insert('test', {
        id: '1',
        vector: [1.0, 0.0, 0.0],
        payload: { title: 'Test' },
      });

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/test/vectors',
        expect.objectContaining({
          method: 'POST',
          body: JSON.stringify({
            id: '1',
            vector: [1.0, 0.0, 0.0],
            payload: { title: 'Test' },
          }),
        })
      );
    });

    it('should insert batch', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({}),
      });

      await backend.insertBatch('test', [
        { id: '1', vector: [1.0, 0.0] },
        { id: '2', vector: [0.0, 1.0] },
      ]);

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/test/vectors/batch',
        expect.objectContaining({ method: 'POST' })
      );
    });

    it('should search vectors', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve([
          { id: '1', score: 0.95 },
          { id: '2', score: 0.85 },
        ]),
      });

      const results = await backend.search('test', [1.0, 0.0], { k: 5 });
      expect(results.length).toBe(2);
      expect(results[0].score).toBe(0.95);
    });

    it('should delete a vector', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ deleted: true }),
      });

      const deleted = await backend.delete('test', '1');
      expect(deleted).toBe(true);
    });

    it('should get a vector', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ id: '1', vector: [1.0, 0.0], payload: { title: 'Test' } }),
      });

      const doc = await backend.get('test', '1');
      expect(doc?.id).toBe('1');
      expect(doc?.payload).toEqual({ title: 'Test' });
    });
  });

  describe('error handling', () => {
    beforeEach(async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ status: 'ok' }),
      });
      await backend.init();
      vi.clearAllMocks();
    });

    it('should handle API errors', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        json: () => Promise.resolve({ code: 'VALIDATION_ERROR', message: 'Invalid request' }),
      });

      await expect(backend.createCollection('test', { dimension: 128 }))
        .rejects.toThrow(VelesDBError);
    });

    it('should handle timeout', async () => {
      const abortError = new Error('Aborted');
      abortError.name = 'AbortError';
      mockFetch.mockRejectedValueOnce(abortError);

      await expect(backend.createCollection('test', { dimension: 128 }))
        .rejects.toThrow(ConnectionError);
    });
  });
});
