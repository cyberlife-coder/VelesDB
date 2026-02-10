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

    it('should share health check across concurrent init() calls (BEG-07)', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ status: 'ok' }),
      });

      // Call init() concurrently — only ONE health check should fire
      await Promise.all([backend.init(), backend.init(), backend.init()]);

      expect(backend.isInitialized()).toBe(true);
      expect(mockFetch).toHaveBeenCalledTimes(1);
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

    it('should list collections and unwrap { collections: [...] } with field mapping', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          collections: [
            { name: 'col1', dimension: 128, metric: 'cosine', point_count: 50, storage_mode: 'full' },
            { name: 'col2', dimension: 256, metric: 'euclidean', point_count: 100, storage_mode: 'sq8' },
          ],
        }),
      });

      const list = await backend.listCollections();
      expect(list.length).toBe(2);
      expect(list[0].name).toBe('col1');
      expect(list[0].count).toBe(50);
      expect(list[0].metric).toBe('cosine');
      expect(list[0].storageMode).toBe('full');
      expect(list[1].name).toBe('col2');
      expect(list[1].count).toBe(100);
      expect(list[1].metric).toBe('euclidean');
      expect(list[1].storageMode).toBe('sq8');
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
        'http://localhost:8080/collections/test/points',
        expect.objectContaining({
          method: 'POST',
          body: JSON.stringify({
            points: [{
              id: '1',
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
        { id: '1', vector: [1.0, 0.0] },
        { id: '2', vector: [0.0, 1.0] },
      ]);

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/test/points',
        expect.objectContaining({ method: 'POST' })
      );
    });

    it('should search vectors and unwrap { results: [...] } envelope', async () => {
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
    });

    it('should delete a vector', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ deleted: true }),
      });

      const deleted = await backend.delete('test', '1');
      expect(deleted).toBe(true);
    });

    it('should default delete to true on HTTP success without body', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({}),
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

    it('should handle collection not found', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        status: 404,
        json: () => Promise.resolve({ code: 'NOT_FOUND', message: 'Collection not found' }),
      });

      await expect(backend.multiQuerySearch('nonexistent', [[0.1, 0.2]]))
        .rejects.toThrow(NotFoundError);
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

  describe('query() smart routing + aggregation', () => {
    beforeEach(async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ status: 'ok' }),
      });
      await backend.init();
      vi.clearAllMocks();
    });

    it('should route MATCH queries to matchQuery() endpoint', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          results: [{ bindings: { a: 1 }, score: 0.9, depth: 1, projected: {} }],
          took_ms: 5,
          count: 1,
        }),
      });

      const result = await backend.query(
        'docs',
        'MATCH (a:Person)-[:KNOWS]->(b) RETURN a',
      );

      // Should hit /collections/docs/match, NOT /query
      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/docs/match',
        expect.objectContaining({ method: 'POST' })
      );

      expect(result.results.length).toBe(1);
      expect(result.stats.strategy).toBe('match');
    });

    it('should handle aggregation responses (singular result)', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          result: { avg_price: 42.5, count: 100 },
          timing_ms: 3,
        }),
      });

      const result = await backend.query(
        'products',
        'SELECT AVG(price) as avg_price, COUNT(*) as count FROM products',
      );

      expect(result.results.length).toBe(1);
      expect(result.results[0].bindings).toEqual({ avg_price: 42.5, count: 100 });
      expect(result.stats.strategy).toBe('aggregation');
      expect(result.stats.executionTimeMs).toBe(3);
    });

    it('should handle scalar aggregation result', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          result: 42,
          timing_ms: 1,
        }),
      });

      const result = await backend.query(
        'products',
        'SELECT COUNT(*) FROM products',
      );

      expect(result.results.length).toBe(1);
      expect(result.results[0].bindings).toEqual({ value: 42 });
    });

    it('should handle standard SELECT responses with proper mapping', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          results: [
            { id: 1, score: 0.95, payload: { title: 'Test' } },
            { id: 2, score: 0.85, payload: { title: 'Other' } },
          ],
          timing_ms: 10,
          rows_returned: 2,
        }),
      });

      const result = await backend.query(
        'docs',
        'SELECT * FROM docs WHERE vector NEAR $v LIMIT 10',
      );

      expect(result.results.length).toBe(2);
      expect(result.results[0].nodeId).toBe(1);
      expect(result.results[0].vectorScore).toBe(0.95);
      expect(result.results[0].bindings).toEqual({ title: 'Test' });
      expect(result.stats.strategy).toBe('select');
      expect(result.stats.executionTimeMs).toBe(10);
    });
  });

  describe('matchQuery()', () => {
    beforeEach(async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ status: 'ok' }),
      });
      await backend.init();
      vi.clearAllMocks();
    });

    it('should call POST /collections/{name}/match', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          results: [
            { bindings: { a: 123, b: 456 }, score: 0.95, depth: 1, projected: { 'a.name': 'Alice' } },
          ],
          took_ms: 15,
          count: 1,
        }),
      });

      const result = await backend.matchQuery(
        'docs',
        'MATCH (a:Person)-[:KNOWS]->(b) RETURN a.name',
        { v: [0.1, 0.2, 0.3] },
        { vector: [0.1, 0.2, 0.3], threshold: 0.8 }
      );

      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/collections/docs/match',
        expect.objectContaining({ method: 'POST' })
      );

      expect(result.results.length).toBe(1);
      expect(result.results[0].bindings).toEqual({ a: 123, b: 456 });
      expect(result.results[0].score).toBe(0.95);
      expect(result.results[0].depth).toBe(1);
      expect(result.results[0].projected).toEqual({ 'a.name': 'Alice' });
      expect(result.tookMs).toBe(15);
      expect(result.count).toBe(1);
    });

    it('should handle collection not found', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        status: 404,
        json: () => Promise.resolve({ code: 'NOT_FOUND', message: 'Collection not found' }),
      });

      await expect(backend.matchQuery('nonexistent', 'MATCH (a) RETURN a'))
        .rejects.toThrow(NotFoundError);
    });

    it('should send vector and threshold when provided', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ results: [], took_ms: 0, count: 0 }),
      });

      await backend.matchQuery('docs', 'MATCH (a) RETURN a', {}, {
        vector: [0.1, 0.2],
        threshold: 0.85,
      });

      const callBody = JSON.parse(mockFetch.mock.calls[0][1].body);
      expect(callBody.vector).toEqual([0.1, 0.2]);
      expect(callBody.threshold).toBe(0.85);
    });
  });

  describe('explain', () => {
    beforeEach(async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ status: 'ok' }),
      });
      await backend.init();
      vi.clearAllMocks();
    });

    it('should send explain request and map snake_case response to camelCase', async () => {
      const serverResponse = {
        query: 'SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8 LIMIT 10',
        query_type: 'SELECT',
        collection: 'docs',
        plan: [
          { step: 1, operation: 'VectorSearch', description: 'ANN search using HNSW', estimated_rows: 10 },
          { step: 2, operation: 'Limit', description: 'Apply LIMIT 10 OFFSET 0', estimated_rows: 10 },
        ],
        estimated_cost: {
          uses_index: true,
          index_name: 'HNSW',
          selectivity: 0.01,
          complexity: 'O(log n)',
        },
        features: {
          has_vector_search: true,
          has_filter: false,
          has_order_by: false,
          has_group_by: false,
          has_aggregation: false,
          has_join: false,
          has_fusion: false,
          limit: 10,
          offset: null,
        },
      };

      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve(serverResponse),
      });

      const result = await backend.explain(
        'SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8 LIMIT 10'
      );

      expect(result.queryType).toBe('SELECT');
      expect(result.collection).toBe('docs');
      expect(result.plan).toHaveLength(2);
      expect(result.plan[0].operation).toBe('VectorSearch');
      expect(result.plan[0].estimatedRows).toBe(10);
      expect(result.plan[1].operation).toBe('Limit');
      expect(result.estimatedCost.usesIndex).toBe(true);
      expect(result.estimatedCost.indexName).toBe('HNSW');
      expect(result.estimatedCost.selectivity).toBe(0.01);
      expect(result.estimatedCost.complexity).toBe('O(log n)');
      expect(result.features.hasVectorSearch).toBe(true);
      expect(result.features.hasFilter).toBe(false);
      expect(result.features.hasAggregation).toBe(false);
      expect(result.features.limit).toBe(10);
      expect(result.features.offset).toBeUndefined();
    });

    it('should send params when provided', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          query: 'SELECT * FROM docs',
          query_type: 'SELECT',
          collection: 'docs',
          plan: [],
          estimated_cost: { uses_index: false, index_name: null, selectivity: 1.0, complexity: 'O(n)' },
          features: {
            has_vector_search: false, has_filter: false, has_order_by: false,
            has_group_by: false, has_aggregation: false, has_join: false, has_fusion: false,
          },
        }),
      });

      await backend.explain('SELECT * FROM docs', { v: [0.1, 0.2] });

      const callBody = JSON.parse(mockFetch.mock.calls[0][1].body);
      expect(callBody.query).toBe('SELECT * FROM docs');
      expect(callBody.params).toEqual({ v: [0.1, 0.2] });
    });

    it('should throw on parse error', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        json: () => Promise.resolve({ code: 'BAD_REQUEST', message: 'Parse error at position 5' }),
      });

      await expect(backend.explain('INVALID QUERY'))
        .rejects.toThrow(VelesDBError);
    });

    it('should handle null estimated_rows and index_name', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          query: 'SELECT * FROM docs',
          query_type: 'SELECT',
          collection: 'docs',
          plan: [{ step: 1, operation: 'FullScan', description: 'Scan docs', estimated_rows: null }],
          estimated_cost: { uses_index: false, index_name: null, selectivity: 1.0, complexity: 'O(n)' },
          features: {
            has_vector_search: false, has_filter: false, has_order_by: false,
            has_group_by: false, has_aggregation: false, has_join: false, has_fusion: false,
            limit: null, offset: null,
          },
        }),
      });

      const result = await backend.explain('SELECT * FROM docs');
      expect(result.plan[0].estimatedRows).toBeUndefined();
      expect(result.estimatedCost.indexName).toBeUndefined();
      expect(result.features.limit).toBeUndefined();
    });
  });

  describe('search efSearch and mode', () => {
    beforeEach(async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ status: 'ok' }),
      });
      await backend.init();
      vi.clearAllMocks();
    });

    it('should send ef_search when efSearch option is provided', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ results: [] }),
      });

      await backend.search('docs', [0.1, 0.2], { k: 10, efSearch: 200 });

      const callBody = JSON.parse(mockFetch.mock.calls[0][1].body);
      expect(callBody.ef_search).toBe(200);
      expect(callBody.mode).toBeUndefined();
    });

    it('should send mode when mode option is provided', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ results: [] }),
      });

      await backend.search('docs', [0.1, 0.2], { k: 10, mode: 'accurate' });

      const callBody = JSON.parse(mockFetch.mock.calls[0][1].body);
      expect(callBody.mode).toBe('accurate');
      expect(callBody.ef_search).toBeUndefined();
    });

    it('should not send ef_search or mode when not provided (backward compatible)', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ results: [] }),
      });

      await backend.search('docs', [0.1, 0.2], { k: 5 });

      const callBody = JSON.parse(mockFetch.mock.calls[0][1].body);
      expect(callBody.ef_search).toBeUndefined();
      expect(callBody.mode).toBeUndefined();
      expect(callBody.top_k).toBe(5);
    });
  });

  describe('edge cases', () => {
    beforeEach(async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ status: 'ok' }),
      });
      await backend.init();
      vi.clearAllMocks();
    });

    it('should return [] for search with empty results', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ results: [] }),
      });

      const results = await backend.search('docs', [0.1, 0.2]);
      expect(results).toEqual([]);
    });

    it('should return [] for listCollections with empty response', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ collections: [] }),
      });

      const cols = await backend.listCollections();
      expect(cols).toEqual([]);
    });

    it('should return null for getPoint with non-existent ID', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        json: () => Promise.resolve({ code: 'NOT_FOUND', message: 'Point not found' }),
      });

      const result = await backend.get('docs', 'nonexistent');
      expect(result).toBeNull();
    });

    it('should return false for delete on non-existent point (NOT_FOUND)', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        json: () => Promise.resolve({ code: 'NOT_FOUND', message: 'Not found' }),
      });

      const deleted = await backend.delete('docs', 'nonexistent');
      expect(deleted).toBe(false);
    });

    it('should return empty traverseGraph results', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          results: [],
          next_cursor: null,
          has_more: false,
          stats: { visited: 0, depth_reached: 0 },
        }),
      });

      const result = await backend.traverseGraph('social', { source: 1 });
      expect(result.results).toEqual([]);
      expect(result.hasMore).toBe(false);
      expect(result.stats.visited).toBe(0);
      expect(result.stats.depthReached).toBe(0);
    });

    it('should throw NotFoundError for query on non-existent collection', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        json: () => Promise.resolve({ code: 'NOT_FOUND', message: 'Collection not found' }),
      });

      await expect(backend.query('nonexistent', 'SELECT * FROM nonexistent'))
        .rejects.toThrow(NotFoundError);
    });

    it('should throw NotFoundError for matchQuery on non-existent collection', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        json: () => Promise.resolve({ code: 'NOT_FOUND', message: 'Collection not found' }),
      });

      await expect(backend.matchQuery('nonexistent', 'MATCH (a) RETURN a'))
        .rejects.toThrow(NotFoundError);
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
