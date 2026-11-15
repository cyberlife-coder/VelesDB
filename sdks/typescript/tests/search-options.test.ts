/**
 * Parity-audit tests for the search/query/stats option and response fields
 * that were missing or truncated: ef_search, timeout_ms, fusion,
 * sparse_vectors on /search, full batch entries, rrf_k on /search/hybrid,
 * took_ms + meta on /query, the extended EXPLAIN fields, index_stats on
 * /stats, and the /aggregate -> /query fallback.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  search,
  searchBatch,
  searchIds,
  hybridSearch,
  sparseSearchNamed,
  type SearchTransport,
} from '../src/backends/search-backend';
import {
  query,
  queryExplain,
  type QueryTransport,
} from '../src/backends/query-backend';
import {
  getCollectionStats,
  analyzeCollection,
  type AdminTransport,
} from '../src/backends/admin-backend';
import { VelesDB } from '../src/client';
import { VelesDBError } from '../src/types';

function buildSearchTransport(): SearchTransport {
  return {
    requestJson: vi.fn().mockResolvedValue({ data: { results: [] } }),
    sparseToRest: (sv: Record<number, number>) => {
      const out: Record<string, number> = {};
      for (const [k, v] of Object.entries(sv)) out[String(k)] = v;
      return out;
    },
  };
}

function bodyOf(transport: SearchTransport, call = 0): Record<string, unknown> {
  return (transport.requestJson as ReturnType<typeof vi.fn>).mock.calls[call]![2] as Record<string, unknown>;
}

describe('search option fields (full SearchRequest)', () => {
  beforeEach(() => vi.clearAllMocks());

  it('search forwards ef_search, timeout_ms, fusion and sparse_vectors', async () => {
    const t = buildSearchTransport();
    await search(t, 'docs', [0.1, 0.2], {
      k: 3,
      efSearch: 128,
      timeoutMs: 5000,
      fusion: { strategy: 'weighted', avgWeight: 0.7, maxWeight: 0.2, hitWeight: 0.1 },
      sparseVectors: { title: { 7: 0.9 } },
    });
    const body = bodyOf(t);
    expect(body.top_k).toBe(3);
    expect(body.ef_search).toBe(128);
    expect(body.timeout_ms).toBe(5000);
    expect(body.fusion).toEqual({
      strategy: 'weighted',
      k: undefined,
      dense_w: undefined,
      sparse_w: undefined,
      avg_w: 0.7,
      max_w: 0.2,
      hit_w: 0.1,
    });
    expect(body.sparse_vectors).toEqual({ title: { '7': 0.9 } });
  });

  it('searchBatch forwards full per-entry fields', async () => {
    const t = buildSearchTransport();
    (t.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [{ results: [] }, { results: [] }] },
    });
    await searchBatch(t, 'docs', [
      { vector: [0.1], k: 2, efSearch: 64, timeoutMs: 100, sparseVector: { 1: 0.5 } },
      { vector: [0.2], quality: 'fast', filter: { condition: { type: 'eq', field: 'a', value: 1 } } },
    ]);
    const body = bodyOf(t);
    const searches = body.searches as Record<string, unknown>[];
    expect(searches[0]).toMatchObject({ top_k: 2, ef_search: 64, timeout_ms: 100, sparse_vector: { '1': 0.5 } });
    expect(searches[1]).toMatchObject({ top_k: 10, mode: 'fast' });
    expect(searches[1]!.filter).toBeDefined();
  });

  it('searchIds forwards the full option bag', async () => {
    const t = buildSearchTransport();
    await searchIds(t, 'docs', [0.1], { k: 4, efSearch: 32, timeoutMs: 250 });
    const body = bodyOf(t);
    expect(body.top_k).toBe(4);
    expect(body.ef_search).toBe(32);
    expect(body.timeout_ms).toBe(250);
  });

  it('hybridSearch forwards rrf_k', async () => {
    const t = buildSearchTransport();
    await hybridSearch(t, 'docs', [0.1], 'rust', { k: 5, vectorWeight: 0.8, rrfK: 42 });
    const body = bodyOf(t);
    expect(body).toMatchObject({ top_k: 5, vector_weight: 0.8, rrf_k: 42, query: 'rust' });
  });

  it('sparseSearchNamed forwards ef_search/timeout_ms and pins the named index', async () => {
    const t = buildSearchTransport();
    await sparseSearchNamed(t, 'docs', { 3: 1.0 }, 'splade', { k: 2, efSearch: 16, timeoutMs: 50 });
    const body = bodyOf(t);
    expect(body.sparse_index).toBe('splade');
    expect(body.sparse_vectors).toEqual({ splade: { '3': 1.0 } });
    expect(body.ef_search).toBe(16);
    expect(body.timeout_ms).toBe(50);
  });
});

describe('client refuses REST-only options on a non-REST backend', () => {
  it('throws NOT_SUPPORTED instead of silently dropping efSearch on wasm', async () => {
    const db = new VelesDB({ backend: 'wasm' });
    // skip init: the guard runs after ensureInitialized, so mark initialized
    (db as unknown as { initialized: boolean }).initialized = true;
    await expect(
      db.search('c', [0.1], { efSearch: 128 })
    ).rejects.toMatchObject({ code: 'NOT_SUPPORTED' });
    await expect(
      db.hybridSearch('c', [0.1], 'q', { rrfK: 10 })
    ).rejects.toMatchObject({ code: 'NOT_SUPPORTED' });
    await expect(
      db.searchBatch('c', [{ vector: [0.1], timeoutMs: 5 }])
    ).rejects.toMatchObject({ code: 'NOT_SUPPORTED' });
    await expect(db.search('c', [0.1], { timeoutMs: 5 })).rejects.toBeInstanceOf(VelesDBError);
  });
});

describe('query response mapping (took_ms, meta)', () => {
  it('forwards took_ms and meta from /query', async () => {
    const requestJson = vi.fn().mockResolvedValue({
      data: {
        results: [{ id: 1 }],
        timing_ms: 1.5,
        took_ms: 2,
        rows_returned: 1,
        meta: { velesql_contract_version: '2.0', count: 1 },
      },
    });
    const t: QueryTransport = { requestJson, parseNodeId: (v) => Number(v) };
    const res = await query(t, 'docs', 'SELECT * FROM docs');
    expect(res).toMatchObject({ tookMs: 2, meta: { velesqlContractVersion: '2.0', count: 1 } });
    expect(requestJson).toHaveBeenCalledWith('POST', '/query', expect.objectContaining({ collection: 'docs' }));
  });

  it('falls back to /query when /aggregate refuses a misrouted SELECT', async () => {
    const requestJson = vi
      .fn()
      .mockResolvedValueOnce({
        error: { code: 'BAD_REQUEST', message: 'Only aggregation queries are accepted on /aggregate' },
      })
      .mockResolvedValueOnce({
        data: { results: [{ id: 1 }], timing_ms: 0.5, took_ms: 1, rows_returned: 1, meta: { velesql_contract_version: '2.0', count: 1 } },
      });
    const t: QueryTransport = { requestJson, parseNodeId: (v) => Number(v) };
    // The string literal contains "count(" but the query is not an aggregation.
    const res = await query(t, 'docs', "SELECT * FROM docs WHERE title = 'count(1) please'");
    expect(requestJson).toHaveBeenNthCalledWith(1, 'POST', '/aggregate', expect.anything());
    expect(requestJson).toHaveBeenNthCalledWith(2, 'POST', '/query', expect.anything());
    expect(res).toMatchObject({ tookMs: 1 });
  });

  it('does not fall back on a non-validation /aggregate error', async () => {
    const requestJson = vi.fn().mockResolvedValue({
      error: { code: 'VELES-002', message: "Collection 'missing' not found" },
    });
    const t: QueryTransport = { requestJson, parseNodeId: (v) => Number(v) };
    await expect(query(t, 'missing', 'SELECT COUNT(*) FROM missing')).rejects.toMatchObject({
      code: 'VELES-002',
    });
    expect(requestJson).toHaveBeenCalledTimes(1);
  });
});

describe('explain response mapping (extended fields)', () => {
  it('maps cache_hit, plan_reuse_count, estimated_cost_ms, actual_time_ms and actual_stats extras', async () => {
    const requestJson = vi.fn().mockResolvedValue({
      data: {
        query: 'SELECT * FROM docs',
        query_type: 'select',
        collection: 'docs',
        plan: [],
        estimated_cost: { uses_index: true, index_name: 'hnsw', selectivity: 0.1, complexity: 'low' },
        features: {
          has_vector_search: false, has_filter: false, has_order_by: false,
          has_group_by: false, has_aggregation: false, has_join: false,
          has_fusion: false, limit: 10, offset: null,
        },
        cache_hit: true,
        plan_reuse_count: 7,
        estimated_cost_ms: 0.4,
        actual_time_ms: 1.2,
        actual_stats: {
          actual_rows: 3,
          actual_time_ms: 1.2,
          loops: 1,
          nodes_visited: 10,
          edges_traversed: 4,
          traversal_counters_approximate: true,
          executed_filter_strategy: 'prefilter',
        },
        node_stats: null,
      },
    });
    const t: QueryTransport = { requestJson, parseNodeId: (v) => Number(v) };
    const res = await queryExplain(t, 'SELECT * FROM docs', undefined, { analyze: true });
    expect(res.cacheHit).toBe(true);
    expect(res.planReuseCount).toBe(7);
    expect(res.estimatedCostMs).toBe(0.4);
    expect(res.actualTimeMs).toBe(1.2);
    expect(res.actualStats).toMatchObject({
      traversalCountersApproximate: true,
      executedFilterStrategy: 'prefilter',
    });
  });
});

describe('stats response mapping (index_stats, nullable last_analyzed)', () => {
  it('maps index_stats and a null last_analyzed_epoch_ms', async () => {
    const requestJson = vi.fn().mockResolvedValue({
      data: {
        total_points: 10,
        total_size_bytes: 1024,
        row_count: 10,
        deleted_count: 0,
        avg_row_size_bytes: 102,
        payload_size_bytes: 512,
        last_analyzed_epoch_ms: null,
        column_stats: {},
        index_stats: {
          hnsw: { name: 'hnsw', index_type: 'HNSW', entry_count: 10, depth: 4, size_bytes: 4096 },
        },
      },
    });
    const t: AdminTransport = { requestJson };
    const res = await getCollectionStats(t, 'docs');
    expect(res).not.toBeNull();
    expect(res!.lastAnalyzedEpochMs).toBeNull();
    expect(res!.indexStats).toEqual({
      hnsw: { name: 'hnsw', indexType: 'HNSW', entryCount: 10, depth: 4, sizeBytes: 4096 },
    });
  });

  it('analyzeCollection maps index_stats too', async () => {
    const requestJson = vi.fn().mockResolvedValue({
      data: {
        total_points: 1, total_size_bytes: 1, row_count: 1, deleted_count: 0,
        avg_row_size_bytes: 1, payload_size_bytes: 1, last_analyzed_epoch_ms: 123,
        column_stats: {},
        index_stats: { bm25: { name: 'bm25', index_type: 'BM25', entry_count: 1, depth: 0, size_bytes: 8 } },
      },
    });
    const t: AdminTransport = { requestJson };
    const res = await analyzeCollection(t, 'docs');
    expect(res.lastAnalyzedEpochMs).toBe(123);
    expect(res.indexStats!.bm25).toMatchObject({ indexType: 'BM25', sizeBytes: 8 });
  });
});
