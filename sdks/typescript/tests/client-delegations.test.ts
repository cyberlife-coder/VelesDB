/**
 * VelesDB Client delegation tests (#598)
 *
 * Covers `src/client.ts`, `src/client/search-methods.ts`, and
 * `src/client/graph-methods.ts` by injecting a mocked `IVelesDBBackend`
 * via `(client as any).backend = mock` and `.initialized = true`.
 *
 * Focus:
 *  - every search / admin / streaming / scroll / stats facade method
 *    on `VelesDB` forwards to the backend with the right arguments.
 *  - every graph facade method validates its inputs and forwards.
 *  - `capabilities()`, `close()`, `isEmpty`, `flush`, `isInitialized`
 *    reach the backend.
 *
 * Complements `client.test.ts` (happy-path + input validation) by
 * driving the full delegation surface in one file.
 */

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { VelesDB } from '../src/client';
import type { IVelesDBBackend } from '../src/types';
import { ValidationError } from '../src/types';

function buildBackend(): IVelesDBBackend {
  return {
    init: vi.fn(() => Promise.resolve()),
    close: vi.fn(() => Promise.resolve()),
    isInitialized: vi.fn(() => true),
    capabilities: vi.fn(() => ({}) as ReturnType<IVelesDBBackend['capabilities']>),

    createCollection: vi.fn(() => Promise.resolve()),
    deleteCollection: vi.fn(() => Promise.resolve()),
    getCollection: vi.fn(() => Promise.resolve(null)),
    listCollections: vi.fn(() => Promise.resolve([])),

    upsert: vi.fn(() => Promise.resolve()),
    upsertBatch: vi.fn(() => Promise.resolve()),
    delete: vi.fn(() => Promise.resolve(true)),
    get: vi.fn(() => Promise.resolve(null)),
    isEmpty: vi.fn(() => Promise.resolve(true)),
    flush: vi.fn(() => Promise.resolve()),

    search: vi.fn(() => Promise.resolve([])),
    searchBatch: vi.fn(() => Promise.resolve([])),
    textSearch: vi.fn(() => Promise.resolve([])),
    hybridSearch: vi.fn(() => Promise.resolve([])),
    multiQuerySearch: vi.fn(() => Promise.resolve([])),
    searchIds: vi.fn(() => Promise.resolve([])),

    scroll: vi.fn(() => Promise.resolve({ points: [] })),
    trainPq: vi.fn(() => Promise.resolve('ok')),
    streamInsert: vi.fn(() => Promise.resolve()),
    streamUpsertPoints: vi.fn(() =>
      Promise.resolve({ upserted: 0, errors: 0 })
    ),

    query: vi.fn(() => Promise.resolve({ results: [], stats: {} })),
    queryExplain: vi.fn(() => Promise.resolve({ plan: {} })),
    collectionSanity: vi.fn(() => Promise.resolve({ ok: true })),

    getCollectionStats: vi.fn(() => Promise.resolve(null)),
    analyzeCollection: vi.fn(() => Promise.resolve({} as never)),
    getCollectionConfig: vi.fn(() => Promise.resolve({} as never)),

    rebuildIndex: vi.fn(() => Promise.resolve({ rebuilt: true } as never)),
    getGuardrails: vi.fn(() => Promise.resolve({} as never)),
    updateGuardrails: vi.fn(() => Promise.resolve({} as never)),
    aggregate: vi.fn(() => Promise.resolve({} as never)),

    matchQuery: vi.fn(() => Promise.resolve({} as never)),
    removeEdge: vi.fn(() => Promise.resolve(true)),
    getEdgeCount: vi.fn(() => Promise.resolve(0)),
    listNodes: vi.fn(() => Promise.resolve({} as never)),
    getNodeEdges: vi.fn(() => Promise.resolve([])),
    getNodePayload: vi.fn(() => Promise.resolve({} as never)),
    upsertNodePayload: vi.fn(() => Promise.resolve()),
    graphSearch: vi.fn(() => Promise.resolve({} as never)),

    addEdge: vi.fn(() => Promise.resolve()),
    getEdges: vi.fn(() => Promise.resolve([])),
    traverseGraph: vi.fn(() => Promise.resolve({} as never)),
    traverseParallel: vi.fn(() => Promise.resolve({} as never)),
    getNodeDegree: vi.fn(() =>
      Promise.resolve({ inDegree: 0, outDegree: 0 })
    ),
    createGraphCollection: vi.fn(() => Promise.resolve()),

    createIndex: vi.fn(() => Promise.resolve()),
    listIndexes: vi.fn(() => Promise.resolve([])),
    hasIndex: vi.fn(() => Promise.resolve(false)),
    dropIndex: vi.fn(() => Promise.resolve(false)),

    storeSemanticFact: vi.fn(() => Promise.resolve()),
    searchSemanticMemory: vi.fn(() => Promise.resolve([])),
    recordEpisodicEvent: vi.fn(() => Promise.resolve()),
    recallEpisodicEvents: vi.fn(() => Promise.resolve([])),
    storeProceduralPattern: vi.fn(() => Promise.resolve()),
    matchProceduralPatterns: vi.fn(() => Promise.resolve([])),
  } as unknown as IVelesDBBackend;
}

function injectBackend(
  db: VelesDB,
  backend: IVelesDBBackend
): void {
  (db as unknown as { initialized: boolean; backend: IVelesDBBackend }).initialized =
    true;
  (db as unknown as { backend: IVelesDBBackend }).backend = backend;
}

function setup(): { db: VelesDB; backend: IVelesDBBackend } {
  const db = new VelesDB({ backend: 'wasm' });
  const backend = buildBackend();
  injectBackend(db, backend);
  return { db, backend };
}

describe('VelesDB — config + lifecycle', () => {
  it('createMetadataCollection calls backend with collectionType=metadata_only', async () => {
    const { db, backend } = setup();
    await db.createMetadataCollection('m');
    expect(backend.createCollection).toHaveBeenCalledWith('m', {
      collectionType: 'metadata_only',
    });
  });

  it('createCollection rejects metadata_only with empty name', async () => {
    const { db } = setup();
    await expect(db.createMetadataCollection('')).rejects.toThrow(
      ValidationError
    );
  });

  it('createCollection rejects when dimension is missing on vector collections', async () => {
    const { db } = setup();
    await expect(
      db.createCollection('c', { dimension: 0 } as never)
    ).rejects.toThrow(ValidationError);
  });

  it('capabilities() delegates to backend.capabilities()', () => {
    const { db, backend } = setup();
    db.capabilities();
    expect(backend.capabilities).toHaveBeenCalled();
  });

  it('close() flips initialized to false when already initialized', async () => {
    const { db, backend } = setup();
    await db.close();
    expect(backend.close).toHaveBeenCalled();
    expect(db.isInitialized()).toBe(false);
  });

  it('close() is a no-op when not initialized', async () => {
    const db = new VelesDB({ backend: 'wasm' });
    const backend = buildBackend();
    (db as unknown as { backend: IVelesDBBackend }).backend = backend;
    await db.close();
    expect(backend.close).not.toHaveBeenCalled();
  });

  it('init() sets initialized', async () => {
    const db = new VelesDB({ backend: 'wasm' });
    const backend = buildBackend();
    (db as unknown as { backend: IVelesDBBackend }).backend = backend;
    await db.init();
    expect(backend.init).toHaveBeenCalled();
    expect(db.isInitialized()).toBe(true);
  });

  it('init() is idempotent', async () => {
    const { db, backend } = setup();
    await db.init();
    // already initialized → backend.init not called
    expect(backend.init).not.toHaveBeenCalled();
  });

  it('isEmpty delegates', async () => {
    const { db, backend } = setup();
    await db.isEmpty('c');
    expect(backend.isEmpty).toHaveBeenCalledWith('c');
  });

  it('flush delegates', async () => {
    const { db, backend } = setup();
    await db.flush('c');
    expect(backend.flush).toHaveBeenCalledWith('c');
  });
});

describe('VelesDB — search / admin / scroll delegations (search-methods.ts)', () => {
  let db: VelesDB;
  let backend: IVelesDBBackend;

  beforeEach(() => {
    ({ db, backend } = setup());
  });

  it('textSearch validates query and delegates', async () => {
    await db.textSearch('c', 'hello', { k: 5 });
    expect(backend.textSearch).toHaveBeenCalledWith('c', 'hello', { k: 5 });
    await expect(db.textSearch('c', '')).rejects.toThrow(ValidationError);
  });

  it('hybridSearch validates vector + text and delegates', async () => {
    await db.hybridSearch('c', [0.1, 0.2], 'q');
    expect(backend.hybridSearch).toHaveBeenCalled();
    await expect(db.hybridSearch('c', [0.1], '')).rejects.toThrow(
      ValidationError
    );
  });

  it('searchBatch rejects non-array and delegates otherwise', async () => {
    await db.searchBatch('c', [{ vector: [0.1] }]);
    expect(backend.searchBatch).toHaveBeenCalled();
    await expect(db.searchBatch('c', null as never)).rejects.toThrow(
      ValidationError
    );
  });

  it('searchIds delegates', async () => {
    await db.searchIds('c', [0.1], { k: 3 });
    expect(backend.searchIds).toHaveBeenCalledWith('c', [0.1], { k: 3 });
  });

  it('query validates collection + string and delegates', async () => {
    await db.query('c', 'q');
    expect(backend.query).toHaveBeenCalled();
    await expect(db.query('', 'q')).rejects.toThrow(ValidationError);
    await expect(db.query('c', '')).rejects.toThrow(ValidationError);
  });

  it('scroll validates batchSize bounds', async () => {
    await db.scroll('c');
    expect(backend.scroll).toHaveBeenCalled();
    await expect(db.scroll('c', { batchSize: 0 })).rejects.toThrow(
      ValidationError
    );
    await expect(db.scroll('c', { batchSize: 10001 })).rejects.toThrow(
      ValidationError
    );
  });

  it('trainPq delegates', async () => {
    await db.trainPq('c', { iterations: 1 } as never);
    expect(backend.trainPq).toHaveBeenCalled();
  });

  it('streamInsert validates docs and delegates', async () => {
    await db.streamInsert('c', [{ id: 1, vector: [0.1, 0.2] }]);
    expect(backend.streamInsert).toHaveBeenCalled();
  });

  it('streamUpsertPoints validates and delegates', async () => {
    await db.streamUpsertPoints('c', [{ id: 1, vector: [0.1, 0.2] }]);
    expect(backend.streamUpsertPoints).toHaveBeenCalled();
  });

  it('getCollectionStats / analyzeCollection / getCollectionConfig delegate', async () => {
    await db.getCollectionStats('c');
    expect(backend.getCollectionStats).toHaveBeenCalledWith('c');
    await db.analyzeCollection('c');
    expect(backend.analyzeCollection).toHaveBeenCalledWith('c');
    await db.getCollectionConfig('c');
    expect(backend.getCollectionConfig).toHaveBeenCalledWith('c');
  });

  it('rebuildIndex / getGuardrails / updateGuardrails / aggregate delegate', async () => {
    await db.rebuildIndex('c');
    expect(backend.rebuildIndex).toHaveBeenCalledWith('c');
    await db.getGuardrails();
    expect(backend.getGuardrails).toHaveBeenCalled();
    await db.updateGuardrails({ enabled: true });
    expect(backend.updateGuardrails).toHaveBeenCalled();
    await db.aggregate('q');
    expect(backend.aggregate).toHaveBeenCalled();
    await expect(db.aggregate('')).rejects.toThrow(ValidationError);
    await expect(db.rebuildIndex('')).rejects.toThrow(ValidationError);
  });

  it('collectionSanity + queryExplain validate + delegate', async () => {
    await db.queryExplain('q');
    expect(backend.queryExplain).toHaveBeenCalled();
    await expect(db.queryExplain('')).rejects.toThrow(ValidationError);
    await db.collectionSanity('c');
    expect(backend.collectionSanity).toHaveBeenCalledWith('c');
    await expect(db.collectionSanity('')).rejects.toThrow(ValidationError);
  });
});

describe('VelesDB — index management delegations', () => {
  let db: VelesDB;
  let backend: IVelesDBBackend;

  beforeEach(() => {
    ({ db, backend } = setup());
  });

  it('createIndex rejects missing label/property', async () => {
    await expect(
      db.createIndex('c', { label: '', property: 'p' } as never)
    ).rejects.toThrow(ValidationError);
    await expect(
      db.createIndex('c', { label: 'L', property: '' } as never)
    ).rejects.toThrow(ValidationError);
  });

  it('createIndex / listIndexes / hasIndex / dropIndex delegate', async () => {
    await db.createIndex('c', { label: 'L', property: 'p' });
    expect(backend.createIndex).toHaveBeenCalled();
    await db.listIndexes('c');
    expect(backend.listIndexes).toHaveBeenCalledWith('c');
    await db.hasIndex('c', 'L', 'p');
    expect(backend.hasIndex).toHaveBeenCalledWith('c', 'L', 'p');
    await db.dropIndex('c', 'L', 'p');
    expect(backend.dropIndex).toHaveBeenCalledWith('c', 'L', 'p');
  });
});

describe('VelesDB — graph delegations (graph-methods.ts)', () => {
  let db: VelesDB;
  let backend: IVelesDBBackend;

  beforeEach(() => {
    ({ db, backend } = setup());
  });

  it('addEdge rejects missing label', async () => {
    await expect(
      db.addEdge('c', { id: 1, source: 10, target: 20, label: '' })
    ).rejects.toThrow(ValidationError);
  });

  it('addEdge rejects non-numeric source/target', async () => {
    await expect(
      db.addEdge('c', {
        id: 1,
        source: 'a' as unknown as number,
        target: 20,
        label: 'L',
      })
    ).rejects.toThrow(ValidationError);
    await expect(
      db.addEdge('c', {
        id: 1,
        source: 10,
        target: 'a' as unknown as number,
        label: 'L',
      })
    ).rejects.toThrow(ValidationError);
  });

  it('addEdge + getEdges + traverseGraph + traverseParallel + getNodeDegree + createGraphCollection delegate', async () => {
    await db.addEdge('c', { id: 1, source: 10, target: 20, label: 'L' });
    expect(backend.addEdge).toHaveBeenCalled();

    await db.getEdges('c', { label: 'L' });
    expect(backend.getEdges).toHaveBeenCalledWith('c', { label: 'L' });

    await db.traverseGraph('c', { source: 10 });
    expect(backend.traverseGraph).toHaveBeenCalled();

    await db.traverseParallel('c', { sources: [1, 2] });
    expect(backend.traverseParallel).toHaveBeenCalled();

    await db.getNodeDegree('c', 42);
    expect(backend.getNodeDegree).toHaveBeenCalledWith('c', 42);

    await db.createGraphCollection('kg');
    expect(backend.createGraphCollection).toHaveBeenCalled();
  });

  it('traverseGraph rejects invalid strategy and non-numeric source', async () => {
    await expect(
      db.traverseGraph('c', { source: 1, strategy: 'bad' as never })
    ).rejects.toThrow(ValidationError);
    await expect(
      db.traverseGraph('c', { source: 'x' as unknown as number })
    ).rejects.toThrow(ValidationError);
  });

  it('traverseParallel rejects empty sources', async () => {
    await expect(
      db.traverseParallel('c', { sources: [] })
    ).rejects.toThrow(ValidationError);
  });

  it('getNodeDegree rejects non-numeric nodeId', async () => {
    await expect(
      db.getNodeDegree('c', 'x' as unknown as number)
    ).rejects.toThrow(ValidationError);
  });

  it('matchQuery validates and delegates', async () => {
    await db.matchQuery('c', 'q');
    expect(backend.matchQuery).toHaveBeenCalled();
    await expect(db.matchQuery('', 'q')).rejects.toThrow(ValidationError);
    await expect(db.matchQuery('c', '')).rejects.toThrow(ValidationError);
  });

  it('removeEdge / getEdgeCount / listNodes validate collection and delegate', async () => {
    await db.removeEdge('c', 7);
    expect(backend.removeEdge).toHaveBeenCalledWith('c', 7);
    await expect(db.removeEdge('', 7)).rejects.toThrow(ValidationError);

    await db.getEdgeCount('c');
    expect(backend.getEdgeCount).toHaveBeenCalledWith('c');
    await expect(db.getEdgeCount('')).rejects.toThrow(ValidationError);

    await db.listNodes('c');
    expect(backend.listNodes).toHaveBeenCalledWith('c');
    await expect(db.listNodes('')).rejects.toThrow(ValidationError);
  });

  it('getNodeEdges + getNodePayload + upsertNodePayload + graphSearch validate + delegate', async () => {
    await db.getNodeEdges('c', 1);
    expect(backend.getNodeEdges).toHaveBeenCalled();
    await expect(db.getNodeEdges('', 1)).rejects.toThrow(ValidationError);

    await db.getNodePayload('c', 1);
    expect(backend.getNodePayload).toHaveBeenCalled();
    await expect(db.getNodePayload('', 1)).rejects.toThrow(ValidationError);

    await db.upsertNodePayload('c', 1, { k: 'v' });
    expect(backend.upsertNodePayload).toHaveBeenCalled();
    await expect(db.upsertNodePayload('', 1, {})).rejects.toThrow(
      ValidationError
    );

    await db.graphSearch('c', { vector: [0.1], k: 5 });
    expect(backend.graphSearch).toHaveBeenCalled();
    await expect(
      db.graphSearch('', { vector: [0.1], k: 5 })
    ).rejects.toThrow(ValidationError);
  });
});

describe('VelesDB — agent memory factory', () => {
  it('returns an AgentMemoryClient instance', () => {
    const { db } = setup();
    const mem = db.agentMemory();
    expect(mem).toBeDefined();
    expect(typeof mem).toBe('object');
  });

  it('throws when called before init', () => {
    const db = new VelesDB({ backend: 'wasm' });
    expect(() => db.agentMemory()).toThrow(ValidationError);
  });
});
