/**
 * WASM capability map ⇔ WASM backend conformance (#2095).
 *
 * `WASM_CAPABILITIES` is what a caller reads to decide whether a call will
 * work. It was written by hand beside the backend and drifted from it:
 * `sparseSearch` said `false` while `search({ sparseVector })` ran, then
 * `true` while upsert never indexed a sparse vector. This file holds every
 * key to the backend's behaviour rather than to a second list.
 *
 * For each key, probes call the real `WasmBackend` over a mocked binding.
 * Each comes out `honoured`, `refused` (NOT_SUPPORTED) or `dropped`: a probe
 * for an option that resolves while the option never took effect (its value
 * never reached the binding, nor shows in the result) was dropped, the
 * failure #2095 is about. A probe must be `honoured` where the map grants
 * the capability and `refused` where it does not, so `dropped` fails either
 * way. List capabilities are probed for every value of their universe
 * (`CAPABILITY_LIST_UNIVERSES`, derived from the SDK's types), and a key
 * with no probe fails the completeness tests.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { WasmBackend } from '../src/backends/wasm';
import { VelesDBError } from '../src/types';
import {
  CAPABILITY_LIST_UNIVERSES,
  WASM_CAPABILITIES,
  type ListCapability,
} from '../src/capabilities';
import { FakeSparseIndex } from './helpers/fake-sparse-index';

class MockVectorStore {
  insert = vi.fn();
  insert_with_payload = vi.fn();
  insert_batch = vi.fn();
  reserve = vi.fn();
  remove = vi.fn(() => true);
  get = vi.fn(() => null);
  free = vi.fn();
  // One hit, so a probe can tell whether an option shaped the result.
  search = vi.fn(() => [[1n, 0.9]]);
  // The binding parses the preset, then runs the same brute-force search
  // (`search_with_quality` in velesdb-wasm's `vector_store.rs`).
  search_with_quality = vi.fn((q: Float32Array, k: number) => this.search(q, k));
  search_with_filter = vi.fn(() => []);
  text_search = vi.fn(() => []);
  hybrid_search = vi.fn(() => []);
  multi_query_search = vi.fn(() => []);
  query = vi.fn(() => []);
  readonly sparse = new FakeSparseIndex();
  sparse_insert = vi.fn((id: bigint, indices: Uint32Array, values: Float32Array) =>
    this.sparse.insert(id, indices, values)
  );
  sparse_search = vi.fn((indices: Uint32Array, values: Float32Array, k: number) =>
    this.sparse.search(indices, values, k)
  );
  len = 0;
  is_empty = true;
  storage_mode = 'full';
  constructor(public dimension: number, public metric: string) {}

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
  hybrid_search_fuse: vi.fn(() => []),
};

vi.mock('@wiscale/velesdb-wasm', () => mockWasmModule);

const C = 'c';
const V = [0.1, 0.2];
const FILTER = { condition: { type: 'eq', field: 'tenant', value: 'mine' } };
const NEAR = 'SELECT * FROM c WHERE vector NEAR $v LIMIT 5';

type Call = (backend: WasmBackend) => Promise<unknown>;
/** Whether the option under test took effect, judged from the binding's calls, the result or the store. */
type Applied = (result: unknown, backend: WasmBackend) => boolean;
interface Probe {
  call: Call;
  applied?: Applied;
}

/** A probe for a whole operation: running it is honouring it. */
const operation = (call: Call): Probe => ({ call });
/** A probe for an option: running it is not enough, the option must take effect. */
const option = (call: Call, applied: Applied): Probe => ({ call, applied });

interface CollectionInternals {
  store: MockVectorStore;
  config: Record<string, unknown>;
}

function collectionOf(backend: WasmBackend, name: string): CollectionInternals | undefined {
  const internals = backend as unknown as { collections: Map<string, CollectionInternals> };
  return internals.collections.get(name);
}

/** Every argument collection `c`'s mocked binding received, typed arrays flattened. */
function bindingArgs(backend: WasmBackend): unknown[] {
  const store = collectionOf(backend, C)!.store;
  const mocks = [
    store.search,
    store.search_with_filter,
    store.sparse_search,
    store.text_search,
    store.hybrid_search,
    store.multi_query_search,
    store.query,
    mockWasmModule.hybrid_search_fuse,
  ];
  return mocks
    .flatMap((mock) => mock.mock.calls.flat())
    .flatMap((arg) => (ArrayBuffer.isView(arg) ? Array.from(arg as Float32Array) : [arg]));
}

const reachesBinding =
  (value: unknown): Applied =>
  (_result, backend) =>
    bindingArgs(backend).includes(value);

const returnsPoint =
  (id: string): Applied =>
  (result) =>
    Array.isArray(result) && result.some((row) => (row as { id?: unknown }).id === id);

const returnsVectors: Applied = (result) =>
  Array.isArray(result) &&
  result.length > 0 &&
  result.every((row) => (row as { vector?: unknown }).vector !== undefined);

/** velesdb-wasm has nowhere to put this option: were it granted, it would be dropped. */
const neverApplied: Applied = () => false;

/** `table[value]`, or an error naming the list value that has no probe. */
function entryFor<T>(table: Record<string, T>, key: string, value: string): T {
  const entry = table[value];
  if (entry === undefined) {
    throw new Error(`no probe for ${key} value '${value}'`);
  }
  return entry;
}

/** A filtered call for each `filteredSearch` value. */
const FILTERED_CALLS: Record<string, Call> = {
  search: (b) => b.search(C, V, { filter: FILTER }),
  sparseSearch: (b) => b.search(C, V, { sparseVector: { 1: 0.5 }, filter: FILTER }),
  searchBatch: (b) => b.searchBatch(C, [{ vector: V, filter: FILTER }]),
  searchIds: (b) => b.searchIds(C, V, { filter: FILTER }),
  textSearch: (b) => b.textSearch(C, 'q', { filter: FILTER }),
  hybridSearch: (b) => b.hybridSearch(C, V, 'q', { filter: FILTER }),
  multiQuerySearch: (b) => b.multiQuerySearch(C, [V], { filter: FILTER }),
  multiQuerySearchIds: (b) => b.multiQuerySearchIds(C, [V], { filter: FILTER }),
  sparseSearchNamed: (b) => b.sparseSearchNamed(C, { 1: 0.5 }, 'idx', { filter: FILTER }),
  scroll: (b) => b.scroll(C, { filter: FILTER }),
};

/**
 * Each `fusionParams` field, under the strategy that reads it: the weighted
 * triple only travels whole, under `weighted`, and must sum to 1.0. The
 * values are exact in f32 and differ from every other argument a
 * multi-query call passes, so `reachesBinding` can only find them.
 */
const WEIGHTED_VALUES = { avgWeight: 0.5, maxWeight: 0.375, hitWeight: 0.125 };
const FUSION_PARAMS: Record<string, { fusion: string; fusionParams: Record<string, number> }> = {
  k: { fusion: 'rrf', fusionParams: { k: 37 } },
  avgWeight: { fusion: 'weighted', fusionParams: WEIGHTED_VALUES },
  maxWeight: { fusion: 'weighted', fusionParams: WEIGHTED_VALUES },
  hitWeight: { fusion: 'weighted', fusionParams: WEIGHTED_VALUES },
  denseWeight: { fusion: 'relative_score', fusionParams: { denseWeight: 0.75 } },
  sparseWeight: { fusion: 'relative_score', fusionParams: { sparseWeight: 0.625 } },
};

/** A `createCollection` setting for each `CollectionConfig` field, and how to see it applied. */
const CONFIG_PROBES: Record<string, { config: Record<string, unknown>; applied: Applied }> = {
  dimension: { config: { dimension: 3 }, applied: (_r, b) => collectionOf(b, 'f')?.store.dimension === 3 },
  metric: {
    config: { metric: 'euclidean' },
    applied: (_r, b) => collectionOf(b, 'f')?.store.metric === 'euclidean',
  },
  storageMode: {
    config: { storageMode: 'sq8' },
    applied: (_r, b) => collectionOf(b, 'f')?.store.storage_mode === 'sq8',
  },
  collectionType: {
    config: { collectionType: 'vector' },
    applied: (_r, b) => collectionOf(b, 'f') !== undefined,
  },
  description: {
    config: { description: 'probe' },
    applied: (_r, b) => collectionOf(b, 'f')?.config.description === 'probe',
  },
  hnsw: { config: { hnsw: { m: 16 } }, applied: neverApplied },
  pqRescoreOversampling: { config: { pqRescoreOversampling: 4 }, applied: neverApplied },
  deferredIndexing: { config: { deferredIndexing: { enabled: true } }, applied: neverApplied },
  asyncIndexBuilder: { config: { asyncIndexBuilder: { segmentCount: 2 } }, applied: neverApplied },
};

/** A `QueryOptions` setting for each field. velesdb-wasm's `query(vector, k)` takes neither. */
const QUERY_OPTIONS: Record<string, Record<string, unknown>> = {
  timeoutMs: { timeoutMs: 4321 },
  stream: { stream: true },
};

/** Probes for each boolean capability. */
const BOOLEAN_PROBES: Record<string, readonly Probe[]> = {
  vectorSearch: [
    operation((b) => b.search(C, V)),
    operation((b) => b.searchBatch(C, [{ vector: V }])),
  ],
  textSearch: [operation((b) => b.textSearch(C, 'q'))],
  hybridSearch: [operation((b) => b.hybridSearch(C, V, 'q'))],
  multiQuerySearch: [operation((b) => b.multiQuerySearch(C, [V]))],
  // Upsert, then search: a sparse search over vectors upsert never indexed runs, and finds nothing.
  sparseSearch: [
    option(async (b) => {
      await b.createCollection('sp', { dimension: 0 });
      await b.upsert('sp', { id: 1, vector: [], sparseVector: { 7: 1 } });
      return b.search('sp', [], { sparseVector: { 7: 1 } });
    }, returnsPoint('1')),
  ],
  namedSparseIndexes: [
    option(
      (b) => b.search(C, V, { sparseVector: { 1: 0.5 }, sparseIndexName: 'splade_v2' }),
      reachesBinding('splade_v2')
    ),
    operation((b) => b.sparseSearchNamed(C, { 1: 0.5 }, 'splade_v2')),
  ],
  includeVectors: [option((b) => b.search(C, V, { includeVectors: true }), returnsVectors)],
  idOnlySearch: [
    operation((b) => b.searchIds(C, V)),
    operation((b) => b.multiQuerySearchIds(C, [V])),
  ],
  scroll: [operation((b) => b.scroll(C))],
  graphTraversal: [
    operation((b) => b.addEdge(C, { id: 1, source: 1, target: 2, label: 'R' })),
    operation((b) => b.getEdges(C)),
    operation((b) => b.traverseGraph(C, { source: 1 })),
    operation((b) => b.traverseParallel(C, { sources: [1] })),
    operation((b) => b.getNodeDegree(C, 1)),
  ],
  secondaryIndexes: [
    operation((b) => b.createIndex(C, { label: 'Doc', property: 'x' })),
    operation((b) => b.listIndexes(C)),
    operation((b) => b.hasIndex(C, 'Doc', 'x')),
    operation((b) => b.dropIndex(C, 'Doc', 'x')),
  ],
  agentMemory: [
    operation((b) => b.storeSemanticFact(C, { id: 1, text: 't', embedding: V })),
    operation((b) => b.searchSemanticMemory(C, V)),
    operation((b) => b.recordEpisodicEvent(C, { eventType: 'e', data: {}, embedding: V })),
    operation((b) => b.recallEpisodicEvents(C, V)),
    operation((b) => b.storeProceduralPattern(C, { name: 'p', steps: [] })),
    operation((b) => b.matchProceduralPatterns(C, V)),
  ],
  enableStreaming: [operation((b) => b.enableStreaming(C))],
  streamInsert: [operation((b) => b.streamInsert(C, [{ id: 1, vector: V }]))],
  pqTraining: [operation((b) => b.trainPq(C))],
  velesqlQuery: [operation((b) => b.query(C, "SELECT * FROM c WHERE tenant = 'mine' LIMIT 5"))],
  collectionIntrospection: [
    operation((b) => b.collectionSanity(C)),
    operation((b) => b.getCollectionStats(C)),
    operation((b) => b.analyzeCollection(C)),
    operation((b) => b.getCollectionConfig(C)),
  ],
  velesqlMatchOrderBy: [
    operation((b) => b.query(C, 'MATCH (d:Doc) RETURN d.id ORDER BY d.id LIMIT 1')),
  ],
  velesqlAlterCollection: [
    operation((b) => b.query(C, 'ALTER COLLECTION c SET(auto_reindex=true)')),
  ],
};

/** For each list capability, the probe for one value of its universe. */
const LIST_PROBES: Record<ListCapability, (value: string) => Probe> = {
  velesqlFusionStrategies: (strategy) =>
    operation((b) =>
      b.query(
        C,
        `SELECT * FROM c WHERE vector NEAR $v USING FUSION(strategy='${strategy}') LIMIT 5`,
        { v: V }
      )
    ),
  filteredSearch: (op) =>
    option(entryFor(FILTERED_CALLS, 'filteredSearch', op), reachesBinding(FILTER)),
  multiQueryFusionParams: (name) => {
    const { fusion, fusionParams } = entryFor(FUSION_PARAMS, 'multiQueryFusionParams', name);
    return option(
      (b) => b.multiQuerySearch(C, [V], { fusion: fusion as never, fusionParams }),
      reachesBinding(fusionParams[name])
    );
  },
  storageModes: (mode) =>
    option(
      (b) => b.createCollection('m', { dimension: 2, storageMode: mode as never }),
      (_r, b) => collectionOf(b, 'm')?.store.storage_mode === mode
    ),
  // velesdb-wasm builds vector stores only: a metadata-only or graph
  // collection created here would really be a vector one.
  collectionTypes: (type) =>
    option(
      (b) => b.createCollection('t', { dimension: 2, collectionType: type as never }),
      (_r, b) => type === 'vector' && collectionOf(b, 't') !== undefined
    ),
  collectionConfig: (field) => {
    const { config, applied } = entryFor(CONFIG_PROBES, 'collectionConfig', field);
    return option((b) => b.createCollection('f', { dimension: 2, ...config }), applied);
  },
  queryOptions: (name) =>
    option(
      (b) => b.query(C, NEAR, { v: V }, entryFor(QUERY_OPTIONS, 'queryOptions', name)),
      neverApplied
    ),
};

type Outcome = 'honoured' | 'refused' | 'dropped';

async function outcomeOf(probe: Probe, backend: WasmBackend): Promise<Outcome> {
  let result: unknown;
  try {
    result = await probe.call(backend);
  } catch (error) {
    if (error instanceof VelesDBError && error.code === 'NOT_SUPPORTED') {
      return 'refused';
    }
    throw error;
  }
  if (probe.applied && !probe.applied(result, backend)) {
    return 'dropped';
  }
  return 'honoured';
}

const expectedOutcome = (granted: boolean): Outcome => (granted ? 'honoured' : 'refused');

const booleanCases = Object.entries(BOOLEAN_PROBES).flatMap(([key, probes]) =>
  probes.map((probe, index) => [key, index, probe] as const)
);

const listCases = (Object.keys(LIST_PROBES) as ListCapability[]).flatMap((key) =>
  CAPABILITY_LIST_UNIVERSES[key].map((value) => [key, value, LIST_PROBES[key](value)] as const)
);

describe('WASM_CAPABILITIES matches what WasmBackend does (#2095)', () => {
  let backend: WasmBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = new WasmBackend();
    await backend.init();
    await backend.createCollection(C, { dimension: V.length, metric: 'cosine' });
  });

  it('has a probe for every capability key', () => {
    const probed = [...Object.keys(BOOLEAN_PROBES), ...Object.keys(LIST_PROBES)].sort();
    expect(probed).toEqual(Object.keys(WASM_CAPABILITIES).sort());
  });

  it('has a universe for every list capability it probes', () => {
    expect(Object.keys(LIST_PROBES).sort()).toEqual(Object.keys(CAPABILITY_LIST_UNIVERSES).sort());
  });

  it.each(Object.keys(LIST_PROBES) as ListCapability[])(
    '%s: every value WASM grants is in its universe, so each is probed',
    (key) => {
      expect(CAPABILITY_LIST_UNIVERSES[key]).toEqual(
        expect.arrayContaining([...WASM_CAPABILITIES[key]])
      );
    }
  );

  it.each(booleanCases)(
    '%s (probe %i): the backend does what the map says',
    async (key, _index, probe) => {
      const granted = WASM_CAPABILITIES[key as keyof typeof WASM_CAPABILITIES];
      expect(typeof granted).toBe('boolean');
      expect(await outcomeOf(probe, backend)).toBe(expectedOutcome(granted as boolean));
    }
  );

  it.each(listCases)(
    '%s lists %s exactly when the backend honours it',
    async (key, value, probe) => {
      const granted = WASM_CAPABILITIES[key] as readonly string[];
      expect(await outcomeOf(probe, backend)).toBe(expectedOutcome(granted.includes(value)));
    }
  );
});
