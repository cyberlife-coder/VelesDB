/**
 * BDD tests: the WASM backend's graph operations and `matchQuery` run on the
 * `WasmDatabase` its collections are attached to.
 *
 * Before: `addEdge`, `getEdges`, `removeEdge`, `getEdgeCount` and
 * `matchQuery` threw `NOT_SUPPORTED`. A binding that exposes the
 * `WasmDatabase` edge API and `executeMatch` now answers them with REST's
 * shapes, `similarity()` scores included; an older binding keeps the old
 * refusal.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { WasmBackend } from '../src/backends/wasm';
import { VelesDBError } from '../src/types';

class MockVectorStore {
  insert = vi.fn();
  insert_with_payload = vi.fn();
  sparse_remove = vi.fn(() => false);
  free = vi.fn();
  len = 0;
  is_empty = true;
  storage_mode = 'full';
  constructor(readonly dimension: number) {}
  static new_with_mode(dimension: number): MockVectorStore {
    return new MockVectorStore(dimension);
  }
}

interface StoredEdge { id: string; source: string; target: string; label: string; properties?: unknown }

class MockWasmDatabase {
  static last: MockWasmDatabase | null = null;
  edges: StoredEdge[] = [];
  matchRows: unknown[] = [];
  matchError: string | null = null;
  attachStore = vi.fn();
  delete_collection = vi.fn();
  executeQuery = vi.fn();
  free = vi.fn();
  addEdge = vi.fn((_name: string, json: string) => {
    // As the binding reads it: ids arrive as strings or numbers and leave as
    // decimal strings, as REST's EdgeResponse.
    const sent = JSON.parse(json) as { id?: string; source: string; target: string; label: string; properties?: unknown };
    if (sent.target === '99') { throw "Node 99 not found in collection 'docs'"; }
    const edge: StoredEdge = { ...sent, id: sent.id ?? '7' };
    this.edges.push(edge);
    return edge.id;
  });
  getEdges = vi.fn((_name: string, label?: string) =>
    JSON.stringify(this.edges.filter((e) => label === undefined || e.label === label))
  );
  removeEdge = vi.fn((_name: string, id: string) => {
    const before = this.edges.length;
    this.edges = this.edges.filter((e) => e.id !== id);
    return before !== this.edges.length;
  });
  edgeCount = vi.fn(() => this.edges.length);
  removePoints = vi.fn((_name: string, ids: BigUint64Array) => {
    const gone = new Set(Array.from(ids, String));
    this.edges = this.edges.filter((e) => !gone.has(e.source) && !gone.has(e.target));
    return ids.length;
  });
  executeMatch = vi.fn((_sql: string, _params: string | null, _vector?: Float32Array, _threshold?: number) => {
    if (this.matchError) { throw this.matchError; }
    return JSON.stringify(this.matchRows);
  });
  constructor() {
    MockWasmDatabase.last = this;
  }
}

const binding = {
  default: vi.fn(() => Promise.resolve()),
  VectorStore: MockVectorStore,
  WasmDatabase: MockWasmDatabase,
  VelesQL: { parse: vi.fn() },
  velesqlContractVersion: vi.fn(() => '3.0.0'),
};
let wasmModule: Record<string, unknown> = binding;

vi.mock('@wiscale/velesdb-wasm', () => ({
  get default() { return wasmModule.default; },
  get VectorStore() { return wasmModule.VectorStore; },
  get WasmDatabase() { return wasmModule.WasmDatabase; },
  get VelesQL() { return wasmModule.VelesQL; },
  get velesqlContractVersion() { return wasmModule.velesqlContractVersion; },
}));
vi.mock('../src/backends/wasm-node-loader', () => ({
  isNodeRuntime: () => false,
  loadWasmBytesNode: vi.fn(() => Promise.resolve(new Uint8Array(0))),
}));

async function backendWithDocs(): Promise<WasmBackend> {
  const backend = new WasmBackend();
  await backend.init();
  await backend.createCollection('docs', { dimension: 2, metric: 'cosine' });
  return backend;
}

async function expectCode(promise: Promise<unknown>, code: string): Promise<void> {
  await expect(promise).rejects.toBeInstanceOf(VelesDBError);
  await expect(promise).rejects.toMatchObject({ code });
}

describe('WASM backend graph operations through the WasmDatabase edge API', () => {
  beforeEach(() => {
    MockWasmDatabase.last = null;
    wasmModule = binding;
  });

  it('when an edge is added, then it reaches the binding as data and getEdges lists it', async () => {
    const backend = await backendWithDocs();

    await backend.addEdge('docs', { id: 10, source: 1, target: 2, label: 'CITES', properties: { weight: 0.5 } });

    const db = MockWasmDatabase.last!;
    expect(JSON.parse(db.addEdge.mock.calls[0]![1] as string)).toEqual({
      id: '10', source: '1', target: '2', label: 'CITES', properties: { weight: 0.5 },
    });
    expect(await backend.getEdges('docs')).toEqual([
      { id: '10', source: '1', target: '2', label: 'CITES', properties: { weight: 0.5 } },
    ]);
    expect(await backend.getEdgeCount('docs')).toBe(1);
  });

  it('when getEdges filters by label, then the label reaches the binding', async () => {
    const backend = await backendWithDocs();
    await backend.addEdge('docs', { id: 10, source: 1, target: 2, label: 'CITES' });
    await backend.addEdge('docs', { id: 11, source: 1, target: 3, label: 'LINKS' });

    const links = await backend.getEdges('docs', { label: 'LINKS' });

    expect(links.map((e) => e.id)).toEqual(['11']);
  });

  it('when an edge is removed, then removeEdge reports it once', async () => {
    const backend = await backendWithDocs();
    await backend.addEdge('docs', { id: 10, source: 1, target: 2, label: 'CITES' });

    expect(await backend.removeEdge('docs', 10)).toBe(true);
    expect(await backend.removeEdge('docs', 10)).toBe(false);
  });

  it('when ids are past 2^53, then they reach the binding exactly, as decimal strings', async () => {
    const backend = await backendWithDocs();
    const hashed = '9007199254740993';

    await backend.addEdge('docs', { id: hashed, source: '9007199254740995', target: 2, label: 'CITES' });

    const db = MockWasmDatabase.last!;
    expect(JSON.parse(db.addEdge.mock.calls[0]![1] as string)).toMatchObject({
      id: hashed, source: '9007199254740995', target: '2',
    });
    expect(await backend.removeEdge('docs', hashed)).toBe(true);
    expect(db.removeEdge).toHaveBeenCalledWith('docs', hashed);
  });

  it('when an id is not a non-negative integer, then the call is BAD_REQUEST, never a hashed id', async () => {
    const backend = await backendWithDocs();

    await expectCode(backend.addEdge('docs', { id: 'doc-a', source: 1, target: 2, label: 'R' }), 'BAD_REQUEST');
    await expectCode(backend.addEdge('docs', { id: 1, source: -1, target: 2, label: 'R' }), 'BAD_REQUEST');
    await expectCode(backend.removeEdge('docs', 1.5), 'BAD_REQUEST');
    expect(MockWasmDatabase.last!.addEdge).not.toHaveBeenCalled();
  });

  it('when points are deleted, then the database removes them with their edges', async () => {
    const backend = await backendWithDocs();
    await backend.addEdge('docs', { id: 10, source: 1, target: 2, label: 'CITES' });
    await backend.addEdge('docs', { id: 11, source: 2, target: 3, label: 'CITES' });

    await backend.delete('docs', 1);
    await backend.bulkDelete('docs', [3]);

    const db = MockWasmDatabase.last!;
    expect(db.removePoints.mock.calls.map(([name, ids]) => [name, Array.from(ids)])).toEqual([
      ['docs', [1n]],
      ['docs', [3n]],
    ]);
    expect(await backend.getEdgeCount('docs')).toBe(0);
  });

  it('when an endpoint is not a node, then addEdge is NOT_FOUND, as on REST', async () => {
    const backend = await backendWithDocs();

    await expectCode(backend.addEdge('docs', { id: 10, source: 1, target: 99, label: 'CITES' }), 'NOT_FOUND');
  });

  it('when the collection does not exist, then every graph operation is NOT_FOUND', async () => {
    const backend = await backendWithDocs();

    await expectCode(backend.addEdge('ghost', { id: 1, source: 1, target: 2, label: 'R' }), 'NOT_FOUND');
    await expectCode(backend.getEdges('ghost'), 'NOT_FOUND');
    await expectCode(backend.removeEdge('ghost', 1), 'NOT_FOUND');
    await expectCode(backend.getEdgeCount('ghost'), 'NOT_FOUND');
    await expectCode(backend.matchQuery('ghost', 'MATCH (a) RETURN a'), 'NOT_FOUND');
  });
});

describe('WASM backend matchQuery through executeMatch', () => {
  beforeEach(() => {
    MockWasmDatabase.last = null;
    wasmModule = binding;
  });

  it('when a MATCH runs, then the collection is named by _collection and the rows come back REST-shaped', async () => {
    const backend = await backendWithDocs();
    const db = MockWasmDatabase.last!;
    // The binding writes each binding's node id as its decimal string, as REST /match.
    db.matchRows = [{ bindings: { a: '1', b: '9007199254740993' }, depth: 1, projected: { 'b.title': 'doc 2' } }];

    const response = await backend.matchQuery('docs', 'MATCH (a)-[:CITES]->(b) RETURN b.title', { min: 1 });

    expect(JSON.parse(db.executeMatch.mock.calls[0]![1] as string)).toEqual({ min: 1, _collection: 'docs' });
    expect(response.results).toEqual([
      { bindings: { a: '1', b: '9007199254740993' }, depth: 1, projected: { 'b.title': 'doc 2' } },
    ]);
    expect(response.count).toBe(1);
    expect(response.meta).toEqual({ velesqlContractVersion: '3.0.0' });
    expect(Number.isInteger(response.tookMs)).toBe(true);
  });

  it('when the binding refuses the statement, then matchQuery is BAD_REQUEST with its message', async () => {
    const backend = await backendWithDocs();
    MockWasmDatabase.last!.matchError = 'executeMatch runs MATCH statements only; use executeQuery';

    await expectCode(backend.matchQuery('docs', 'SELECT * FROM docs'), 'BAD_REQUEST');
  });

  it('when a query vector and threshold are passed, then they reach executeMatch and rows keep their score', async () => {
    const backend = await backendWithDocs();
    const db = MockWasmDatabase.last!;
    db.matchRows = [{ bindings: { d: '3' }, depth: 1, projected: {}, score: 0.97 }];

    const response = await backend.matchQuery('docs', 'MATCH (a)-[:R]->(d) RETURN d', undefined, {
      vector: new Float32Array([1, 0]),
      threshold: 0.7,
    });

    const [, , vector, threshold] = db.executeMatch.mock.calls[0]! as unknown[];
    expect(Array.from(vector as Float32Array)).toEqual([1, 0]);
    expect(threshold).toBe(0.7);
    expect(response.results[0]!.score).toBe(0.97);
  });
});

describe('WASM backend graph operations on a binding without the edge API', () => {
  it('then they keep refusing with NOT_SUPPORTED', async () => {
    class OldDatabase {
      attachStore = vi.fn();
      delete_collection = vi.fn();
      executeQuery = vi.fn();
      free = vi.fn();
    }
    wasmModule = { ...binding, WasmDatabase: OldDatabase };
    const backend = await backendWithDocs();

    await expectCode(backend.addEdge('docs', { id: 1, source: 1, target: 2, label: 'R' }), 'NOT_SUPPORTED');
    await expectCode(backend.matchQuery('docs', 'MATCH (a) RETURN a'), 'NOT_SUPPORTED');
  });
});
