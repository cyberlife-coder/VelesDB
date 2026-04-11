/**
 * REST Backend Integration Tests
 * 
 * Tests the RestBackend class with mocked fetch
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { RestBackend } from '../src/backends/rest';
import { VelesDBError, ConnectionError, ValidationError } from '../src/types';
import { CollectionNotFoundError } from '../src/errors';

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
      // Server emits VELES-002 with the verbatim message; the transport layer
      // recognises CollectionNotFoundError as the "absent" signal and returns null.
      mockFetch.mockResolvedValueOnce({
        ok: false,
        json: () => Promise.resolve({
          code: 'VELES-002',
          message: "[VELES-002] Collection 'nonexistent' not found",
        }),
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
        id: 1,
        vector: [1.0, 0.0, 0.0],
        payload: { title: 'Test' },
      });

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/test/points',
        expect.objectContaining({
          method: 'POST',
          body: JSON.stringify({
            points: [{
              id: 1,
              vector: [1.0, 0.0, 0.0],
              payload: { title: 'Test' },
            }],
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
        { id: 1, vector: [1.0, 0.0] },
        { id: 2, vector: [0.0, 1.0] },
      ]);

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/test/points',
        expect.objectContaining({ method: 'POST' })
      );
    });

    it('should search vectors', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          results: [
            { id: '1', score: 0.95 },
            { id: '2', score: 0.85 },
          ],
        }),
      });

      const results = await backend.search('test', [1.0, 0.0], { k: 5 });
      expect(results.length).toBe(2);
      expect(results[0].score).toBe(0.95);
      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/test/search',
        expect.objectContaining({
          body: JSON.stringify({
            vector: [1.0, 0.0],
            top_k: 5,
            filter: undefined,
            include_vectors: false,
          }),
        })
      );
    });

    it('should delete a vector', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ deleted: true }),
      });

      const deleted = await backend.delete('test', 1);
      expect(deleted).toBe(true);
    });

    it('should get a vector', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ id: 1, vector: [1.0, 0.0], payload: { title: 'Test' } }),
      });

      const doc = await backend.get('test', 1);
      expect(doc?.id).toBe(1);
      expect(doc?.payload).toEqual({ title: 'Test' });
    });

    it('should reject non-numeric IDs for REST operations', async () => {
      await expect(
        backend.insert('test', {
          id: 'doc-1',
          vector: [1.0, 0.0, 0.0],
        })
      ).rejects.toThrow(ValidationError);

      await expect(backend.get('test', 'doc-1')).rejects.toThrow(ValidationError);
      await expect(backend.delete('test', 'doc-1')).rejects.toThrow(ValidationError);
    });

    it('should reject IDs above JS safe integer range for REST operations', async () => {
      const tooLarge = Number.MAX_SAFE_INTEGER + 1;

      await expect(
        backend.insert('test', {
          id: tooLarge,
          vector: [1.0, 0.0, 0.0],
        })
      ).rejects.toThrow(ValidationError);

      await expect(backend.get('test', tooLarge)).rejects.toThrow(ValidationError);
      await expect(backend.delete('test', tooLarge)).rejects.toThrow(ValidationError);
    });
  });

  describe('multiQuerySearch', () => {
    beforeEach(async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ status: 'ok' }),
      });
      await backend.init();
      vi.clearAllMocks();
    });

    it('should send POST to /search/multi with correct body', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ results: [{ id: '1', score: 0.95 }] }),
      });

      const vectors = [[0.1, 0.2], [0.3, 0.4]];
      const options = { k: 10, fusion: 'rrf' as const, fusionParams: { k: 60 } };
      
      await backend.multiQuerySearch('test', vectors, options);

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/test/search/multi',
        expect.objectContaining({
          method: 'POST',
          body: JSON.stringify({
            vectors: vectors,
            top_k: 10,
            strategy: 'rrf',
            rrf_k: 60,
            avg_weight: undefined,
            max_weight: undefined,
            hit_weight: undefined,
            filter: undefined,
          }),
        })
      );
    });

    it('should forward weighted fusion params', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ results: [] }),
      });

      await backend.multiQuerySearch('test', [[0.1, 0.2]], {
        fusion: 'weighted',
        fusionParams: {
          avgWeight: 0.6,
          maxWeight: 0.3,
          hitWeight: 0.1,
        },
      });

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/test/search/multi',
        expect.objectContaining({
          body: JSON.stringify({
            vectors: [[0.1, 0.2]],
            top_k: 10,
            strategy: 'weighted',
            rrf_k: 60,
            avg_weight: 0.6,
            max_weight: 0.3,
            hit_weight: 0.1,
            filter: undefined,
          }),
        })
      );
    });

    it('should return fused search results', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ 
          results: [
            { id: '1', score: 0.95 },
            { id: '2', score: 0.85 }
          ] 
        }),
      });

      const results = await backend.multiQuerySearch('test', [[0.1, 0.2]], { k: 5 });
      
      expect(results.length).toBe(2);
      expect(results[0].id).toBe('1');
      expect(results[0].score).toBe(0.95);
    });

    it('should handle collection not found (VELES-002 → CollectionNotFoundError)', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        status: 404,
        json: () => Promise.resolve({
          code: 'VELES-002',
          message: "[VELES-002] Collection 'nonexistent' not found",
        }),
      });

      await expect(backend.multiQuerySearch('nonexistent', [[0.1, 0.2]]))
        .rejects.toThrow(CollectionNotFoundError);
    });

    it('should use default fusion strategy when not specified', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ results: [] }),
      });

      await backend.multiQuerySearch('test', [[0.1, 0.2]]);

      expect(mockFetch).toHaveBeenCalledWith(
        expect.any(String),
        expect.objectContaining({
          body: expect.stringContaining('"strategy":"rrf"'),
        })
      );
    });
  });

  describe('Knowledge Graph (EPIC-016 US-041)', () => {
    beforeEach(async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ status: 'ok' }),
      });
      await backend.init();
      vi.clearAllMocks();
    });

    it('should send POST to /graph/edges for addEdge', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({}),
      });

      const edge = { id: 1, source: 100, target: 200, label: 'FOLLOWS' };
      await backend.addEdge('social', edge);

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/social/graph/edges',
        expect.objectContaining({ method: 'POST' })
      );
    });

    it('should send GET to /graph/edges for getEdges', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ edges: [], count: 0 }),
      });

      await backend.getEdges('social');

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/social/graph/edges',
        expect.objectContaining({ method: 'GET' })
      );
    });

    it('should filter by label in query params', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ edges: [{ id: 1, source: 100, target: 200, label: 'FOLLOWS' }], count: 1 }),
      });

      const edges = await backend.getEdges('social', { label: 'FOLLOWS' });

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/social/graph/edges?label=FOLLOWS',
        expect.objectContaining({ method: 'GET' })
      );
      expect(edges.length).toBe(1);
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

    it('should handle nested velesql error payload', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        status: 422,
        json: () =>
          Promise.resolve({
            error: {
              code: 'VELESQL_MISSING_COLLECTION',
              message: 'MATCH query via /query requires `collection` in request body',
              hint: 'Add collection',
            },
          }),
      });

      await expect(backend.query('docs', 'MATCH (d:Doc) RETURN d LIMIT 1', {})).rejects.toThrow(
        VelesDBError
      );
    });

    it('should handle timeout', async () => {
      const abortError = new Error('Aborted');
      abortError.name = 'AbortError';
      mockFetch.mockRejectedValueOnce(abortError);

      await expect(backend.createCollection('test', { dimension: 128 }))
        .rejects.toThrow(ConnectionError);
    });
  });

  describe('query forwarding', () => {
    beforeEach(async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ status: 'ok' }),
      });
      await backend.init();
      vi.clearAllMocks();
    });

    it('should forward collection in /query body for MATCH compatibility', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          results: [],
          timing_ms: 0.1,
          rows_returned: 0,
        }),
      });

      await backend.query('docs', 'MATCH (d:Doc) RETURN d LIMIT 1', {});

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/query',
        expect.objectContaining({
          method: 'POST',
          body: JSON.stringify({
            query: 'MATCH (d:Doc) RETURN d LIMIT 1',
            params: {},
            collection: 'docs',
            timeout_ms: undefined,
            stream: false,
          }),
        })
      );
    });

    it('should pass query options timeout/stream to /query', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          results: [],
          timing_ms: 0.2,
          rows_returned: 0,
        }),
      });

      await backend.query('docs', 'SELECT * FROM docs LIMIT 1', {}, { timeoutMs: 1500, stream: true });

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/query',
        expect.objectContaining({
          body: JSON.stringify({
            query: 'SELECT * FROM docs LIMIT 1',
            params: {},
            collection: 'docs',
            timeout_ms: 1500,
            stream: true,
          }),
        })
      );
    });

    it('should map aggregation response from /query', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () =>
          Promise.resolve({
            result: [{ category: 'tech', count: 2 }],
            timing_ms: 0.9,
          }),
      });

      const response = await backend.query(
        'docs',
        'SELECT category, COUNT(*) FROM docs GROUP BY category',
        {}
      );

      expect('result' in response).toBe(true);
      if ('result' in response) {
        expect(Array.isArray(response.result)).toBe(true);
        expect(response.stats.strategy).toBe('aggregation');
      }
    });

    it('should call /query/explain and return explain payload', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          query: 'SELECT * FROM docs LIMIT 10',
          query_type: 'SELECT',
          collection: 'docs',
          plan: [{ step: 1, operation: 'FullScan', description: 'Scan collection', estimated_rows: null }],
          estimated_cost: {
            uses_index: false,
            index_name: null,
            selectivity: 1,
            complexity: 'O(n)',
          },
          features: {
            has_vector_search: false,
            has_filter: false,
            has_order_by: false,
            has_group_by: false,
            has_aggregation: false,
            has_join: false,
            has_fusion: false,
            limit: 10,
            offset: null,
          },
        }),
      });

      const explain = await backend.queryExplain('SELECT * FROM docs LIMIT 10', {});

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/query/explain',
        expect.objectContaining({
          method: 'POST',
          body: JSON.stringify({
            query: 'SELECT * FROM docs LIMIT 10',
            params: {},
          }),
        })
      );
      expect(explain.queryType).toBe('SELECT');
      expect(explain.plan[0]?.operation).toBe('FullScan');
    });

    it('should call /collections/{name}/sanity and map response', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          collection: 'docs',
          dimension: 768,
          metric: 'cosine',
          point_count: 42,
          is_empty: false,
          checks: {
            has_vectors: true,
            search_ready: true,
            dimension_configured: true,
          },
          diagnostics: {
            search_requests_total: 3,
            dimension_mismatch_total: 0,
            empty_search_results_total: 1,
            filter_parse_errors_total: 0,
          },
          hints: ['hint-1'],
        }),
      });

      const sanity = await backend.collectionSanity('docs');

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/docs/sanity',
        expect.objectContaining({ method: 'GET' })
      );
      expect(sanity.pointCount).toBe(42);
      expect(sanity.checks.searchReady).toBe(true);
      expect(sanity.diagnostics.searchRequestsTotal).toBe(3);
    });
  });
});
