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
import type { SearchQuality } from '../src/types';
import { newSparseIds, sparseHits } from '../src/backends/wasm-sparse';
import type {
  CollectionData,
  WasmContext,
  WasmModule,
  WasmVectorStore,
} from '../src/backends/wasm-types';

type StoreStub = Partial<Record<keyof WasmVectorStore, unknown>>;

function buildStore(overrides: StoreStub = {}): WasmVectorStore {
  const stub: StoreStub = {
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
  };
  const store = stub as unknown as WasmVectorStore;
  // The binding parses the preset, then runs the same brute-force search
  // (`search_with_quality` in velesdb-wasm's `vector_store.rs`), so a test
  // that stubs `search` covers both unless it stubs this one too.
  stub.search_with_quality ??= vi.fn((q: Float32Array, k: number) =>
    store.search(q, k)
  );
  return store;
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
    sparseIds: newSparseIds(),
    createdAt: new Date(),
  };
  const module: WasmModule = {
    default: vi.fn(() => Promise.resolve()),
    // `new_metadata_only` is the empty store `requireParsableQuality` asks
    // the binding to parse a preset against; here it parses nothing, so a
    // test that needs the real refusal stubs it.
    VectorStore: {
      new_metadata_only: () => buildStore(),
    } as unknown as WasmModule['VectorStore'],
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

/**
 * What `@wiscale/velesdb-wasm` 6.0.0 throws for an unparseable preset —
 * a BARE STRING, not an `Error`. wasm-bindgen raises a `Result::Err(String)`
 * by throwing the string itself, so a fake that throws `new Error(…)` is
 * kinder than the binding and lets an `e.message` read pass that would
 * return `undefined` in production. Probed, not assumed:
 * `probe.search_with_quality(new Float32Array(0), 0, 'nonsense')` reports
 * `typeof e === 'string'`, `e instanceof Error === false`.
 */
const REFUSAL =
  "Unknown search quality: 'nonsense'. Valid: fast, balanced, accurate, " +
  'perfect, autotune, custom:<ef>, adaptive:<min_ef>:<max_ef>';

/** A module whose `new_metadata_only()` hands back `probe`. */
function moduleWithProbe(probe: WasmVectorStore): Partial<WasmModule> {
  return {
    VectorStore: {
      new_metadata_only: () => probe,
    } as unknown as WasmModule['VectorStore'],
  };
}

describe('wasmSearch — quality reaches the binding, and its refusal reaches back (#2282)', () => {
  beforeEach(() => vi.clearAllMocks());

  it('runs a dense search under the preset the caller named', async () => {
    const search_with_quality = vi.fn(() => [[1n, 0.9]]);
    const store = buildStore({ search_with_quality });
    const ctx = buildCtx('docs', store);

    await wasmSearch(ctx, 'docs', [0.1, 0.2], { k: 3, quality: 'accurate' });

    expect(search_with_quality).toHaveBeenCalledWith(
      expect.any(Float32Array),
      3,
      'accurate'
    );
  });

  it("names 'balanced' when the caller names no preset", async () => {
    const search_with_quality = vi.fn(() => []);
    const store = buildStore({ search_with_quality });
    const ctx = buildCtx('docs', store);

    await wasmSearch(ctx, 'docs', [0.1, 0.2]);

    expect(search_with_quality).toHaveBeenCalledWith(
      expect.any(Float32Array),
      10,
      'balanced'
    );
  });

  it("fuses a hybrid search's dense leg under the caller's preset", async () => {
    const search_with_quality = vi.fn(() => [[1n, 0.9]]);
    const store = buildStore({
      search_with_quality,
      sparse_search: vi.fn(() => [{ doc_id: 2n, score: 0.5 }]),
    });
    const ctx = buildCtx('docs', store, {
      wasmModule: { hybrid_search_fuse: vi.fn(() => []) },
    });
    ctx.getCollection('docs')!.sparseIds.store = store;

    await wasmSearch(ctx, 'docs', [0.1, 0.2], {
      k: 2,
      quality: 'fast',
      sparseVector: { 1: 0.5 },
    });

    expect(search_with_quality).toHaveBeenCalledWith(
      expect.any(Float32Array),
      2,
      'fast'
    );
  });

  // Every path, including the two with no quality-taking binding method and
  // the one that runs no search at all: none may accept a preset the binding
  // refuses. The fake only refuses; the grammar stays in velesdb-wasm.
  it.each([
    ['dense', {}],
    ['filtered', { filter: { tenant: 'a' } }],
    ['sparse-only', { sparseVector: { 1: 0.5 } }],
    ['k <= 0', { k: 0 }],
  ] as const)(
    'a %s search refuses a preset the binding cannot parse',
    async (_path, options) => {
      const refuse = vi.fn(() => {
        throw REFUSAL;
      });
      const probe = buildStore({ search_with_quality: refuse });
      const ctx = buildCtx('docs', buildStore(), {
        dimension: 0,
        wasmModule: moduleWithProbe(probe),
      });

      // A `VelesDBError`, not the bare string: the PR's contract is that a
      // caller can narrow on the refusal, and `String(thrown)` is all the
      // binding gives the SDK to put in it. The CODE is part of that
      // contract — README, CHANGELOG and `capabilities.ts` all name
      // `BAD_REQUEST` for an unparseable preset — so it is asserted, not
      // merely the class: `toBeInstanceOf(VelesDBError)` alone survives
      // `'BAD_REQUEST' → 'INTERNAL'`.
      const outcome = await settle(
        wasmSearch(ctx, 'docs', [], { ...options, quality: 'nonsense' })
      );
      expectBadRequest(outcome, /Unknown search quality/);

      expect(refuse).toHaveBeenCalledWith(expect.any(Float32Array), 0, 'nonsense');
      expect(probe.free).toHaveBeenCalled();
    }
  );

  it('asks the binding nothing when the caller names no preset', async () => {
    const refuse = vi.fn(() => {
      throw 'the probe must not run';
    });
    const probe = buildStore({ search_with_quality: refuse });
    const ctx = buildCtx('docs', buildStore(), {
      wasmModule: moduleWithProbe(probe),
    });

    await wasmSearch(ctx, 'docs', [0.1, 0.2]);

    expect(refuse).not.toHaveBeenCalled();
  });
});

describe('wasmSearchBatch — an unparseable preset stops the batch before it starts (#2282)', () => {
  beforeEach(() => vi.clearAllMocks());

  it('refuses in the pre-loop, so no entry runs half a batch', async () => {
    // The probe is the binding's parser: it refuses `'nonsense'` and accepts
    // everything else, so entry 1's own preset is not what stops the batch.
    const parse = vi.fn((_q: Float32Array, _k: number, quality: string) => {
      if (quality === 'nonsense') {
        throw REFUSAL;
      }
      return [];
    });
    const search_with_quality = vi.fn(() => [[1n, 0.9]]);
    const ctx = buildCtx('docs', buildStore({ search_with_quality }), {
      wasmModule: moduleWithProbe(buildStore({ search_with_quality: parse })),
    });

    const outcome = await settle(
      wasmSearchBatch(ctx, 'docs', [
        { vector: [0.1, 0.2], quality: 'fast' },
        { vector: [0.3, 0.4], quality: 'nonsense' as SearchQuality },
      ])
    );
    expectBadRequest(outcome, /Unknown search quality/);

    expect(search_with_quality).not.toHaveBeenCalled();
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
    // Sparse id 5 is point 5's live sparse vector, in the collection's
    // sparse store (see wasm-sparse.ts).
    const sparseIds = ctx.getCollection('docs')!.sparseIds;
    sparseIds.store = buildStore({ sparse_search });
    sparseIds.byId.set(5n, 5);

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

  it('refuses an empty vectors list, as core does', async () => {
    const multi = vi.fn(() => []);
    const store = buildStore({ multi_query_search: multi });
    const ctx = buildCtx('docs', store);

    const outcome = await settle(wasmMultiQuerySearch(ctx, 'docs', []));
    expect(outcome).toBeInstanceOf(VelesDBError);
    expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
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

  const PURE_NEAR = 'SELECT * FROM docs WHERE vector NEAR $q';

  it('throws BAD_REQUEST when params.q is not a vector', async () => {
    const ctx = buildCtx('docs', buildStore());
    // The title said BAD_REQUEST; only the class and the message were
    // asserted, so `'BAD_REQUEST' → anything` survived. The code is asserted
    // here, as everywhere this SDK promises one in prose.
    expectBadRequest(await settle(wasmQuery(ctx, 'docs', PURE_NEAR, {})), /params\.q/);
  });

  it('accepts Float32Array or number[] for params.q', async () => {
    const query = vi.fn(() => [{ id: 1 }]);
    const store = buildStore({ query });
    const ctx = buildCtx('docs', store);

    await wasmQuery(ctx, 'docs', PURE_NEAR, { q: [0.1, 0.2] });
    expect(query).toHaveBeenCalledWith(expect.any(Float32Array), 10);

    await wasmQuery(ctx, 'docs', PURE_NEAR, { q: new Float32Array([0.1, 0.2]) });
    expect(query).toHaveBeenLastCalledWith(expect.any(Float32Array), 10);
  });

  it("ignores params.k, as REST does: without LIMIT, core's default of 10 applies", async () => {
    const query = vi.fn(() => []);
    const store = buildStore({ query });
    const ctx = buildCtx('docs', store);

    await wasmQuery(ctx, 'docs', PURE_NEAR, { q: [0.1, 0.2], k: 3 });
    expect(query).toHaveBeenLastCalledWith(expect.any(Float32Array), 10);

    await wasmQuery(ctx, 'docs', PURE_NEAR, { q: [0.1, 0.2], k: -5 });
    expect(query).toHaveBeenLastCalledWith(expect.any(Float32Array), 10);

    await wasmQuery(ctx, 'docs', PURE_NEAR, { q: [0.1, 0.2], k: 3.5 });
    expect(query).toHaveBeenLastCalledWith(expect.any(Float32Array), 10);

    await wasmQuery(ctx, 'docs', PURE_NEAR, { q: [0.1, 0.2], k: 0 });
    expect(query).toHaveBeenLastCalledWith(expect.any(Float32Array), 10);
  });

  it('returns raw results with wasm-query stats', async () => {
    const raw = [{ id: 1, title: 'a' }, { id: 2, title: 'b' }];
    const store = buildStore({ query: vi.fn(() => raw) });
    const ctx = buildCtx('docs', store);

    const out = await wasmQuery(ctx, 'docs', PURE_NEAR, { q: [0.1, 0.2] });
    expect(out.results).toBe(raw);
    expect(out.stats.strategy).toBe('wasm-query');
    expect(out.stats.scannedNodes).toBe(2);
    expect(out.stats.executionTimeMs).toBe(0);
  });
});

describe('wasmQuery — VelesQL faithfulness guard', () => {
  beforeEach(() => vi.clearAllMocks());

  it('never silently drops a WHERE filter: rejects and does not run raw k-NN', async () => {
    const query = vi.fn(() => [{ id: 1 }, { id: 2 }]);
    const store = buildStore({ query });
    const ctx = buildCtx('docs', store);

    const err: unknown = await wasmQuery(
      ctx,
      'docs',
      "SELECT * FROM docs WHERE category='tech' AND vector NEAR $q",
      { q: [0.1, 0.2] }
    ).catch((e: unknown) => e);

    expect(err).toBeInstanceOf(VelesDBError);
    expect((err as VelesDBError).code).toBe('NOT_SUPPORTED');
    expect((err as VelesDBError).message).toMatch(/REST/);
    expect(query).not.toHaveBeenCalled();
  });

  it.each([
    ['JOIN', 'SELECT * FROM docs JOIN other ON docs.id = other.id WHERE vector NEAR $q'],
    ['GROUP BY', 'SELECT category, COUNT(*) FROM docs GROUP BY category'],
    ['MATCH', 'SELECT * FROM docs MATCH (a)-[:LINKS]->(b) WHERE vector NEAR $q'],
    ['ORDER BY', 'SELECT * FROM docs WHERE vector NEAR $q ORDER BY id'],
    ['UNION', 'SELECT * FROM docs UNION SELECT * FROM archived'],
    ['FUSION', "SELECT * FROM docs LIMIT 20 USING FUSION(strategy = 'rrf', k = 60)"],
    ['inline NEAR literal', 'SELECT * FROM docs WHERE vector NEAR [0.1, 0.2]'],
    // The pest grammar only admits the literal `vector` keyword left of NEAR
    // (`vector_search = { ^"vector" ~ ^"NEAR" ~ vector_value }`); a column
    // identifier is a guaranteed parse error on velesdb-server, so accepting
    // it in WASM would recreate the WASM-works/REST-breaks divergence.
    ['column identifier NEAR', 'SELECT * FROM docs WHERE embedding NEAR $q LIMIT 5'],
  ])('rejects %s queries with NOT_SUPPORTED instead of raw k-NN', async (_label, sql) => {
    const query = vi.fn(() => []);
    const store = buildStore({ query });
    const ctx = buildCtx('docs', store);

    const err: unknown = await wasmQuery(ctx, 'docs', sql, { q: [0.1, 0.2] }).catch(
      (e: unknown) => e
    );

    expect(err).toBeInstanceOf(VelesDBError);
    expect((err as VelesDBError).code).toBe('NOT_SUPPORTED');
    expect(query).not.toHaveBeenCalled();
  });

  it('rejects a query whose FROM targets a different collection', async () => {
    const query = vi.fn(() => []);
    const store = buildStore({ query });
    const ctx = buildCtx('docs', store);

    const err: unknown = await wasmQuery(
      ctx,
      'docs',
      'SELECT * FROM other WHERE vector NEAR $q',
      { q: [0.1, 0.2] }
    ).catch((e: unknown) => e);

    expect(err).toBeInstanceOf(VelesDBError);
    expect((err as VelesDBError).code).toBe('BAD_REQUEST');
    expect(query).not.toHaveBeenCalled();
  });

  it('executes a pure NEAR query, LIMIT driving k', async () => {
    const query = vi.fn(() => []);
    const store = buildStore({ query });
    const ctx = buildCtx('docs', store);

    await wasmQuery(ctx, 'docs', 'SELECT * FROM docs WHERE vector NEAR $q LIMIT 3', {
      q: [0.1, 0.2],
    });
    expect(query).toHaveBeenLastCalledWith(expect.any(Float32Array), 3);
  });

  it('reads the embedding from the parameter named in the query', async () => {
    const query = vi.fn(() => []);
    const store = buildStore({ query });
    const ctx = buildCtx('docs', store);

    await wasmQuery(ctx, 'docs', 'SELECT * FROM docs WHERE vector NEAR $vec', {
      vec: [0.1, 0.2],
    });
    expect(query).toHaveBeenLastCalledWith(expect.any(Float32Array), 10);
  });

  it('is case-insensitive and tolerates a trailing semicolon', async () => {
    const query = vi.fn(() => []);
    const store = buildStore({ query });
    const ctx = buildCtx('docs', store);

    await wasmQuery(ctx, 'docs', 'select * from docs where vector near $q limit 2;', {
      q: [0.1, 0.2],
    });
    expect(query).toHaveBeenLastCalledWith(expect.any(Float32Array), 2);
  });
});

// ---------------------------------------------------------------------------
// #2095 — the WASM backend refuses what it cannot honour instead of dropping
// it. Every refusal is NOT_SUPPORTED and names the backend and the capability
// a caller can read beforehand from `db.capabilities()`.
// ---------------------------------------------------------------------------

/** Settle `promise`, returning its value or what it rejected with. */
async function settle<T>(promise: Promise<T>): Promise<unknown> {
  return promise.then(
    (value) => value,
    (error: unknown) => error
  );
}

/**
 * A refusal the SDK issues on its own behalf: a `VelesDBError` whose `.code`
 * is `BAD_REQUEST`. The code is the contract README, CHANGELOG and
 * `capabilities.ts` state in prose, so it is asserted rather than the class
 * alone — `toBeInstanceOf(VelesDBError)` by itself survives any rewrite of
 * the code literal.
 */
function expectBadRequest(outcome: unknown, message: RegExp): void {
  expect(outcome).toBeInstanceOf(VelesDBError);
  const err = outcome as VelesDBError;
  expect(err.code).toBe('BAD_REQUEST');
  expect(err.message).toMatch(message);
}

function expectRefusal(outcome: unknown, capability: string): void {
  expect(outcome).toBeInstanceOf(VelesDBError);
  const err = outcome as VelesDBError;
  expect(err.code).toBe('NOT_SUPPORTED');
  expect(err.message).toMatch(/WASM backend/);
  expect(err.message).toContain(capability);
}

const TENANT_FILTER = {
  condition: { type: 'eq', field: 'tenant', value: 'mine' },
};

describe('WASM search — a filter is refused, never dropped (#2095)', () => {
  beforeEach(() => vi.clearAllMocks());

  it('textSearch refuses a filter instead of returning rows it excludes', async () => {
    // velesdb-wasm's text_search(query, k, field?) has no filter slot: its
    // third argument names a payload field. Dropping the filter returns this
    // row, which the caller's filter excludes.
    const text_search = vi.fn(() => [{ id: 2n, payload: { tenant: 'other' } }]);
    const ctx = buildCtx('docs', buildStore({ text_search }));

    const outcome = await settle(
      wasmTextSearch(ctx, 'docs', 'hello', { filter: TENANT_FILTER })
    );

    expectRefusal(outcome, 'filteredSearch');
    expect(text_search).not.toHaveBeenCalled();
  });

  it('hybridSearch refuses a filter', async () => {
    const hybrid_search = vi.fn(() => [
      { id: 2n, score: 0.9, payload: { tenant: 'other' } },
    ]);
    const ctx = buildCtx('docs', buildStore({ hybrid_search }));

    const outcome = await settle(
      wasmHybridSearch(ctx, 'docs', [0.1, 0.2], 'hello', { filter: TENANT_FILTER })
    );

    expectRefusal(outcome, 'filteredSearch');
    expect(hybrid_search).not.toHaveBeenCalled();
  });

  it('multiQuerySearch refuses a filter', async () => {
    const multi_query_search = vi.fn(() => [[2n, 0.9]]);
    const ctx = buildCtx('docs', buildStore({ multi_query_search }));

    const outcome = await settle(
      wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], { filter: TENANT_FILTER })
    );

    expectRefusal(outcome, 'filteredSearch');
    expect(multi_query_search).not.toHaveBeenCalled();
  });

  it('search refuses a filter combined with a sparse vector', async () => {
    const sparse_search = vi.fn(() => [{ doc_id: 2n, score: 0.9 }]);
    const ctx = buildCtx('docs', buildStore({ sparse_search }));

    const outcome = await settle(
      wasmSearch(ctx, 'docs', [0.1, 0.2], {
        sparseVector: { 1: 0.5 },
        filter: TENANT_FILTER,
      })
    );

    expectRefusal(outcome, 'filteredSearch');
    expect(sparse_search).not.toHaveBeenCalled();
  });
});

describe('WASM search — options it has no way to honour are refused (#2095)', () => {
  beforeEach(() => vi.clearAllMocks());

  it('search refuses sparseIndexName: a WASM collection has one sparse index', async () => {
    const sparse_search = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ sparse_search }));

    const outcome = await settle(
      wasmSearch(ctx, 'docs', [0.1, 0.2], {
        sparseVector: { 1: 0.5 },
        sparseIndexName: 'splade_v2',
      })
    );

    expectRefusal(outcome, 'namedSparseIndexes');
    expect(sparse_search).not.toHaveBeenCalled();
  });

  it('search refuses includeVectors: true, since its results carry no vector', async () => {
    const search = vi.fn(() => [[1n, 0.9]]);
    const ctx = buildCtx('docs', buildStore({ search }));

    const outcome = await settle(
      wasmSearch(ctx, 'docs', [0.1, 0.2], { includeVectors: true })
    );

    expectRefusal(outcome, 'includeVectors');
  });

  it('search accepts includeVectors: false, which asks for nothing', async () => {
    const search = vi.fn(() => [[1n, 0.9]]);
    const ctx = buildCtx('docs', buildStore({ search }));

    const rows = await wasmSearch(ctx, 'docs', [0.1, 0.2], { includeVectors: false });

    expect(rows).toEqual([{ id: '1', score: 0.9 }]);
  });
});

describe('wasmMultiQuerySearch — fusionParams reach the binding or are refused (#2095)', () => {
  beforeEach(() => vi.clearAllMocks());

  it("passes avgWeight/maxWeight/hitWeight as the binding's weights argument", async () => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    await wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], {
      fusion: 'weighted',
      fusionParams: { avgWeight: 0.5, maxWeight: 0.25, hitWeight: 0.25 },
    });

    const weights = multi.mock.calls[0]![5];
    expect(weights).toBeInstanceOf(Float32Array);
    expect(Array.from(weights as Float32Array)).toEqual([0.5, 0.25, 0.25]);
  });

  it("passes no weights when none is given, so the binding applies core's defaults", async () => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    await wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], { fusion: 'weighted' });

    expect(multi.mock.calls[0]![5]).toBeNull();
  });

  it.each(['denseWeight', 'sparseWeight'] as const)(
    'refuses fusionParams.%s: WASM relative_score weighs its branches equally',
    async (name) => {
      const multi = vi.fn(() => []);
      const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

      const outcome = await settle(
        wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], {
          fusion: 'relative_score',
          fusionParams: { [name]: 0.7 },
        })
      );

      expectRefusal(outcome, 'multiQueryFusionParams');
      expect(multi).not.toHaveBeenCalled();
    }
  );

  it('refuses a partial weighted triple rather than inventing the missing weights', async () => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    const outcome = await settle(
      wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], {
        fusion: 'weighted',
        fusionParams: { avgWeight: 0.5 },
      })
    );

    expect(outcome).toBeInstanceOf(VelesDBError);
    expect((outcome as VelesDBError).code).toBe('NOT_SUPPORTED');
    expect((outcome as VelesDBError).message).toMatch(/avgWeight, maxWeight and hitWeight/);
    expect((outcome as VelesDBError).message).toContain('multiQueryFusionParams');
    expect(multi).not.toHaveBeenCalled();
  });
});

describe('wasmMultiQuerySearch — a weighted triple core would reject is BAD_REQUEST (#2095)', () => {
  beforeEach(() => vi.clearAllMocks());

  it.each([
    ['does not sum to 1.0', { avgWeight: 0.5, maxWeight: 0.5, hitWeight: 0.5 }],
    ['sums to 1.0 past the 0.001 tolerance', { avgWeight: 0.6, maxWeight: 0.3, hitWeight: 0.102 }],
    ['has a negative weight', { avgWeight: 1.5, maxWeight: -0.25, hitWeight: -0.25 }],
    ['has a non-finite weight', { avgWeight: Number.NaN, maxWeight: 0.5, hitWeight: 0.5 }],
  ])('refuses a triple that %s, before the binding sees it', async (_label, fusionParams) => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    const outcome = await settle(
      wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], { fusion: 'weighted', fusionParams })
    );

    expect(outcome).toBeInstanceOf(VelesDBError);
    expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
    expect(multi).not.toHaveBeenCalled();
  });

  it("passes a triple within core's 0.001 tolerance of 1.0", async () => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    await wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], {
      fusion: 'weighted',
      fusionParams: { avgWeight: 0.6, maxWeight: 0.3, hitWeight: 0.1005 },
    });

    expect(multi).toHaveBeenCalledTimes(1);
  });
});

describe("wasmSearchBatch — each entry's filter reaches the binding (#2095)", () => {
  beforeEach(() => vi.clearAllMocks());

  it('filters every entry through search_with_filter', async () => {
    const search_with_filter = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ search_with_filter }));

    await wasmSearchBatch(ctx, 'docs', [{ vector: [0.1, 0.2], filter: TENANT_FILTER }]);

    expect(search_with_filter).toHaveBeenCalledWith(expect.any(Float32Array), 10, TENANT_FILTER);
  });
});

describe('wasmMultiQuerySearch — the weighted triple is checked in f32, as core checks it (#2095)', () => {
  beforeEach(() => vi.clearAllMocks());

  it.each([
    ['[0.5, 0.5, 0.001]', { avgWeight: 0.5, maxWeight: 0.5, hitWeight: 0.001 }],
    ['[0.6, 0.3, 0.101]', { avgWeight: 0.6, maxWeight: 0.3, hitWeight: 0.101 }],
  ])('refuses %s: its f32 sum is 1.0010000467, past the tolerance', async (_label, fusionParams) => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    const outcome = await settle(
      wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], { fusion: 'weighted', fusionParams })
    );

    expect(outcome).toBeInstanceOf(VelesDBError);
    expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
    expect(multi).not.toHaveBeenCalled();
  });

  it.each([
    ['[0.3, 0.3, 0.399]', { avgWeight: 0.3, maxWeight: 0.3, hitWeight: 0.399 }],
    ['[0.25, 0.25, 0.499]', { avgWeight: 0.25, maxWeight: 0.25, hitWeight: 0.499 }],
  ])('passes %s, which core accepts in f32 though f64 would not', async (_label, fusionParams) => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    await wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], { fusion: 'weighted', fusionParams });

    expect(multi).toHaveBeenCalledTimes(1);
  });
});

describe('wasmMultiQuerySearch — the weighted triple matters only under `weighted`, as in core (#2095)', () => {
  beforeEach(() => vi.clearAllMocks());

  it.each(['rrf', 'average', 'maximum', 'relative_score'] as const)(
    '%s passes a triple `weighted` would reject, since it never reads the weights',
    async (fusion) => {
      const multi = vi.fn(() => []);
      const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

      await wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], {
        fusion,
        fusionParams: { avgWeight: 0.5, maxWeight: 0.5, hitWeight: 0.5 },
      });

      expect(multi).toHaveBeenCalledTimes(1);
      expect(multi.mock.calls[0]![5]).toBeNull();
    }
  );

  it.each(['rrf', 'average', 'maximum', 'relative_score'] as const)(
    '%s accepts a partial triple, since it never reads the weights',
    async (fusion) => {
      const multi = vi.fn(() => []);
      const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

      await wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], {
        fusion,
        fusionParams: { avgWeight: 0.5 },
      });

      expect(multi).toHaveBeenCalledTimes(1);
      expect(multi.mock.calls[0]![5]).toBeNull();
    }
  );
});

describe('WASM search — k = 0 returns nothing, before any binding call (#2095)', () => {
  beforeEach(() => vi.clearAllMocks());

  /** A collection whose binding would return a hit on every path, with one retired sparse id. */
  function collectionWithHits(dimension: number) {
    const store = buildStore({
      search: vi.fn(() => [[1n, 0.9]]),
      search_with_filter: vi.fn(() => [{ id: 1n, score: 0.9 }]),
      text_search: vi.fn(() => [{ id: 1n, payload: {} }]),
      hybrid_search: vi.fn(() => [{ id: 1n, score: 0.9 }]),
      multi_query_search: vi.fn(() => [[1n, 0.9]]),
      query: vi.fn(() => [{ id: 1 }]),
    });
    const fuse = vi.fn(() => [{ doc_id: 1n, score: 0.9 }]);
    const ctx = buildCtx('docs', store, { dimension, wasmModule: { hybrid_search_fuse: fuse } });
    const sparseIds = ctx.getCollection('docs')!.sparseIds;
    const sparseStore = buildStore({ sparse_search: vi.fn(() => [{ doc_id: 1n, score: 1 }]) });
    sparseIds.store = sparseStore;
    sparseIds.byId.set(1n, 1);
    sparseIds.dead = 1;
    const bindingCalls = () =>
      [store, sparseStore]
        .flatMap((s) => Object.values(s))
        .filter((f): f is ReturnType<typeof vi.fn> => typeof f === 'function' && 'mock' in f)
        .reduce((total, f) => total + f.mock.calls.length, fuse.mock.calls.length);
    return { ctx, bindingCalls };
  }

  it.each([0])('every search path returns [] for k = %i and never calls the binding', async (k) => {
    const dense = collectionWithHits(2);
    const sparseOnly = collectionWithHits(0);

    const results = [
      await wasmSearch(dense.ctx, 'docs', [0.1, 0.2], { k }),
      await wasmSearch(dense.ctx, 'docs', [0.1, 0.2], { k, filter: TENANT_FILTER }),
      await wasmSearch(dense.ctx, 'docs', [0.1, 0.2], { k, sparseVector: { 7: 1 } }),
      await wasmSearch(sparseOnly.ctx, 'docs', [], { k, sparseVector: { 7: 1 } }),
      await wasmSearchBatch(dense.ctx, 'docs', [{ vector: [0.1, 0.2], k }]),
      await wasmTextSearch(dense.ctx, 'docs', 'q', { k }),
      await wasmHybridSearch(dense.ctx, 'docs', [0.1, 0.2], 'q', { k }),
      await wasmMultiQuerySearch(dense.ctx, 'docs', [[0.1, 0.2]], { k }),
    ];

    expect(results).toEqual([[], [], [], [], [[]], [], [], []]);
    expect(dense.bindingCalls() + sparseOnly.bindingCalls()).toBe(0);
  });

  it('query with LIMIT 0 returns no rows and never calls the binding', async () => {
    const { ctx, bindingCalls } = collectionWithHits(2);

    const response = await wasmQuery(ctx, 'docs', 'SELECT * FROM docs WHERE vector NEAR $v LIMIT 0', {
      v: [0.1, 0.2],
    });

    expect(response.results).toEqual([]);
    expect(bindingCalls()).toBe(0);
  });
});

describe('sparseHits — a k that is not a positive integer fetches nothing (#2095)', () => {
  it.each([0, -1, 0.5])('returns [] for k = %s without calling the sparse store', (k) => {
    const sparse_search = vi.fn(() => [{ doc_id: 1n, score: 1 }]);
    const ids = newSparseIds();
    ids.store = buildStore({ sparse_search });
    ids.byId.set(1n, 1);
    ids.dead = 1;

    expect(sparseHits(ids, [7], [1], k)).toEqual([]);
    expect(sparse_search).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// #2095 — one validation of a search's inputs, before any early return, as
// core validates them (`validated_hybrid_params`, `validate_multi_query_inputs`).
// ---------------------------------------------------------------------------

/** How many times the binding was called through `store`. */
function bindingCallCount(store: WasmVectorStore): number {
  return Object.values(store)
    .filter((f): f is ReturnType<typeof vi.fn> => typeof f === 'function' && 'mock' in f)
    .reduce((total, f) => total + f.mock.calls.length, 0);
}

describe("WASM search — a search's inputs are validated before any early return (#2095)", () => {
  beforeEach(() => vi.clearAllMocks());

  it.each([
    ['search with k = 0', (ctx: WasmContext) => wasmSearch(ctx, 'docs', [0.1], { k: 0 })],
    ['searchBatch with k = 0', (ctx: WasmContext) => wasmSearchBatch(ctx, 'docs', [{ vector: [0.1], k: 0 }])],
    ['hybridSearch with k = 0', (ctx: WasmContext) => wasmHybridSearch(ctx, 'docs', [0.1], 'q', { k: 0 })],
    ['multiQuerySearch with k = 0', (ctx: WasmContext) => wasmMultiQuerySearch(ctx, 'docs', [[0.1]], { k: 0 })],
    [
      'query with LIMIT 0',
      (ctx: WasmContext) =>
        wasmQuery(ctx, 'docs', 'SELECT * FROM docs WHERE vector NEAR $v LIMIT 0', { v: [0.1] }),
    ],
  ])('%s still refuses a vector of the wrong dimension', async (_label, call) => {
    const store = buildStore();
    const ctx = buildCtx('docs', store, { dimension: 2 });

    const outcome = await settle(call(ctx));

    expect(outcome).toBeInstanceOf(VelesDBError);
    expect((outcome as VelesDBError).code).toBe('DIMENSION_MISMATCH');
    expect(bindingCallCount(store)).toBe(0);
  });

  it.each([
    ['a short vector', [[0.1, 0.2], [0.3]]],
    ['a long vector', [[0.1, 0.2], [0.3, 0.4, 0.5]]],
  ])('multiQuerySearch refuses %s rather than pad or overflow it', async (_label, vectors) => {
    const store = buildStore();
    const ctx = buildCtx('docs', store, { dimension: 2 });

    const outcome = await settle(wasmMultiQuerySearch(ctx, 'docs', vectors as number[][]));

    expect(outcome).toBeInstanceOf(VelesDBError);
    expect((outcome as VelesDBError).code).toBe('DIMENSION_MISMATCH');
    expect(bindingCallCount(store)).toBe(0);
  });

  it('searchBatch validates every entry before it runs any', async () => {
    const store = buildStore({ search: vi.fn(() => [[1n, 0.9]]) });
    const ctx = buildCtx('docs', store, { dimension: 2 });

    const outcome = await settle(
      wasmSearchBatch(ctx, 'docs', [{ vector: [0.1, 0.2] }, { vector: [0.1] }])
    );

    expect((outcome as VelesDBError).code).toBe('DIMENSION_MISMATCH');
    expect(bindingCallCount(store)).toBe(0);
  });

  it.each([0.5, 1.5, Number.NaN, -1])(
    "every search path refuses k = %s: core's k is an unsigned integer",
    async (k) => {
      const denseStore = buildStore({ search: vi.fn(() => [[1n, 0.9]]) });
      const dense = buildCtx('docs', denseStore, { dimension: 2 });
      const sparseOnly = buildCtx('docs', buildStore(), { dimension: 0 });
      const sparseIds = sparseOnly.getCollection('docs')!.sparseIds;
      const sparse_search = vi.fn(() => [{ doc_id: 1n, score: 1 }]);
      sparseIds.store = buildStore({ sparse_search });
      sparseIds.byId.set(1n, 1);

      const calls = [
        () => wasmSearch(dense, 'docs', [0.1, 0.2], { k }),
        () => wasmSearch(sparseOnly, 'docs', [], { k, sparseVector: { 7: 1 } }),
        () => wasmSearchBatch(dense, 'docs', [{ vector: [0.1, 0.2], k }]),
        () => wasmTextSearch(dense, 'docs', 'q', { k }),
        () => wasmHybridSearch(dense, 'docs', [0.1, 0.2], 'q', { k }),
        () => wasmMultiQuerySearch(dense, 'docs', [[0.1, 0.2]], { k }),
      ];
      for (const call of calls) {
        const outcome = await settle(call());
        expect(outcome).toBeInstanceOf(VelesDBError);
        expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
      }
      expect(bindingCallCount(denseStore)).toBe(0);
      expect(sparse_search).not.toHaveBeenCalled();
    }
  );

  it("query reads no k from its params, as REST does: the rows are LIMIT's, or core's default", async () => {
    const query = vi.fn(() => [{ id: 1 }, { id: 2 }]);
    const ctx = buildCtx('docs', buildStore({ query }), { dimension: 2 });
    const NEAR = 'SELECT * FROM docs WHERE vector NEAR $v';

    await wasmQuery(ctx, 'docs', NEAR, { v: [0.1, 0.2] });
    await wasmQuery(ctx, 'docs', NEAR, { v: [0.1, 0.2], k: 1 });
    await wasmQuery(ctx, 'docs', NEAR, { v: [0.1, 0.2], k: 0 });

    const limits = query.mock.calls.map((call) => (call as unknown[])[1]);
    expect(new Set(limits).size).toBe(1);
    expect(limits[0]).toBeGreaterThan(0);
  });
});

describe('wasmMultiQuerySearch — denseWeight and sparseWeight matter only under relative_score (#2095)', () => {
  beforeEach(() => vi.clearAllMocks());

  it.each(['rrf', 'average', 'maximum', 'weighted'] as const)(
    '%s ignores denseWeight and sparseWeight, which it never reads',
    async (fusion) => {
      const multi = vi.fn(() => []);
      const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

      await wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], {
        fusion,
        fusionParams: { denseWeight: 0.7, sparseWeight: 0.3 },
      });

      expect(multi).toHaveBeenCalledTimes(1);
    }
  );
});

// ---------------------------------------------------------------------------
// #2095 — the rest of core's input rules: strategy names, vector count, LIMIT.
// ---------------------------------------------------------------------------

describe('wasmMultiQuerySearch — a strategy name is read as core reads it (#2095)', () => {
  beforeEach(() => vi.clearAllMocks());

  it.each(['rsf', 'RELATIVE_SCORE', 'Relative_Score'])(
    '%s is relative_score, so its denseWeight is refused',
    async (fusion) => {
      const multi = vi.fn(() => []);
      const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

      const outcome = await settle(
        wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], {
          fusion: fusion as never,
          fusionParams: { denseWeight: 0.7 },
        })
      );

      expectRefusal(outcome, 'multiQueryFusionParams');
      expect(multi).not.toHaveBeenCalled();
    }
  );

  it.each(['WEIGHTED', 'Weighted'])("%s is weighted, so the caller's triple reaches the binding", async (fusion) => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    await wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], {
      fusion: fusion as never,
      fusionParams: { avgWeight: 0.5, maxWeight: 0.375, hitWeight: 0.125 },
    });

    const call = multi.mock.calls[0] as unknown[];
    expect(call[3]).toBe('weighted');
    expect(Array.from(call[5] as Float32Array)).toEqual([0.5, 0.375, 0.125]);
  });

  it.each([
    ['avg', 'average'],
    ['MAX', 'maximum'],
    ['RRF', 'rrf'],
  ])('%s reaches the binding as %s', async (fusion, canonical) => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    await wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], { fusion: fusion as never });

    expect((multi.mock.calls[0] as unknown[])[3]).toBe(canonical);
  });

  it.each(['mean', 'constructor'])('refuses %s, a strategy core does not know', async (fusion) => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    const outcome = await settle(
      wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], { fusion: fusion as never })
    );

    expect(outcome).toBeInstanceOf(VelesDBError);
    expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
    expect(multi).not.toHaveBeenCalled();
  });
});

describe('WASM search — the rest of core input rules (#2095)', () => {
  beforeEach(() => vi.clearAllMocks());

  it('multiQuerySearch refuses more than 10 vectors, as core does', async () => {
    const store = buildStore();
    const ctx = buildCtx('docs', store, { dimension: 2 });

    const outcome = await settle(
      wasmMultiQuerySearch(ctx, 'docs', Array.from({ length: 11 }, () => [0.1, 0.2]))
    );

    expect(outcome).toBeInstanceOf(VelesDBError);
    expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
    expect(bindingCallCount(store)).toBe(0);
  });

  it('multiQuerySearch takes 10 vectors', async () => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }), { dimension: 2 });

    await wasmMultiQuerySearch(ctx, 'docs', Array.from({ length: 10 }, () => [0.1, 0.2]));

    expect(multi).toHaveBeenCalledTimes(1);
  });

  it("query caps LIMIT at core's MAX_LIMIT of 100,000", async () => {
    const query = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ query }), { dimension: 2 });

    await wasmQuery(ctx, 'docs', 'SELECT * FROM docs WHERE vector NEAR $v LIMIT 250000', {
      v: [0.1, 0.2],
    });

    expect(query).toHaveBeenLastCalledWith(expect.any(Float32Array), 100_000);
  });
});

// ---------------------------------------------------------------------------
// #2095 round 7 — a strategy that is not a string, and a LIMIT past a u64.
// ---------------------------------------------------------------------------

describe('wasmMultiQuerySearch — a strategy that is not a string (#2095)', () => {
  beforeEach(() => vi.clearAllMocks());

  // `Object.create(null)` has no prototype, so `String()` throws on it: the refusal
  // must name a value that is not a string by its type, never by coercing it.
  it.each([5, { name: 'rrf' }, Object.create(null)])('refuses %s with BAD_REQUEST, before the binding sees it', async (fusion) => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    const outcome = await settle(
      wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], { fusion: fusion as never })
    );

    expect(outcome).toBeInstanceOf(VelesDBError);
    expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
    expect((outcome as VelesDBError).message).toContain(`of type ${typeof fusion}`);
    expect(multi).not.toHaveBeenCalled();
  });

  it('reads a null strategy as absent, rrf, as the REST backend does', async () => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    await wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], { fusion: null as never });

    expect((multi.mock.calls[0] as unknown[])[3]).toBe('rrf');
  });
});

describe("wasmQuery — LIMIT is read as core's parser reads it, a u64 (#2095)", () => {
  beforeEach(() => vi.clearAllMocks());

  it('refuses LIMIT 18446744073709551616, one past the largest u64', async () => {
    const query = vi.fn(() => [{ id: 1 }]);
    const ctx = buildCtx('docs', buildStore({ query }), { dimension: 2 });

    const outcome = await settle(
      wasmQuery(ctx, 'docs', 'SELECT * FROM docs WHERE vector NEAR $v LIMIT 18446744073709551616', {
        v: [0.1, 0.2],
      })
    );

    expect(outcome).toBeInstanceOf(VelesDBError);
    expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
    expect(query).not.toHaveBeenCalled();
  });

  it('caps LIMIT 18446744073709551615, the largest u64, at 100,000', async () => {
    const query = vi.fn(() => [{ id: 1 }]);
    const ctx = buildCtx('docs', buildStore({ query }), { dimension: 2 });

    await wasmQuery(ctx, 'docs', 'SELECT * FROM docs WHERE vector NEAR $v LIMIT 18446744073709551615', {
      v: [0.1, 0.2],
    });

    expect(query).toHaveBeenLastCalledWith(expect.any(Float32Array), 100_000);
  });
});

// ---------------------------------------------------------------------------
// #2095 round 9 — every number this backend reads is checked for the binding
// that receives it: a 32-bit `k`, a u32 RRF `k`, and a value that is not a
// number named by its type, never coerced.
// ---------------------------------------------------------------------------

/** The largest integer velesdb-wasm's 32-bit `usize` and core's `u32` carry. */
const LARGEST_U32 = 2 ** 32 - 1;

/** The message part naming a refused value: its type when it is not a number. */
function named(value: unknown): string {
  return typeof value === 'number' ? String(value) : `of type ${typeof value}`;
}

describe("WASM search — k fits velesdb-wasm's 32-bit usize (#2095)", () => {
  beforeEach(() => vi.clearAllMocks());

  it.each([
    ['search', 'search', (ctx: WasmContext, k: number) => wasmSearch(ctx, 'docs', [0.1, 0.2], { k })],
    [
      'searchBatch',
      'search',
      (ctx: WasmContext, k: number) => wasmSearchBatch(ctx, 'docs', [{ vector: [0.1, 0.2], k }]),
    ],
    [
      'multiQuerySearch',
      'multi_query_search',
      (ctx: WasmContext, k: number) => wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], { k }),
    ],
  ] as const)('%s passes k = 2^32 - 1 to the binding', async (_label, method, call) => {
    const binding = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ [method]: binding }), { dimension: 2 });

    await call(ctx, LARGEST_U32);

    expect(binding).toHaveBeenCalledTimes(1);
    expect((binding.mock.calls[0] as unknown[]).includes(LARGEST_U32)).toBe(true);
  });

  it.each([2 ** 32, 2 ** 32 + 2])(
    'every search path refuses k = %s, which the binding would wrap modulo 2^32',
    async (k) => {
      const store = buildStore({ search: vi.fn(() => [[1n, 0.9]]) });
      const ctx = buildCtx('docs', store, { dimension: 2 });

      const calls = [
        () => wasmSearch(ctx, 'docs', [0.1, 0.2], { k }),
        () => wasmSearchBatch(ctx, 'docs', [{ vector: [0.1, 0.2], k }]),
        () => wasmTextSearch(ctx, 'docs', 'q', { k }),
        () => wasmHybridSearch(ctx, 'docs', [0.1, 0.2], 'q', { k }),
        () => wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], { k }),
      ];
      for (const call of calls) {
        const outcome = await settle(call());
        expect(outcome).toBeInstanceOf(VelesDBError);
        expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
      }
      expect(bindingCallCount(store)).toBe(0);
    }
  );
});

describe('wasmMultiQuerySearch — fusionParams.k is a u32, as core reads it (#2095)', () => {
  beforeEach(() => vi.clearAllMocks());

  it.each([0, LARGEST_U32])('passes fusionParams.k = %s to the binding', async (k) => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    await wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], { fusionParams: { k } });

    expect((multi.mock.calls[0] as unknown[])[4]).toBe(k);
  });

  // REST's `rrf_k` is a `u32` field: a value that does not deserialize refuses the
  // whole request, whichever strategy it names.
  it.each([
    ['rrf', -1],
    ['rrf', 1.5],
    ['rrf', Number.NaN],
    ['rrf', 2 ** 32],
    ['rrf', 'abc'],
    ['rrf', Object.create(null)],
    ['average', -1],
  ] as const)('under %s, refuses fusionParams.k = %s before the binding sees it', async (fusion, k) => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    const outcome = await settle(
      wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], { fusion, fusionParams: { k: k as never } })
    );

    expect(outcome).toBeInstanceOf(VelesDBError);
    expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
    expect((outcome as VelesDBError).message).toContain(named(k));
    expect(multi).not.toHaveBeenCalled();
  });
});

describe('WASM search — a number option that is not a number is BAD_REQUEST, named by its type (#2095)', () => {
  beforeEach(() => vi.clearAllMocks());

  // `Object.create(null)` has no prototype, so any coercion of it throws a TypeError;
  // a string would be coerced by the binding, where REST's JSON number refuses it.
  const NOT_NUMBERS = [
    ['a string', '5'],
    ['an object with no prototype', Object.create(null)],
  ] as const;

  it.each(NOT_NUMBERS)('every search path refuses k as %s', async (_label, k) => {
    const store = buildStore();
    const ctx = buildCtx('docs', store, { dimension: 2 });

    const calls = [
      () => wasmSearch(ctx, 'docs', [0.1, 0.2], { k: k as never }),
      () => wasmSearchBatch(ctx, 'docs', [{ vector: [0.1, 0.2], k: k as never }]),
      () => wasmTextSearch(ctx, 'docs', 'q', { k: k as never }),
      () => wasmHybridSearch(ctx, 'docs', [0.1, 0.2], 'q', { k: k as never }),
      () => wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], { k: k as never }),
    ];
    for (const call of calls) {
      const outcome = await settle(call());
      expect(outcome).toBeInstanceOf(VelesDBError);
      expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
      expect((outcome as VelesDBError).message).toContain(named(k));
    }
    expect(bindingCallCount(store)).toBe(0);
  });

  it.each(NOT_NUMBERS)('multiQuerySearch refuses a weighted triple holding %s', async (_label, weight) => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    const outcome = await settle(
      wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], {
        fusion: 'weighted',
        fusionParams: { avgWeight: weight as never, maxWeight: 0.5, hitWeight: 0 },
      })
    );

    expect(outcome).toBeInstanceOf(VelesDBError);
    expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
    expect((outcome as VelesDBError).message).toContain(named(weight));
    expect(multi).not.toHaveBeenCalled();
  });

  // With k = 0 too: the check runs before the early return, as every input check does.
  it.each(NOT_NUMBERS)('hybridSearch refuses vectorWeight as %s, whatever k is', async (_label, vectorWeight) => {
    const hybrid = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ hybrid_search: hybrid }), { dimension: 2 });

    for (const k of [10, 0]) {
      const outcome = await settle(
        wasmHybridSearch(ctx, 'docs', [0.1, 0.2], 'q', { k, vectorWeight: vectorWeight as never })
      );

      expect(outcome).toBeInstanceOf(VelesDBError);
      expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
      expect((outcome as VelesDBError).message).toContain(named(vectorWeight));
    }
    expect(hybrid).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// #2095 round 10 — a weight is a finite number, as REST's f32 fields are, and
// every fusionParams weight sent is checked whichever strategy reads it.
// ---------------------------------------------------------------------------

/**
 * Values REST's f32 fields refuse: `JSON.stringify` sends each as `null`,
 * which serde does not read as an f32.
 */
const NON_FINITE = [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY];

describe('WASM search — a weight is a finite number, as REST reads an f32 (#2095)', () => {
  beforeEach(() => vi.clearAllMocks());

  it.each(NON_FINITE)('hybridSearch refuses vectorWeight = %s before the binding sees it', async (vectorWeight) => {
    const hybrid = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ hybrid_search: hybrid }), { dimension: 2 });

    const outcome = await settle(wasmHybridSearch(ctx, 'docs', [0.1, 0.2], 'q', { vectorWeight }));

    expect(outcome).toBeInstanceOf(VelesDBError);
    expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
    expect((outcome as VelesDBError).message).toContain(`vectorWeight must be a finite number; got ${vectorWeight}`);
    expect(hybrid).not.toHaveBeenCalled();
  });

  it.each(NON_FINITE)(
    'multiQuerySearch refuses a weighted triple holding %s, naming the field',
    async (avgWeight) => {
      const multi = vi.fn(() => []);
      const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

      const outcome = await settle(
        wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], {
          fusion: 'weighted',
          fusionParams: { avgWeight, maxWeight: 0.5, hitWeight: 0.5 },
        })
      );

      expect(outcome).toBeInstanceOf(VelesDBError);
      expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
      expect((outcome as VelesDBError).message).toContain(
        `fusionParams.avgWeight must be a finite number; got ${avgWeight}`
      );
      expect(multi).not.toHaveBeenCalled();
    }
  );

  // REST deserializes every field of the request before it reads the strategy, so a
  // weight of the wrong type refuses it even where the strategy would never read it.
  it.each([
    ['rrf', 'avgWeight', 'abc'],
    ['rrf', 'maxWeight', Number.NEGATIVE_INFINITY],
    ['rrf', 'hitWeight', Object.create(null)],
    ['rrf', 'denseWeight', Number.NaN],
    ['average', 'sparseWeight', Number.POSITIVE_INFINITY],
    ['weighted', 'denseWeight', Number.NaN],
    ['relative_score', 'avgWeight', 'abc'],
    ['relative_score', 'denseWeight', Number.NaN],
    // The REST backend sends a null weight as JSON `null`, which an f32 field refuses.
    ['rrf', 'avgWeight', null],
    ['average', 'sparseWeight', null],
    ['weighted', 'hitWeight', null],
  ] as const)('under %s, refuses fusionParams.%s = %s with BAD_REQUEST', async (fusion, name, value) => {
    const multi = vi.fn(() => []);
    const ctx = buildCtx('docs', buildStore({ multi_query_search: multi }));

    const outcome = await settle(
      wasmMultiQuerySearch(ctx, 'docs', [[0.1, 0.2]], {
        fusion,
        fusionParams: { [name]: value as never },
      })
    );

    expect(outcome).toBeInstanceOf(VelesDBError);
    expect((outcome as VelesDBError).code).toBe('BAD_REQUEST');
    expect((outcome as VelesDBError).message).toContain(`fusionParams.${name} must be a finite number`);
    expect((outcome as VelesDBError).message).toContain(named(value));
    expect(multi).not.toHaveBeenCalled();
  });
});
