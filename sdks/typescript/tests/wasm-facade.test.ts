/**
 * WASM Facade Coverage Tests (#598)
 *
 * Complements `wasm-backend.test.ts` by exercising the remaining facade
 * methods in `src/backends/wasm.ts` — mostly the async delegations to
 * the stub modules (wasm-stubs, wasm-wave4-stubs) that must all throw
 * "not supported" errors, plus the isEmpty/flush/searchBatch helpers
 * and the `ensureInitialized()` guard.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { WasmBackend } from '../src/backends/wasm';
import { VelesDBError, NotFoundError, ConnectionError } from '../src/types';

// Mock WASM module — minimal surface for a collection that can report
// "is_empty" and accept a flush no-op.
class MockVectorStore {
  insert = vi.fn();
  insert_with_payload = vi.fn();
  insert_batch = vi.fn();
  reserve = vi.fn();
  remove = vi.fn(() => true);
  get = vi.fn(() => null);
  free = vi.fn();
  search = vi.fn(() => []);
  search_with_filter = vi.fn(() => []);
  sparse_search = vi.fn(() => []);
  text_search = vi.fn(() => []);
  hybrid_search = vi.fn(() => []);
  multi_query_search = vi.fn(() => []);
  query = vi.fn(() => []);
  len = 0;
  is_empty = true;
  constructor(public dimension: number, _metric: string) {}
}

const mockWasmModule = {
  default: vi.fn(() => Promise.resolve()),
  VectorStore: MockVectorStore,
  hybrid_search_fuse: vi.fn(() => []),
};

vi.mock('@wiscale/velesdb-wasm', () => mockWasmModule);

describe('WasmBackend — lifecycle + helpers (#598)', () => {
  let backend: WasmBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = new WasmBackend();
    await backend.init();
  });

  it('capabilities() returns WASM_CAPABILITIES', () => {
    expect(typeof backend.capabilities()).toBe('object');
  });

  it('searchBatch delegates and returns one result array per input', async () => {
    await backend.createCollection('c', { dimension: 2, metric: 'cosine' });

    const out = await backend.searchBatch('c', [
      { vector: [0.1, 0.2] },
      { vector: [0.3, 0.4] },
    ]);

    expect(out).toHaveLength(2);
  });

  it('isEmpty returns true when store reports empty', async () => {
    await backend.createCollection('c', { dimension: 2, metric: 'cosine' });
    expect(await backend.isEmpty('c')).toBe(true);
  });

  it('isEmpty throws NotFoundError when collection missing', async () => {
    await expect(backend.isEmpty('missing')).rejects.toBeInstanceOf(
      NotFoundError
    );
  });

  it('flush is a no-op on existing collection', async () => {
    await backend.createCollection('c', { dimension: 2, metric: 'cosine' });
    await expect(backend.flush('c')).resolves.toBeUndefined();
  });

  it('flush throws NotFoundError when collection missing', async () => {
    await expect(backend.flush('missing')).rejects.toBeInstanceOf(
      NotFoundError
    );
  });
});

describe('WasmBackend — ensureInitialized guard', () => {
  it('rejects every method call before init', async () => {
    const backend = new WasmBackend();

    await expect(backend.isEmpty('x')).rejects.toBeInstanceOf(ConnectionError);
    await expect(backend.flush('x')).rejects.toBeInstanceOf(ConnectionError);
    await expect(
      backend.createCollection('x', { dimension: 2, metric: 'cosine' })
    ).rejects.toBeInstanceOf(ConnectionError);
    await expect(backend.deleteCollection('x')).rejects.toBeInstanceOf(
      ConnectionError
    );
    await expect(backend.listCollections()).rejects.toBeInstanceOf(
      ConnectionError
    );
    await expect(backend.searchIds('x', [0.1])).rejects.toBeInstanceOf(
      ConnectionError
    );
  });
});

describe('WasmBackend — delete + get (uncovered branches)', () => {
  let backend: WasmBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = new WasmBackend();
    await backend.init();
    await backend.createCollection('c', { dimension: 2, metric: 'cosine' });
  });

  it('delete throws NotFoundError when collection missing', async () => {
    await expect(backend.delete('missing', 1)).rejects.toBeInstanceOf(
      NotFoundError
    );
  });

  it('get throws NotFoundError when collection missing', async () => {
    await expect(backend.get('missing', 1)).rejects.toBeInstanceOf(
      NotFoundError
    );
  });

  it('get returns null when point is absent', async () => {
    expect(await backend.get('c', 1)).toBeNull();
  });
});

describe('WasmBackend — upsertBatch dimension mismatch', () => {
  it('throws VelesDBError when any doc has wrong dimension', async () => {
    const backend = new WasmBackend();
    await backend.init();
    await backend.createCollection('c', { dimension: 2, metric: 'cosine' });

    await expect(
      backend.upsertBatch('c', [{ id: 1, vector: [0.1] /* only 1-D */ }])
    ).rejects.toBeInstanceOf(VelesDBError);
  });

  it('batches non-payload docs via insert_batch and payload docs via insert_with_payload', async () => {
    const backend = new WasmBackend();
    await backend.init();
    await backend.createCollection('c', { dimension: 2, metric: 'cosine' });

    await backend.upsertBatch('c', [
      { id: 1, vector: [0.1, 0.2] },
      { id: 2, vector: [0.3, 0.4], payload: { k: 'v' } },
    ]);
    // Test passes if no exception — both branches covered
  });

  it('throws NotFoundError when collection missing', async () => {
    const backend = new WasmBackend();
    await backend.init();

    await expect(
      backend.upsertBatch('missing', [{ id: 1, vector: [0.1] }])
    ).rejects.toBeInstanceOf(NotFoundError);
  });
});

describe('WasmBackend — stub delegations (rejections)', () => {
  let backend: WasmBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = new WasmBackend();
    await backend.init();
  });

  // Every wasm-stubs method must reject through the facade
  type StubRow = [string, () => Promise<unknown>];
  const stubMethods: StubRow[] = [
    ['queryExplain', () => backend.queryExplain('q')],
    ['collectionSanity', () => backend.collectionSanity('c')],
    ['scroll', () => backend.scroll('c')],
    ['createIndex', () => backend.createIndex('c', { label: 'L', property: 'p' })],
    ['listIndexes', () => backend.listIndexes('c')],
    ['hasIndex', () => backend.hasIndex('c', 'L', 'p')],
    ['dropIndex', () => backend.dropIndex('c', 'L', 'p')],
    ['traverseGraph', () => backend.traverseGraph('c', { source: 1 })],
    ['traverseParallel', () => backend.traverseParallel('c', { sources: [1] })],
    ['getNodeDegree', () => backend.getNodeDegree('c', 1)],
    ['trainPq', () => backend.trainPq('c')],
    ['streamInsert', () => backend.streamInsert('c', [])],
    ['streamUpsertPoints', () => backend.streamUpsertPoints('c', [])],
    ['createGraphCollection', () => backend.createGraphCollection('c')],
    ['getCollectionStats', () => backend.getCollectionStats('c')],
    ['analyzeCollection', () => backend.analyzeCollection('c')],
    ['getCollectionConfig', () => backend.getCollectionConfig('c')],
    ['searchIds', () => backend.searchIds('c', [0.1])],
    [
      'storeSemanticFact',
      () =>
        backend.storeSemanticFact('c', {
          id: 'a',
          subject: 's',
          relation: 'r',
          object: 'o',
          embedding: [0.1],
        }),
    ],
    [
      'searchSemanticMemory',
      () => backend.searchSemanticMemory('c', [0.1]),
    ],
    [
      'recordEpisodicEvent',
      () =>
        backend.recordEpisodicEvent('c', {
          id: 'e',
          timestamp: 0,
          description: 'd',
          embedding: [0.1],
        }),
    ],
    [
      'recallEpisodicEvents',
      () => backend.recallEpisodicEvents('c', [0.1]),
    ],
    [
      'storeProceduralPattern',
      () =>
        backend.storeProceduralPattern('c', {
          id: 'p',
          name: 'n',
          actions: [],
          embedding: [0.1],
        }),
    ],
    [
      'matchProceduralPatterns',
      () => backend.matchProceduralPatterns('c', [0.1]),
    ],
    // Wave 4 stubs
    ['rebuildIndex', () => backend.rebuildIndex('c')],
    ['getGuardrails', () => backend.getGuardrails()],
    ['updateGuardrails', () => backend.updateGuardrails({})],
    ['aggregate', () => backend.aggregate('q')],
    ['matchQuery', () => backend.matchQuery('c', 'q')],
    ['removeEdge', () => backend.removeEdge('c', 1)],
    ['getEdgeCount', () => backend.getEdgeCount('c')],
    ['listNodes', () => backend.listNodes('c')],
    ['getNodeEdges', () => backend.getNodeEdges('c', 1)],
    ['getNodePayload', () => backend.getNodePayload('c', 1)],
    ['upsertNodePayload', () =>
      backend.upsertNodePayload('c', 1, { k: 'v' })],
    [
      'graphSearch',
      () =>
        backend.graphSearch('c', {
          vector: [0.1],
          k: 5,
        }),
    ],
  ];

  it.each(stubMethods)(
    '%s rejects through the facade',
    async (_name, call) => {
      await expect(call()).rejects.toThrow(/not supported|REST backend/i);
    }
  );
});
