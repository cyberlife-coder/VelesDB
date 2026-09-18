/**
 * `searchBatch`'s own filter guard (#2282).
 *
 * `wasmSearchBatch` calls `requireWasmFilterSupport('searchBatch', …)` before
 * running any entry. Under the shipped map that call can never throw —
 * `WASM_CAPABILITIES.filteredSearch` lists `searchBatch` — so deleting it
 * left the whole suite green: the guard was untested, not dead. What it
 * guards is the map changing: `wasmSearch` asks about `'search'`, so without
 * this call a map that stopped listing `searchBatch` would let a batch run
 * filtered anyway, under another operation's permission.
 *
 * The map is a frozen module constant, so this file mocks it — hence a file
 * of its own, where `vi.mock` applies to every import.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

vi.mock('../src/capabilities', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/capabilities')>();
  return {
    ...actual,
    WASM_CAPABILITIES: Object.freeze({
      ...actual.WASM_CAPABILITIES,
      // `search` stays listed, so only `searchBatch`'s own guard can refuse.
      filteredSearch: Object.freeze(['search']),
    }),
  };
});

import { wasmSearchBatch } from '../src/backends/wasm-search';
import { VelesDBError } from '../src/types';
import { newSparseIds } from '../src/backends/wasm-sparse';
import type {
  CollectionData,
  WasmContext,
  WasmModule,
  WasmVectorStore,
} from '../src/backends/wasm-types';

const search = vi.fn(() => [[1n, 0.9]]);
const search_with_filter = vi.fn(() => []);

function buildCtx(): { ctx: WasmContext; store: WasmVectorStore } {
  const store = {
    search,
    search_with_quality: vi.fn((q: Float32Array, k: number) => search(q, k)),
    search_with_filter,
    sparse_search: vi.fn(() => []),
    text_search: vi.fn(() => []),
    hybrid_search: vi.fn(() => []),
    multi_query_search: vi.fn(() => []),
    query: vi.fn(() => []),
    free: vi.fn(),
    len: 1,
    is_empty: false,
  } as unknown as WasmVectorStore;

  const data: CollectionData = {
    config: { dimension: 2, metric: 'cosine' },
    store,
    payloads: new Map(),
    sparseIds: newSparseIds(),
    createdAt: new Date(),
  };

  const ctx: WasmContext = {
    wasmModule: {
      default: vi.fn(() => Promise.resolve()),
      VectorStore: { new_metadata_only: () => store } as unknown as WasmModule['VectorStore'],
      hybrid_search_fuse: vi.fn(() => []),
    } as WasmModule,
    getCollection: (name: string) => (name === 'docs' ? data : undefined),
    canonicalPayloadKeyFromResultId: (id) => String(id),
    canonicalPayloadKey: (id) => String(id),
    sparseVectorToArrays: () => ({ indices: [], values: [] }),
    toNumericId: (id) => (typeof id === 'number' ? id : Number(id) || 0),
  };
  return { ctx, store };
}

describe("wasmSearchBatch — a filter is refused under searchBatch's own name", () => {
  beforeEach(() => vi.clearAllMocks());

  it('refuses the batch, naming searchBatch and not search', async () => {
    const { ctx } = buildCtx();

    await expect(
      wasmSearchBatch(ctx, 'docs', [{ vector: [0.1, 0.2], filter: { tenant: 'a' } }])
    ).rejects.toBeInstanceOf(VelesDBError);
    await expect(
      wasmSearchBatch(ctx, 'docs', [{ vector: [0.1, 0.2], filter: { tenant: 'a' } }])
    ).rejects.toThrow(/searchBatch with a filter/);
  });

  it('refuses before any entry searches, so no entry runs half a batch', async () => {
    const { ctx } = buildCtx();

    await expect(
      wasmSearchBatch(ctx, 'docs', [
        { vector: [0.1, 0.2] },
        { vector: [0.3, 0.4], filter: { tenant: 'a' } },
      ])
    ).rejects.toThrow(/searchBatch with a filter/);

    expect(search).not.toHaveBeenCalled();
    expect(search_with_filter).not.toHaveBeenCalled();
  });

  it('runs the batch when no entry carries a filter', async () => {
    const { ctx } = buildCtx();

    const results = await wasmSearchBatch(ctx, 'docs', [
      { vector: [0.1, 0.2] },
      { vector: [0.3, 0.4] },
    ]);

    expect(results).toHaveLength(2);
    expect(search).toHaveBeenCalledTimes(2);
  });
});
