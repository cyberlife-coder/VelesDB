/**
 * WASM Backend Integration Tests
 * 
 * Tests the WasmBackend class with mock WASM module
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { WasmBackend } from '../src/backends/wasm';
import { VelesDBError, NotFoundError, ConnectionError } from '../src/types';
import { FakeSparseIndex } from './helpers/fake-sparse-index';

// Mock WASM module with class-based VectorStore
class MockVectorStore {
  insert = vi.fn();
  insert_with_payload = vi.fn();
  insert_batch = vi.fn();
  search = vi.fn(() => [[BigInt(1), 0.95], [BigInt(2), 0.85]]);
  // The binding parses the preset, then runs the same brute-force search
  // (`search_with_quality` in velesdb-wasm's `vector_store.rs`).
  search_with_quality = vi.fn((q: Float32Array, k: number) => this.search(q, k));
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
  storage_mode = 'full';
  readonly sparse = new FakeSparseIndex();
  sparse_insert = vi.fn((id: bigint, indices: Uint32Array, values: Float32Array) =>
    this.sparse.insert(id, indices, values)
  );
  sparse_search = vi.fn((indices: Uint32Array, values: Float32Array, k: number) =>
    this.sparse.search(indices, values, k)
  );

  constructor(dimension: number, _metric: string) {
    this.dimension = dimension;
  }

  static new_with_mode(dimension: number, metric: string, mode: string): MockVectorStore {
    const store = new MockVectorStore(dimension, metric);
    store.storage_mode = mode;
    return store;
  }

  static new_metadata_only(): MockVectorStore {
    return new MockVectorStore(0, 'cosine');
  }
}

const mockWasmModule = {
  default: vi.fn(() => Promise.resolve()),
  VectorStore: MockVectorStore,
};

// Mock the dynamic import - must match the import path in wasm.ts
vi.mock('@wiscale/velesdb-wasm', () => mockWasmModule);

// Stub the Node-only loader so the unit suite does not touch the real
// filesystem. Without this, `WasmBackend.init()` would call into
// `loadWasmBytesNode()` which uses `createRequire` + `fs.readdir` to
// locate the real velesdb_wasm package on disk — turning these unit
// tests into integration tests that depend on `@wiscale/velesdb-wasm`
// being installed AND containing a `.wasm` binary. (Devin Review PR #710.)
vi.mock('../src/backends/wasm-node-loader', () => ({
  isNodeRuntime: () => false,
  loadWasmBytesNode: vi.fn(() => Promise.resolve(new Uint8Array(0))),
}));

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

    it("keeps the loader's reason, which lives only in the message (#2282)", async () => {
      // Probed on 6.0.0: `mod.default()` rejects with a
      // `WebAssembly.CompileError`. The reason has to reach the message
      // anyway — a caller reading `err.message` sees only "Failed to
      // initialize WASM module" unless it is carried there.
      mockWasmModule.default.mockRejectedValueOnce(
        new WebAssembly.CompileError(
          'WebAssembly.instantiate(): expected magic word 00 61 73 6d'
        )
      );

      await expect(backend.init()).rejects.toThrow(
        /Failed to initialize WASM module: WebAssembly\.instantiate\(\): expected magic word 00 61 73 6d/
      );
    });

    it('should coalesce concurrent init() calls into one wasm-bindgen invocation', async () => {
      // Pre-existing TOCTOU race in init() flagged by Devin Review on PR #709:
      // two callers entering init() before _initialized is set both raced into
      // wasm-bindgen's default(). The fix memoizes a single in-flight promise.
      // This test makes sure the wasm `default()` initializer is invoked
      // exactly once even with three concurrent init() callers.
      const defaultSpy = mockWasmModule.default;
      defaultSpy.mockClear();
      await Promise.all([backend.init(), backend.init(), backend.init()]);
      expect(defaultSpy).toHaveBeenCalledTimes(1);
      expect(backend.isInitialized()).toBe(true);
    });

    it('should not flip back to initialized when close() races an in-flight init()', async () => {
      // Devin Review on PR #709 round 2 flagged that close() during in-flight
      // runInit() did not cancel: runInit() would still complete and set
      // _initialized = true after close() set it false. The fix bumps a
      // generation counter in close() that runInit() captures at entry and
      // checks before publishing _initialized. This test simulates the race
      // by starting init() and calling close() before the awaited promise
      // resolves, then ensures the backend stays closed.
      const initPromise = backend.init();
      await backend.close();
      await initPromise;
      expect(backend.isInitialized()).toBe(false);
    });

    it('should null out wasmModule on close()', async () => {
      // Devin Review on PR #709 round 2: close() previously left wasmModule
      // set, so a follow-up init() reused a stale handle. We now clear it
      // and re-import on the next init().
      await backend.init();
      expect(backend.isInitialized()).toBe(true);
      await backend.close();
      expect(backend.isInitialized()).toBe(false);
      // Re-init must succeed (i.e. the cleared module reference does not
      // break the lifecycle).
      await backend.init();
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

    it('should upsert a vector', async () => {
      await backend.upsert('vectors', {
        id: '1',
        vector: [1.0, 0.0, 0.0, 0.0],
        payload: { title: 'Test' },
      });

      const collections = (backend as any).collections;
      const collection = collections.get('vectors');
      const store = collection.store as MockVectorStore;

      // payload-bearing upsert must dispatch to insert_with_payload, not insert
      expect(store.insert_with_payload).toHaveBeenCalledTimes(1);
      expect(store.insert_with_payload).toHaveBeenCalledWith(
        BigInt(1),
        expect.any(Float32Array),
        { title: 'Test' },
      );
      expect(store.insert).not.toHaveBeenCalled();

      // and the in-memory payload map must be populated (read by get())
      expect(collection.payloads.get('1')).toEqual({ title: 'Test' });
    });

    it('should get a vector by id', async () => {
      await backend.upsert('vectors', {
        id: 'abc',
        vector: [1.0, 0.0, 0.0, 0.0],
        payload: { title: 'Stored' },
      });
      const doc = await backend.get('vectors', 'abc');
      expect(doc).not.toBeNull();
      expect(doc?.payload).toEqual({ title: 'Stored' });
    });

    it('should throw on dimension mismatch', async () => {
      await expect(backend.upsert('vectors', {
        id: '1',
        vector: [1.0, 0.0], // Wrong dimension
      })).rejects.toThrow('dimension mismatch');
    });

    // A mutant of either code literal below leaves the rest of the suite
    // green (#2341): only asserting `.code` pins the promised error code
    // against a silent rename or copy-paste drift.
    it('tags a dimension mismatch on upsert with DIMENSION_MISMATCH', async () => {
      const outcome = await backend.upsert('vectors', {
        id: '1',
        vector: [1.0, 0.0],
      }).catch((err) => err);

      expect(outcome).toBeInstanceOf(VelesDBError);
      expect((outcome as VelesDBError).code).toBe('DIMENSION_MISMATCH');
      expect((outcome as VelesDBError).message).toMatch(/expected 4, got 2/);
    });

    it('should throw on non-existent collection', async () => {
      await expect(backend.upsert('nonexistent', {
        id: '1',
        vector: [1.0, 0.0, 0.0, 0.0],
      })).rejects.toThrow(NotFoundError);
    });

    it('should upsert batch', async () => {
      await backend.upsertBatch('vectors', [
        { id: '1', vector: [1.0, 0.0, 0.0, 0.0] },
        { id: '2', vector: [0.0, 1.0, 0.0, 0.0] },
      ]);

      const collections = (backend as any).collections;
      const store = collections.get('vectors').store as MockVectorStore;

      expect(store.insert_with_payload).not.toHaveBeenCalled();
      expect(store.insert_batch).toHaveBeenCalledTimes(1);
      expect(store.insert_batch).toHaveBeenCalledWith([
        [BigInt(1), [1.0, 0.0, 0.0, 0.0]],
        [BigInt(2), [0.0, 1.0, 0.0, 0.0]],
      ]);
    });

    it('should forward payload docs in upsertBatch via insert_with_payload', async () => {
      await backend.upsertBatch('vectors', [
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

    it.each([
      ['first', [{ id: '1', vector: [1.0, 0.0] }, { id: '2', vector: [0.0, 1.0, 0.0, 0.0] }]],
      ['last', [{ id: '1', vector: [1.0, 0.0, 0.0, 0.0] }, { id: '2', vector: [0.0, 1.0] }]],
    ])(
      'tags a dimension mismatch on upsertBatch with DIMENSION_MISMATCH when the bad vector is %s, refusing before any insert',
      async (_position, docs) => {
        const collections = (backend as any).collections;
        const store = collections.get('vectors').store as MockVectorStore;

        const outcome = await backend.upsertBatch('vectors', docs).catch((err) => err);

        expect(outcome).toBeInstanceOf(VelesDBError);
        expect((outcome as VelesDBError).code).toBe('DIMENSION_MISMATCH');
        expect((outcome as VelesDBError).message).toMatch(/dimension mismatch for doc/i);
        expect(store.insert_batch).not.toHaveBeenCalled();
        expect(store.insert_with_payload).not.toHaveBeenCalled();
      },
    );

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
      await backend.upsert('vectors', {
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

    it('should accept weighted fusion strategy', async () => {
      const collections = (backend as any).collections;
      const store = collections.get('vectors').store as MockVectorStore;

      const results = await backend.multiQuerySearch('vectors', [[0.1, 0.2, 0.3, 0.4]], {
        fusion: 'weighted',
        fusionParams: { avgWeight: 0.6, maxWeight: 0.3, hitWeight: 0.1 },
      });

      // The weights reach the binding as its sixth argument, [avg, max, hit].
      // This test used to pin a five-argument call, i.e. the drop (#2095).
      expect(store.multi_query_search).toHaveBeenCalledWith(
        expect.any(Float32Array), 1, 10, 'weighted', 60, expect.any(Float32Array),
      );
      const weights = store.multi_query_search.mock.calls[0][5] as Float32Array;
      expect(Array.from(weights)).toEqual(Array.from(new Float32Array([0.6, 0.3, 0.1])));
      expect(results.length).toBe(1);
      expect(results[0].score).toBe(0.95);
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

// ---------------------------------------------------------------------------
// #2095 — what upsert, createCollection and query honour, and what they refuse
// ---------------------------------------------------------------------------

/** Settle `promise`, returning its value or what it rejected with. */
async function settle<T>(promise: Promise<T>): Promise<unknown> {
  return promise.then(
    (value) => value,
    (error: unknown) => error
  );
}

function expectRefusal(outcome: unknown, capability: string): void {
  expect(outcome).toBeInstanceOf(VelesDBError);
  const err = outcome as VelesDBError;
  expect(err.code).toBe('NOT_SUPPORTED');
  expect(err.message).toMatch(/WASM backend/);
  expect(err.message).toContain(capability);
}

function sparseIdsOf(
  backend: WasmBackend,
  collection: string
): {
  store: MockVectorStore | null;
  dead: number;
  byId: Map<bigint, unknown>;
  vectors: Map<bigint, unknown>;
} {
  const internals = backend as unknown as {
    collections: Map<
      string,
      {
        sparseIds: {
          store: MockVectorStore | null;
          dead: number;
          byId: Map<bigint, unknown>;
          vectors: Map<bigint, unknown>;
        };
      }
    >;
  };
  return internals.collections.get(collection)!.sparseIds;
}

function storeOf(backend: WasmBackend, collection: string): MockVectorStore {
  const internals = backend as unknown as {
    collections: Map<string, { store: MockVectorStore }>;
  };
  return internals.collections.get(collection)!.store;
}

describe('WasmBackend — upsert indexes sparse vectors; sparse search sees live ones only (#2095)', () => {
  let backend: WasmBackend;
  const hitIds = async (sparseVector: Record<number, number>, k = 10) =>
    (await backend.search('s', [], { k, sparseVector })).map((r) => r.id);

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = new WasmBackend();
    await backend.init();
    await backend.createCollection('s', { dimension: 0 });
  });

  it('finds a point by the sparse vector it was upserted with', async () => {
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 1 } });
    await backend.upsert('s', { id: 2, vector: [], sparseVector: { 9: 1 } });

    expect(await hitIds({ 7: 1 })).toEqual(['1']);
  });

  it('upsertBatch indexes each sparse vector too', async () => {
    await backend.upsertBatch('s', [
      { id: 1, vector: [], sparseVector: { 7: 1 } },
      { id: 2, vector: [], payload: { p: 1 }, sparseVector: { 7: 0.5 } },
    ]);

    expect(await hitIds({ 7: 1 })).toEqual(['1', '2']);
  });

  it('never returns a deleted point, though the binding keeps its postings', async () => {
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 1 } });
    await backend.delete('s', 1);

    expect(await hitIds({ 7: 1 })).toEqual([]);
  });

  it('never returns a point bulkDelete removed', async () => {
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 1 } });
    await backend.upsert('s', { id: 2, vector: [], sparseVector: { 7: 0.5 } });
    await backend.bulkDelete('s', [1]);

    expect(await hitIds({ 7: 1 })).toEqual(['2']);
  });

  it('drops the terms a re-upsert leaves out', async () => {
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 1 } });
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 9: 1 } });

    expect(await hitIds({ 7: 1 })).toEqual([]);
    expect(await hitIds({ 9: 1 })).toEqual(['1']);
  });

  it('keeps the sparse vector when a re-upsert brings none, as core does', async () => {
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 1 } });
    await backend.upsert('s', { id: 1, vector: [], payload: { v: 2 } });

    expect(await hitIds({ 7: 1 })).toEqual(['1']);
  });

  it('caps a sparse search at k when retired ids rank below live ones', async () => {
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 1 } });
    await backend.upsert('s', { id: 2, vector: [], sparseVector: { 7: 3 } });
    await backend.upsert('s', { id: 3, vector: [], sparseVector: { 7: 2 } });
    await backend.delete('s', 1);

    expect(await hitIds({ 7: 1 }, 1)).toEqual(['2']);
  });

  it('keeps retired sparse ids no more numerous than live ones', async () => {
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 1 } });
    await backend.upsert('s', { id: 2, vector: [], sparseVector: { 8: 1 } });
    for (let i = 0; i < 50; i += 1) {
      await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 1, [100 + i]: 1 } });
      const ids = sparseIdsOf(backend, 's');
      expect(ids.dead).toBeLessThanOrEqual(ids.byId.size);
      expect(ids.vectors.size).toBe(ids.byId.size);
    }
  });

  it("rebuilds without the retired vectors, so none can take a live point's place", async () => {
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 5 } });
    await backend.upsert('s', { id: 2, vector: [], sparseVector: { 7: 1 } });
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 8: 1 } });
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 9: 1 } });
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 10: 1 } });

    expect(sparseIdsOf(backend, 's').dead).toBe(0);
    expect(await hitIds({ 7: 1 }, 1)).toEqual(['2']);
  });

  it('never returns a replaced or deleted point, before or after the sparse index is rebuilt', async () => {
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 1 } });
    await backend.upsert('s', { id: 2, vector: [], sparseVector: { 7: 0.5 } });
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 9: 1 } });
    expect(await hitIds({ 7: 1 })).toEqual(['2']);
    expect(await hitIds({ 9: 1 })).toEqual(['1']);

    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 11: 1 } });
    await backend.delete('s', 2);

    expect(sparseIdsOf(backend, 's').dead).toBe(0);
    expect(await hitIds({ 7: 1 })).toEqual([]);
    expect(await hitIds({ 9: 1 })).toEqual([]);
    const [hit] = await backend.search('s', [], { sparseVector: { 11: 2 } });
    expect(hit).toMatchObject({ id: '1', score: 2 });
  });

  it('frees the sparse store a rebuild replaces', async () => {
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 1 } });
    await backend.upsert('s', { id: 2, vector: [], sparseVector: { 8: 1 } });
    const replaced = sparseIdsOf(backend, 's').store!;
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 9: 1 } });
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 10: 1 } });
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 11: 1 } });

    const current = sparseIdsOf(backend, 's').store!;
    expect(current).not.toBe(replaced);
    expect(replaced.free).toHaveBeenCalledTimes(1);
    expect(current.free).not.toHaveBeenCalled();
  });

  it('frees the sparse store when a rebuild leaves no live sparse vector', async () => {
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 1 } });
    const replaced = sparseIdsOf(backend, 's').store!;
    await backend.delete('s', 1);

    expect(sparseIdsOf(backend, 's').store).toBeNull();
    expect(replaced.free).toHaveBeenCalledTimes(1);
  });

  it('deleteCollection frees the vector store and the sparse store', async () => {
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 1 } });
    const vectors = storeOf(backend, 's');
    const sparse = sparseIdsOf(backend, 's').store!;
    await backend.deleteCollection('s');

    expect(vectors.free).toHaveBeenCalledTimes(1);
    expect(sparse.free).toHaveBeenCalledTimes(1);
  });

  it('close frees the vector store and the sparse store', async () => {
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 1 } });
    const vectors = storeOf(backend, 's');
    const sparse = sparseIdsOf(backend, 's').store!;
    await backend.close();

    expect(vectors.free).toHaveBeenCalledTimes(1);
    expect(sparse.free).toHaveBeenCalledTimes(1);
  });

  it('still returns k live points when dead postings outrank them', async () => {
    await backend.upsert('s', { id: 1, vector: [], sparseVector: { 7: 2 } });
    await backend.upsert('s', { id: 2, vector: [], sparseVector: { 7: 1 } });
    await backend.delete('s', 1);

    expect(await hitIds({ 7: 1 }, 1)).toEqual(['2']);
  });
});

describe('WasmBackend — createCollection applies or refuses each option (#2095)', () => {
  let backend: WasmBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = new WasmBackend();
    await backend.init();
  });

  it.each(['sq8', 'binary'] as const)('creates the store in storageMode %s', async (mode) => {
    await backend.createCollection('c', { dimension: 4, storageMode: mode });

    expect(storeOf(backend, 'c').storage_mode).toBe(mode);
  });

  it.each(['pq', 'rabitq'] as const)(
    'refuses storageMode %s, which velesdb-wasm would store as SQ8',
    async (mode) => {
      expectRefusal(
        await settle(backend.createCollection('c', { dimension: 4, storageMode: mode })),
        'storageModes'
      );
      expect(await backend.getCollection('c')).toBeNull();
    }
  );

  it.each(['metadata_only', 'graph'] as const)('refuses collectionType %s', async (type) => {
    expectRefusal(
      await settle(backend.createCollection('c', { dimension: 4, collectionType: type })),
      'collectionTypes'
    );
  });

  it.each([
    ['hnsw', { hnsw: { m: 16 } }],
    ['pqRescoreOversampling', { pqRescoreOversampling: 4 }],
    ['deferredIndexing', { deferredIndexing: { enabled: true } }],
    ['asyncIndexBuilder', { asyncIndexBuilder: { segmentCount: 2 } }],
  ] as const)('refuses %s, which it has nowhere to apply', async (_name, extra) => {
    expectRefusal(
      await settle(backend.createCollection('c', { dimension: 4, ...extra })),
      'collectionConfig'
    );
  });

  it('accepts an hnsw object that sets nothing', async () => {
    await backend.createCollection('c', { dimension: 4, hnsw: {} });

    expect(await backend.getCollection('c')).not.toBeNull();
  });
});

describe('WasmBackend — query refuses the QueryOptions it cannot apply (#2095)', () => {
  let backend: WasmBackend;
  const NEAR = 'SELECT * FROM vectors WHERE vector NEAR $v LIMIT 5';

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = new WasmBackend();
    await backend.init();
    await backend.createCollection('vectors', { dimension: 4 });
  });

  it.each([
    ['timeoutMs', { timeoutMs: 500 }],
    ['stream', { stream: true }],
  ] as const)('refuses %s', async (_name, options) => {
    expectRefusal(
      await settle(backend.query('vectors', NEAR, { v: [1, 0, 0, 0] }, options)),
      'queryOptions'
    );
  });

  it('accepts stream: false, which asks for nothing', async () => {
    const response = await backend.query('vectors', NEAR, { v: [1, 0, 0, 0] }, { stream: false });

    expect(response.results).toHaveLength(1);
  });
});
