/**
 * WASM Search Tests (#598)
 *
 * Covers `src/backends/wasm-search.ts`: wasmSearch (dense / filter /
 * sparse-only / hybrid-fusion branches), wasmSearchBatch,
 * wasmTextSearch (tuple + object result shapes), wasmHybridSearch,
 * wasmMultiQuerySearch (empty/non-empty, flattening), and wasmQuery
 * (VelesQL-over-WASM happy path + validation errors).
 *
 * Stubs the WasmContext manually instead of loading @wiscale/velesdb-wasm
 * so the tests stay pure Node and don't depend on WASM compilation.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  wasmSearch,
  wasmSearchBatch,
  wasmTextSearch,
  wasmHybridSearch,
  wasmMultiQuerySearch,
  wasmQuery,
} from '../src/backends/wasm-search';
import { NotFoundError, VelesDBError } from '../src/types';
import type {
  CollectionData,
  WasmContext,
  WasmModule,
  WasmVectorStore,
} from '../src/backends/wasm-types';

type StoreStub = Partial<Record<keyof WasmVectorStore, unknown>>;

function buildStore(overrides: StoreStub = {}): WasmVectorStore {
  return {
    search: vi.fn(() => []),
    search_with_filter: vi.fn(() => []),
    sparse_search: vi.fn(() => []),
    text_search: vi.fn(() => []),
    hybrid_search: vi.fn(() => []),
    multi_query_search: vi.fn(() => []),
    query: vi.fn(() => []),
    len: 0,
    is_empty: true,
    free: vi.fn(),
    insert: vi.fn(),
    insert_with_payload: vi.fn(),
    insert_batch: vi.fn(),
    reserve: vi.fn(),
    remove: vi.fn(),
    get: vi.fn(),
    ...overrides,
  } as unknown as WasmVectorStore;
}

function buildCtx(
  collectionName: string,
  store: WasmVectorStore,
  opts: {
    dimension?: number;
    payloads?: Map<string, Record<string, unknown>>;
    wasmModule?: Partial<WasmModule>;
  } = {}
): WasmContext {
  const data: CollectionData = {
    config: { dimension: opts.dimension ?? 2, metric: 'cosine' },
    store,
    payloads: opts.payloads ?? new Map(),
    createdAt: new Date(),
  };
  const module: WasmModule = {
    default: vi.fn(() => Promise.resolve()),
    VectorStore: (() => ({})) as unknown as WasmModule['VectorStore'],
    hybrid_search_fuse: vi.fn(() => []),
    ...opts.wasmModule,
  } as WasmModule;
  return {
    wasmModule: module,
    getCollection: (name: string) => (name === collectionName ? data : undefined),
    canonicalPayloadKeyFromResultId: (id) =>
      typeof id === 'bigint' ? id.toString() : String(id),
    canonicalPayloadKey: (id) => String(id),
    sparseVectorToArrays: (sv) => {
      const indices: number[] = [];
      const values: number[] = [];
      for (const [k, v] of Object.entries(sv)) {
        indices.push(Number(k));
        values.push(v);
      }
      return { indices, values };
    },
    toNumericId: (id) => (typeof id === 'number' ? id : Number(id) || 0),
  };
}

describe('wasmSearch — validation + dense happy path', () => {
  beforeEach(() => vi.clearAllMocks());

  it('throws NotFoundError when the collection is missing', async () => {
    const ctx = buildCtx('docs', buildStore());
    await expect(wasmSearch(ctx, 'missing', [0.1, 0.2])).rejects.toBeInstanceOf(
      NotFoundError
    );
  });

  it('throws DIMENSION_MISMATCH when query length != collection.dimension', async () => {
    const ctx = buildCtx('docs', buildStore(), { dimension: 3 });
    await expect(wasmSearch(ctx, 'docs', [0.1, 0.2])).rejects.toThrow(
      VelesDBError
    );
    await expect(wasmSearch(ctx, 'docs', [0.1, 0.2])).rejects.toThrow(
      /dimension mismatch/i
    );
  });

  it('dense-only branch maps tuples to SearchResult with payload from map', async () => {
    const payloads = new Map<string, Record<string, unknown>>([
      ['42', { title: 'x' }],
    ]);
    const store = buildStore({
      search: vi.fn(() => [[42n, 0.9]]),
    });
    const ctx = buildCtx('docs', store, { dimension: 2, payloads });

    const result = await wasmSearch(ctx, 'docs', [0.1, 0.2]);

    expect(result).toEqual([{ id: '42', score: 0.9, payload: { title: 'x' } }]);
  });

  it('dense-only branch omits payload when the map has no entry', async () => {
    const store = buildStore({ search: vi.fn(() => [[7n, 0.5]]) });
    const ctx = buildCtx('docs', store);

    const result = await wasmSearch(ctx, 'docs', [0.1, 0.2]);
    expect(result).toEqual([{ id: '7', score: 0.5 }]);
  });

  it('applies default k=10 when options.k is omitted', async () => {
    const search = vi.fn(() => []);
    const store = buildStore({ search });
    const ctx = buildCtx('docs', store);

    await wasmSearch(ctx, 'docs', new Float32Array([0.1, 0.2]));
    expect(search).toHaveBeenCalledWith(expect.any(Float32Array), 10);
  });
});

describe('wasmSearch — filter / sparse / hybrid branches', () => {
  beforeEach(() => vi.clearAllMocks());

  it('filter branch uses search_with_filter and preserves r.payload', async () => {
    const payloads = new Map<string, Record<string, unknown>>([
      ['9', { fallback: true }],
    ]);
    const store = buildStore({
      search_with_filter: vi.fn(() => [
        { id: 1n, score: 0.9, payload: { inline: true } },
        { id: 9n, score: 0.5 }, // no inline payload → falls back to map
      ]),
    });
    const ctx = buildCtx('docs', store, { payloads });

    const result = await wasmSearch(ctx, 'docs', [0.1, 0.2], {
      filter: { category: 'x' },
    });

    expect(store.search_with_filter).toHaveBeenCalled();
    expect(result[0]).toEqual({ id: '1', score: 0.9, payload: { inline: true } });
    expect(result[1]).toEqual({ id: '9', score: 0.5, payload: { fallback: true } });
  });

  it('sparse-only branch activates when dimension=0 and sparseVector provided', async () => {
    const sparse_search = vi.fn(() => [{ doc_id: 5n, score: 0.7 }]);
    const store = buildStore({ sparse_search });
    const ctx = buildCtx('docs', store, { dimension: 0 });

    const result = await wasmSearch(ctx, 'docs', [], {
      sparseVector: { 1: 0.5, 2: 0.3 },
    });

    expect(sparse_search).toHaveBeenCalled();
    expect(result).toEqual([{ id: '5', score: 0.7, payload: undefined }]);
  });

  it('hybrid fusion branch calls wasmModule.hybrid_search_fuse and slices to k', async () => {
    const fuse = vi.fn(() => [
      { doc_id: 10n, score: 0.9 },
      { doc_id: 20n, score: 0.8 },
      { doc_id: 30n, score: 0.7 },
    ]);
    const store = buildStore({
      search: vi.fn(() => [[1n, 0.9]]),
      sparse_search: vi.fn(() => [{ doc_id: 2n, score: 0.5 }]),
    });
    const ctx = buildCtx('docs', store, {
      wasmModule: { hybrid_search_fuse: fuse },
    });

    const result = await wasmSearch(ctx, 'docs', [0.1, 0.2], {
      k: 2,
      sparseVector: { 1: 0.5 },
    });

    expect(fuse).toHaveBeenCalled();
    expect(result).toHaveLength(2);
    expect(result[0]!.id).toBe('10');
  });
});

describe('wasmSearchBatch', () => {
  beforeEach(() => vi.clearAllMocks());

  it('calls wasmSearch once per search entry, preserving order', async () => {
    const store = buildStore({
      search: vi
        .fn()
        .mockReturnValueOnce([[1n, 0.1]])
        .mockReturnValueOnce([[2n, 0.2]]),
    });
    const ctx = buildCtx('docs', store);

    const result = await wasmSearchBatch(ctx, 'docs', [
      { vector: [0.1, 0.2], k: 1 },
      { vector: new Float32Array([0.3, 0.4]), k: 1, quality: 'fast' },
    ]);

    expect(result).toHaveLength(2);
    expect(result[0]![0]!.id).toBe('1');
    expect(result[1]![0]!.id).toBe('2');
  });

  it('bubbles up NotFoundError from wasmSearch', async () => {
    const ctx = buildCtx('docs', buildStore());
    await expect(
      wasmSearchBatch(ctx, 'missing', [{ vector: [0.1, 0.2] }])
    ).rejects.toBeInstanceOf(NotFoundError);
  });
});

describe('wasmTextSearch', () => {
  beforeEach(() => vi.clearAllMocks());

  it('throws NotFoundError when collection missing', async () => {
    const ctx = buildCtx('docs', buildStore());
    await expect(wasmTextSearch(ctx, 'missing', 'q')).rejects.toBeInstanceOf(
      NotFoundError
    );
  });

  it('maps tuple results and object results via mapWasmResult', async () => {
    const payloads = new Map<string, Record<string, unknown>>([
      ['1', { from: 'map' }],
    ]);
    const store = buildStore({
      text_search: vi.fn(() => [
        [1n, 0.9], // tuple shape
        { id: 2n, score: 0.5, payload: { inline: true } }, // object shape
      ]),
    });
    const ctx = buildCtx('docs', store, { payloads });

    const result = await wasmTextSearch(ctx, 'docs', 'hello', { k: 5 });

    expect(result[0]).toEqual({ id: '1', score: 0.9, payload: { from: 'map' } });
    expect(result[1]).toEqual({
      id: '2',
      score: 0.5,
      payload: { inline: true },
    });
  });
});

describe('wasmHybridSearch', () => {
  beforeEach(() => vi.clearAllMocks());

  it('throws NotFoundError when collection missing', async () => {
    const ctx = buildCtx('docs', buildStore());
    await expect(
      wasmHybridSearch(ctx, 'missing', [0.1, 0.2], 'q')
    ).rejects.toBeInstanceOf(NotFoundError);
  });

  it('forwards k and vectorWeight to hybrid_search with defaults', async () => {
    const hybrid = vi.fn(() => []);
    const store = buildStore({ hybrid_search: hybrid });
    const ctx = buildCtx('docs', store);

    await wasmHybridSearch(ctx, 'docs', [0.1, 0.2], 'q');
    expect(hybrid).toHaveBeenCalledWith(expect.any(Float32Array), 'q', 10, 0.5);

    await wasmHybridSearch(ctx, 'docs', new Float32Array([0.3, 0.4]), 'q', {
      k: 3,
      vectorWeight: 0.9,
    });
    expect(hybrid).toHaveBeenCalledWith(expect.any(Float32Array), 'q', 3, 0.9);
  });

  it('maps results with inline payload priority over map', async () => {
    const payloads = new Map<string, Record<string, unknown>>([
      ['5', { from: 'map' }],
    ]);
    const store = buildStore({
      hybrid_search: vi.fn(() => [
        { id: 5n, score: 0.8, payload: { inline: true } },
        { id: 9n, score: 0.1 },
      ]),
    });
    const ctx = buildCtx('docs', store, { payloads });

    const result = await wasmHybridSearch(ctx, 'docs', [0.1, 0.2], 'q');
    expect(result[0]!.payload).toEqual({ inline: true });
    expect(result[1]!.payload).toBeUndefined();
  });
});

describe('wasmMultiQuerySearch', () => {
  beforeEach(() => vi.clearAllMocks());

  it('throws NotFoundError when collection missing', async () => {
    const ctx = buildCtx('docs', buildStore());
    await expect(
      wasmMultiQuerySearch(ctx, 'missing', [[0.1, 0.2]])
    ).rejects.toBeInstanceOf(NotFoundError);
  });

  it('returns [] immediately when vectors list is empty', async () => {
    const multi = vi.fn(() => []);
    const store = buildStore({ multi_query_search: multi });
    const ctx = buildCtx('docs', store);

    const result = await wasmMultiQuerySearch(ctx, 'docs', []);
    expect(result).toEqual([]);
    expect(multi).not.toHaveBeenCalled();
  });

  it('flattens vectors to a single Float32Array and forwards strategy/rrfK', async () => {
    const multi = vi.fn(() => [[7n, 0.77]]);
    const store = buildStore({ multi_query_search: multi });
    const ctx = buildCtx('docs', store, { dimension: 2 });

    await wasmMultiQuerySearch(
      ctx,
      'docs',
      [new Float32Array([1, 2]), [3, 4]],
      { k: 5, fusion: 'rrf', fusionParams: { k: 77 } }
    );

    expect(multi).toHaveBeenCalledTimes(1);
    const args = multi.mock.calls[0]!;
    const flat = args[0] as Float32Array;
    expect(Array.from(flat)).toEqual([1, 2, 3, 4]);
    expect(args[1]).toBe(2); // numVectors
    expect(args[2]).toBe(5); // k
    expect(args[3]).toBe('rrf'); // strategy
    expect(args[4]).toBe(77); // rrf_k
  });

  it('applies defaults: fusion=rrf, rrfK=60, k=10', async () => {
    const multi = vi.fn(() => []);
    const store = buildStore({ multi_query_search: multi });
    const ctx = buildCtx('docs', store, { dimension: 2 });

    await wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]]);
    const args = multi.mock.calls[0]!;
    expect(args[2]).toBe(10);
    expect(args[3]).toBe('rrf');
    expect(args[4]).toBe(60);
  });
});

describe('wasmQuery', () => {
  beforeEach(() => vi.clearAllMocks());

  it('throws NotFoundError when collection missing', async () => {
    const ctx = buildCtx('docs', buildStore());
    await expect(wasmQuery(ctx, 'missing', 'q')).rejects.toBeInstanceOf(
      NotFoundError
    );
  });

  it('throws BAD_REQUEST when params.q is not a vector', async () => {
    const ctx = buildCtx('docs', buildStore());
    await expect(wasmQuery(ctx, 'docs', 'q', {})).rejects.toThrow(VelesDBError);
    await expect(wasmQuery(ctx, 'docs', 'q', {})).rejects.toThrow(
      /params\.q/
    );
  });

  it('accepts Float32Array or number[] for params.q', async () => {
    const query = vi.fn(() => [{ id: 1 }]);
    const store = buildStore({ query });
    const ctx = buildCtx('docs', store);

    await wasmQuery(ctx, 'docs', 'q', { q: [0.1, 0.2] });
    expect(query).toHaveBeenCalledWith(expect.any(Float32Array), 10);

    await wasmQuery(ctx, 'docs', 'q', {
      q: new Float32Array([0.1, 0.2]),
      k: 5,
    });
    expect(query).toHaveBeenLastCalledWith(expect.any(Float32Array), 5);
  });

  it('clamps invalid k to default 10', async () => {
    const query = vi.fn(() => []);
    const store = buildStore({ query });
    const ctx = buildCtx('docs', store);

    await wasmQuery(ctx, 'docs', 'q', { q: [0.1, 0.2], k: -5 });
    expect(query).toHaveBeenLastCalledWith(expect.any(Float32Array), 10);

    await wasmQuery(ctx, 'docs', 'q', { q: [0.1, 0.2], k: 3.5 });
    expect(query).toHaveBeenLastCalledWith(expect.any(Float32Array), 10);

    await wasmQuery(ctx, 'docs', 'q', { q: [0.1, 0.2], k: 0 });
    expect(query).toHaveBeenLastCalledWith(expect.any(Float32Array), 10);
  });

  it('returns raw results with wasm-query stats', async () => {
    const raw = [{ id: 1, title: 'a' }, { id: 2, title: 'b' }];
    const store = buildStore({ query: vi.fn(() => raw) });
    const ctx = buildCtx('docs', store);

    const out = await wasmQuery(ctx, 'docs', 'q', { q: [0.1, 0.2] });
    expect(out.results).toBe(raw);
    expect(out.stats.strategy).toBe('wasm-query');
    expect(out.stats.scannedNodes).toBe(2);
    expect(out.stats.executionTimeMs).toBe(0);
  });
});
