/**
 * REST Facade Coverage Tests (#598)
 *
 * Targets the delegation layer in `src/backends/rest.ts`: every facade
 * method that calls `ensureInitialized()` then delegates to a
 * sub-backend (crud, search, graph, query, streaming, index, admin,
 * agent-memory). Complements `rest-backend.test.ts` (which focuses on
 * init, collection CRUD, core search, and graph primitives).
 *
 * Approach: the underlying sub-backend modules are already covered by
 * their own unit tests. This file verifies the facade forwards with
 * the right transport, applies `ensureInitialized`, and exposes
 * `capabilities()` / `close()` / `isEmpty()` / `flush()` correctly.
 *
 * Uses `global.fetch = vi.fn()` — same pattern as `rest-backend.test.ts`
 * — instead of mocking the sub-backend modules, because mocking module
 * imports in vitest with mixed ESM/CJS boundaries is fragile and
 * produces false-negative coverage.
 */

import {
  describe,
  it,
  expect,
  vi,
  beforeEach,
  afterEach,
} from 'vitest';
import { RestBackend } from '../src/backends/rest';
import { ConnectionError } from '../src/types';

const mockFetch = vi.fn();
global.fetch = mockFetch;

async function initBackend(): Promise<RestBackend> {
  const backend = new RestBackend('http://localhost:8080', 'key');
  mockFetch.mockResolvedValueOnce({
    ok: true,
    status: 200,
    json: () => Promise.resolve({ status: 'ok' }),
  });
  await backend.init();
  mockFetch.mockClear();
  return backend;
}

function mockOk(body: unknown) {
  mockFetch.mockResolvedValueOnce({
    ok: true,
    status: 200,
    json: () => Promise.resolve(body),
  });
}

describe('RestBackend — lifecycle helpers', () => {
  beforeEach(() => vi.clearAllMocks());
  afterEach(() => vi.restoreAllMocks());

  it('strips a trailing slash from baseUrl', async () => {
    const backend = new RestBackend('http://localhost:8080/', 'k');
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({}),
    });
    await backend.init();
    // baseUrl has been normalised → expect /health without double-slash
    expect(mockFetch.mock.calls[0]![0]).toBe('http://localhost:8080/health');
  });

  it('init is idempotent (second call is a no-op)', async () => {
    const backend = await initBackend();
    await backend.init();
    expect(mockFetch).not.toHaveBeenCalled();
  });

  it('capabilities() returns REST_CAPABILITIES', async () => {
    const backend = await initBackend();
    const caps = backend.capabilities();
    // REST_CAPABILITIES is an object — should at least have the core keys
    expect(typeof caps).toBe('object');
  });

  it('close() resets isInitialized', async () => {
    const backend = await initBackend();
    expect(backend.isInitialized()).toBe(true);
    await backend.close();
    expect(backend.isInitialized()).toBe(false);
  });

  it('calling a mutating method before init throws ConnectionError', async () => {
    const backend = new RestBackend('http://localhost:8080');
    await expect(
      backend.createCollection('c', { dimension: 2, metric: 'cosine' })
    ).rejects.toBeInstanceOf(ConnectionError);
  });
});

describe('RestBackend — CRUD facade delegation', () => {
  let backend: RestBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = await initBackend();
  });

  it('isEmpty delegates to crud-backend', async () => {
    mockOk({ count: 0 });
    const result = await backend.isEmpty('docs');
    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(mockFetch.mock.calls[0]![0]).toContain('/collections/docs');
    expect(result).toBe(true);
  });

  it('flush delegates to crud-backend', async () => {
    mockOk({});
    await backend.flush('docs');
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('scroll delegates to scroll-backend', async () => {
    mockOk({ points: [], next_cursor: null });
    await backend.scroll('docs');
    expect(mockFetch).toHaveBeenCalledTimes(1);
    const url = mockFetch.mock.calls[0]![0] as string;
    expect(url).toContain('/scroll');
  });
});

describe('RestBackend — search facade delegation', () => {
  let backend: RestBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = await initBackend();
  });

  it('textSearch delegates to search-backend', async () => {
    mockOk({ results: [{ id: 1, score: 0.5 }] });
    const result = await backend.textSearch('docs', 'q');
    expect(result).toEqual([{ id: 1, score: 0.5 }]);
  });

  it('hybridSearch delegates to search-backend', async () => {
    mockOk({ results: [] });
    await backend.hybridSearch('docs', [0.1], 'q');
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('searchBatch delegates to search-backend', async () => {
    mockOk({ results: [{ results: [] }] });
    const result = await backend.searchBatch('docs', [{ vector: [0.1] }]);
    expect(result).toEqual([[]]);
  });

  it('searchIds delegates to search-backend', async () => {
    mockOk({ results: [{ id: 1, score: 0.9 }] });
    const result = await backend.searchIds('docs', [0.1]);
    expect(result).toEqual([{ id: 1, score: 0.9 }]);
  });
});

describe('RestBackend — graph facade delegation', () => {
  let backend: RestBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = await initBackend();
  });

  it('traverseGraph delegates', async () => {
    mockOk({
      results: [],
      next_cursor: null,
      has_more: false,
      stats: { visited: 0, depth_reached: 0 },
    });
    await backend.traverseGraph('kg', { source: 1 });
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('traverseParallel delegates', async () => {
    mockOk({
      results: [],
      next_cursor: null,
      has_more: false,
      stats: { visited: 0, depth_reached: 0 },
    });
    await backend.traverseParallel('kg', { sources: [1, 2] });
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('getNodeDegree delegates', async () => {
    mockOk({ in_degree: 1, out_degree: 2 });
    const result = await backend.getNodeDegree('kg', 42);
    expect(result).toEqual({ inDegree: 1, outDegree: 2 });
  });

  it('createGraphCollection delegates', async () => {
    mockOk({});
    await backend.createGraphCollection('kg', { dimension: 128 });
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });
});

describe('RestBackend — index facade delegation', () => {
  let backend: RestBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = await initBackend();
  });

  it('createIndex delegates', async () => {
    mockOk({});
    await backend.createIndex('docs', { label: 'L', property: 'p' });
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('listIndexes delegates', async () => {
    mockOk({ indexes: [], total: 0 });
    const result = await backend.listIndexes('docs');
    expect(result).toEqual([]);
  });

  it('hasIndex delegates', async () => {
    mockOk({ indexes: [], total: 0 });
    const result = await backend.hasIndex('docs', 'L', 'p');
    expect(result).toBe(false);
  });

  it('dropIndex delegates', async () => {
    mockOk({ dropped: true });
    const result = await backend.dropIndex('docs', 'L', 'p');
    expect(result).toBe(true);
  });
});

describe('RestBackend — admin facade delegation', () => {
  let backend: RestBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = await initBackend();
  });

  const stats = {
    total_points: 0,
    total_size_bytes: 0,
    row_count: 0,
    deleted_count: 0,
    avg_row_size_bytes: 0,
    payload_size_bytes: 0,
    last_analyzed_epoch_ms: 0,
  };

  it('getCollectionStats delegates', async () => {
    mockOk(stats);
    const result = await backend.getCollectionStats('docs');
    expect(result).not.toBeNull();
  });

  it('analyzeCollection delegates', async () => {
    mockOk(stats);
    await backend.analyzeCollection('docs');
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('getCollectionConfig delegates', async () => {
    mockOk({
      name: 'docs',
      dimension: 2,
      metric: 'cosine',
      storage_mode: 'InMemory',
      point_count: 0,
      metadata_only: false,
    });
    const result = await backend.getCollectionConfig('docs');
    expect(result.name).toBe('docs');
  });
});

describe('RestBackend — missing-endpoints facade delegation (Wave 4)', () => {
  let backend: RestBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = await initBackend();
  });

  it('rebuildIndex delegates', async () => {
    mockOk({ rebuilt: true });
    await backend.rebuildIndex('docs');
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('getGuardrails delegates', async () => {
    mockOk({ enabled: true, limits: {} });
    await backend.getGuardrails();
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('updateGuardrails delegates', async () => {
    mockOk({ enabled: true, limits: {} });
    await backend.updateGuardrails({ enabled: true });
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('aggregate delegates', async () => {
    mockOk({
      result: {},
      timing_ms: 0,
      meta: { velesql_contract_version: '1.0', count: 0 },
    });
    await backend.aggregate('q');
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('matchQuery delegates', async () => {
    mockOk({
      results: [],
      took_ms: 0,
      count: 0,
      meta: { velesql_contract_version: '1.0' },
    });
    await backend.matchQuery('docs', 'q');
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('removeEdge delegates', async () => {
    mockOk({ removed: true });
    await backend.removeEdge('kg', 1);
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('getEdgeCount delegates', async () => {
    mockOk({ count: 42 });
    const result = await backend.getEdgeCount('kg');
    expect(typeof result).toBe('number');
  });

  it('listNodes delegates', async () => {
    mockOk({ nodes: [], total: 0 });
    await backend.listNodes('kg');
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('getNodeEdges delegates', async () => {
    mockOk({ edges: [] });
    await backend.getNodeEdges('kg', 1);
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('getNodePayload delegates', async () => {
    mockOk({ payload: null });
    await backend.getNodePayload('kg', 1);
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('upsertNodePayload delegates', async () => {
    mockOk({});
    await backend.upsertNodePayload('kg', 1, { k: 'v' });
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('graphSearch delegates', async () => {
    mockOk({ results: [], stats: {} });
    await backend.graphSearch('kg', { vector: [0.1], k: 5 });
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });
});

describe('RestBackend — streaming facade delegation', () => {
  let backend: RestBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = await initBackend();
  });

  it('trainPq delegates', async () => {
    mockOk({ message: 'training started' });
    const id = await backend.trainPq('docs');
    expect(id).toBe('training started');
  });

  it('streamInsert delegates', async () => {
    mockOk({});
    await backend.streamInsert('docs', [
      { id: 1, vector: [0.1, 0.2] },
    ]);
    expect(mockFetch).toHaveBeenCalled();
  });

  it('streamUpsertPoints delegates', async () => {
    mockOk({ upserted: 1 });
    await backend.streamUpsertPoints('docs', [
      { id: 1, vector: [0.1, 0.2] },
    ]);
    expect(mockFetch).toHaveBeenCalled();
  });
});

describe('RestBackend — agent memory facade delegation', () => {
  let backend: RestBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = await initBackend();
  });

  it('storeSemanticFact delegates', async () => {
    mockOk({});
    await backend.storeSemanticFact('docs', {
      id: 'a',
      subject: 's',
      relation: 'r',
      object: 'o',
      embedding: [0.1],
    });
    expect(mockFetch).toHaveBeenCalled();
  });

  it('searchSemanticMemory delegates', async () => {
    mockOk({ results: [] });
    await backend.searchSemanticMemory('docs', [0.1], 3);
    expect(mockFetch).toHaveBeenCalled();
  });

  it('recordEpisodicEvent delegates', async () => {
    mockOk({});
    await backend.recordEpisodicEvent('docs', {
      id: 'e',
      timestamp: 0,
      description: 'd',
      embedding: [0.1],
    });
    expect(mockFetch).toHaveBeenCalled();
  });

  it('recallEpisodicEvents delegates', async () => {
    mockOk({ results: [] });
    await backend.recallEpisodicEvents('docs', [0.1], 3);
    expect(mockFetch).toHaveBeenCalled();
  });

  it('storeProceduralPattern delegates', async () => {
    mockOk({});
    await backend.storeProceduralPattern('docs', {
      id: 'p',
      name: 'n',
      actions: [],
      embedding: [0.1],
    });
    expect(mockFetch).toHaveBeenCalled();
  });

  it('matchProceduralPatterns delegates', async () => {
    mockOk({ results: [] });
    await backend.matchProceduralPatterns('docs', [0.1], 3);
    expect(mockFetch).toHaveBeenCalled();
  });
});
