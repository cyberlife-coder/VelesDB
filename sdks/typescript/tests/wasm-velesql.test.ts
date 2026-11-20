/**
 * BDD tests: the WASM backend runs VelesQL through a `WasmDatabase` its
 * collections are attached to.
 *
 * Before: the backend owned bare `VectorStore`s that no VelesQL executor
 * could see, so `query()` answered only the pure `NEAR` shape and threw
 * `NOT_SUPPORTED` on every `WHERE`. A binding that exposes
 * `WasmDatabase.attachStore` (after velesdb-wasm 8.0.0) shares each store
 * with an executor; 8.0.0 — the floor the SDK's dependency range admits —
 * has none, and keeps the old behaviour.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { WasmBackend } from '../src/backends/wasm';
import { CollectionExistsError, InvalidCollectionNameError, QueryError, QueryParameterError } from '../src/errors';
import { VelesDBError } from '../src/types';

const calls: string[] = [];

class MockVectorStore {
  insert = vi.fn();
  insert_with_payload = vi.fn();
  query = vi.fn(() => [{ id: 1, score: 0.9 }]);
  sparse_insert = vi.fn();
  sparse_remove = vi.fn(() => false);
  free = vi.fn(() => calls.push('store.free'));
  len = 0;
  is_empty = true;
  storage_mode = 'full';
  constructor(readonly dimension: number) {}
  static new_with_mode(dimension: number): MockVectorStore {
    return new MockVectorStore(dimension);
  }
  static new_metadata_only(): MockVectorStore {
    return new MockVectorStore(0);
  }
}

class MockQueryResult {
  constructor(private readonly rows: unknown[]) {}
  get kind(): string {
    return 'rows';
  }
  get rowCount(): number {
    return this.rows.length;
  }
  get rowsJson(): string {
    return JSON.stringify(this.rows);
  }
  free = vi.fn();
}

class MockWasmDatabase {
  static last: MockWasmDatabase | null = null;
  attached = new Map<string, MockVectorStore>();
  nextRows: unknown[] = [{ id: 1, category: 'tech' }];
  nextError: unknown = null;
  attachStore = vi.fn((name: string, store: MockVectorStore) => {
    this.attached.set(name, store);
  });
  delete_collection = vi.fn((name: string) => {
    calls.push(`db.delete_collection:${name}`);
    this.attached.delete(name);
  });
  executeQuery = vi.fn((_sql: string, _params?: string) => {
    if (this.nextError) { throw this.nextError; }
    return new MockQueryResult(this.nextRows);
  });
  free = vi.fn();
  constructor() {
    MockWasmDatabase.last = this;
  }
}

/**
 * What the binding's own parser reports for each statement the tests send:
 * the SDK routes on these flags, never on the statement's text.
 */
const PARSED: Record<string, { isSelect: boolean; isAggregation?: boolean; collectionName?: string }> = {
  "SELECT * FROM docs WHERE category = 'tech' LIMIT 10": { isSelect: true, isAggregation: false, collectionName: 'docs' },
  'SELECT * FROM docs WHERE vector NEAR $v LIMIT 3': { isSelect: true, isAggregation: false, collectionName: 'docs' },
  'SELECT * FROM docs WHERE vector NEAR $v AND category = $c LIMIT 5': { isSelect: true, isAggregation: false, collectionName: 'docs' },
"SELECT * FROM docs WHERE vector SPARSE_NEAR {7: 1.0} LIMIT 5": { isSelect: true, isAggregation: false, collectionName: 'docs' },
  'SELECT * FROM docs WHERE vector NEAR $v AND vector SPARSE_NEAR $s LIMIT 5': { isSelect: true, isAggregation: false, collectionName: 'docs' },
  "SELECT * FROM other WHERE category = 'x'": { isSelect: true, isAggregation: false, collectionName: 'other' },
  "SELECT * FROM docs WHERE title = 'Having fun'": { isSelect: true, isAggregation: false, collectionName: 'docs' },
  'SELECT "from archive" FROM docs': { isSelect: true, isAggregation: false, collectionName: 'docs' },
  'SELECT category, COUNT(*) FROM docs GROUP BY category': { isSelect: true, isAggregation: true, collectionName: 'docs' },
  'MATCH (a)-[:R]->(b) RETURN a': { isSelect: false, isAggregation: false },
  'INSERT INTO docs (id, vector) VALUES (1, $v)': { isSelect: false, isAggregation: false, collectionName: 'docs' },
};

const parseFree = vi.fn();
const MockVelesQL = {
  parse: vi.fn((sql: string) => {
    const flags = PARSED[sql];
    if (!flags) { throw `VelesQL parse error in: ${sql}`; }
    return { ...flags, free: parseFree };
  }),
};

const moduleWithExecutor = {
  default: vi.fn(() => Promise.resolve()),
  VectorStore: MockVectorStore,
  WasmDatabase: MockWasmDatabase,
  VelesQL: MockVelesQL,
};
let wasmModule: Record<string, unknown> = moduleWithExecutor;

vi.mock('@wiscale/velesdb-wasm', () => ({
  get default() { return wasmModule.default; },
  get VectorStore() { return wasmModule.VectorStore; },
  get WasmDatabase() { return wasmModule.WasmDatabase; },
  get VelesQL() { return wasmModule.VelesQL; },
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

describe('WASM backend VelesQL through an attached WasmDatabase', () => {
  beforeEach(() => {
    calls.length = 0;
    MockWasmDatabase.last = null;
    wasmModule = moduleWithExecutor;
  });

  it('given a binding with attachStore, when a collection is created, then its store is attached under its name', async () => {
    await backendWithDocs();

    const db = MockWasmDatabase.last!;
    expect(db.attachStore).toHaveBeenCalledTimes(1);
    expect(db.attached.has('docs')).toBe(true);
  });

  it('when the name is not a valid collection name, then it is refused with InvalidCollectionNameError before any store is built', async () => {
    const backend = new WasmBackend();
    await backend.init();
    const build = vi.spyOn(MockVectorStore, 'new_with_mode');

    await expect(backend.createCollection('bad.name', { dimension: 2 }))
      .rejects.toBeInstanceOf(InvalidCollectionNameError);

    expect(build).not.toHaveBeenCalled();
    expect(MockWasmDatabase.last!.attachStore).not.toHaveBeenCalled();
    build.mockRestore();
  });

  it('when the binding refuses to attach the store, then the store is freed and the refusal keeps its VELES code', async () => {
    const backend = new WasmBackend();
    await backend.init();
    // velesdb-wasm throws an `Error` whose `code` property is core's code.
    const refusal = Object.assign(new Error('[VELES-034] Invalid collection name'), { code: 'VELES-034' });
    MockWasmDatabase.last!.attachStore.mockImplementationOnce(() => { throw refusal; });

    const error = await backend.createCollection('docs', { dimension: 2 }).catch((e: unknown) => e);

    expect(error).toBeInstanceOf(InvalidCollectionNameError);
    expect((error as InvalidCollectionNameError).message).toBe('[VELES-034] Invalid collection name');
    expect(calls).toEqual(['store.free']);
    expect(await backend.getCollection('docs')).toBeNull();
  });

  it("when the binding refuses to attach a taken name, then the store is freed and the refusal is core's CollectionExistsError", async () => {
    const backend = new WasmBackend();
    await backend.init();
    MockWasmDatabase.last!.attachStore.mockImplementationOnce(() => {
      throw Object.assign(new Error("[VELES-001] Collection 'docs' already exists"), { code: 'VELES-001' });
    });

    const error = await backend.createCollection('docs', { dimension: 2 }).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(CollectionExistsError);
    expect(error).toMatchObject({ code: 'VELES-001', message: "[VELES-001] Collection 'docs' already exists" });
    expect(calls).toEqual(['store.free']);
    expect(await backend.getCollection('docs')).toBeNull();
  });

  it('when the collection is deleted, then the database drops it before the store is freed', async () => {
    const backend = await backendWithDocs();

    await backend.deleteCollection('docs');

    expect(calls).toEqual(['db.delete_collection:docs', 'store.free']);
  });

  it('when query() runs a SELECT with WHERE, then the executor answers with REST-shaped rows', async () => {
    const backend = await backendWithDocs();
    const sql = "SELECT * FROM docs WHERE category = 'tech' LIMIT 10";

    const response = await backend.query('docs', sql);

    expect(MockWasmDatabase.last!.executeQuery).toHaveBeenCalledWith(sql, null);
    expect(response).toEqual({
      results: [{ id: 1, category: 'tech' }],
      stats: { executionTimeMs: expect.any(Number), strategy: 'select', scannedNodes: 1 },
    });
  });

  // The executor runs SPARSE_NEAR over the store it shares: the sparse vector
  // must be in that store, under the point's id, for the query to see it.
  it('when a point carries a sparse vector, then SPARSE_NEAR runs over the attached store that holds it', async () => {
    const backend = await backendWithDocs();
    await backend.upsert('docs', { id: 3, vector: [1, 0], sparseVector: { 7: 1 } });
    const attached = MockWasmDatabase.last!.attached.get('docs')!;
    expect(attached.sparse_insert).toHaveBeenCalledTimes(1);
    expect(attached.sparse_insert.mock.calls[0]![0]).toBe(3n);
    MockWasmDatabase.last!.nextRows = [{ id: '3', score: 1 }];

    const alone = "SELECT * FROM docs WHERE vector SPARSE_NEAR {7: 1.0} LIMIT 5";
    const fused = 'SELECT * FROM docs WHERE vector NEAR $v AND vector SPARSE_NEAR $s LIMIT 5';
    expect(await backend.query('docs', alone)).toMatchObject({ results: [{ id: '3', score: 1 }] });
    await backend.query('docs', fused, { v: [1, 0], s: { 7: 1 } });

    const executed = MockWasmDatabase.last!.executeQuery.mock.calls;
    expect(executed.map((c) => c[0])).toEqual([alone, fused]);
    expect(JSON.parse(executed[1]![1] as string)).toEqual({ v: [1, 0], s: { 7: 1 } });
  });

  it('when params hold a Float32Array, then they reach the executor as a JSON array', async () => {
    const backend = await backendWithDocs();
    const sql = 'SELECT * FROM docs WHERE vector NEAR $v AND category = $c LIMIT 5';

    await backend.query('docs', sql, { v: new Float32Array([1, 0]), c: 'tech' });

    const json = MockWasmDatabase.last!.executeQuery.mock.calls[0]![1] as string;
    expect(JSON.parse(json)).toEqual({ v: [1, 0], c: 'tech' });
  });

  it('when the query is the pure NEAR shape, then the executor answers it too, with the rows a WHERE gets', async () => {
    const backend = await backendWithDocs();
    const sql = 'SELECT * FROM docs WHERE vector NEAR $v LIMIT 3';

    const response = await backend.query('docs', sql, { v: [1, 0] });

    expect(MockWasmDatabase.last!.executeQuery).toHaveBeenCalledWith(sql, JSON.stringify({ v: [1, 0] }));
    expect(MockWasmDatabase.last!.attached.get('docs')!.query).not.toHaveBeenCalled();
    expect(response).toMatchObject({ results: [{ id: 1, category: 'tech' }], stats: { strategy: 'select' } });
  });

  it('when the SELECT targets another collection, then it is refused as BAD_REQUEST', async () => {
    const backend = await backendWithDocs();

    await expect(
      backend.query('docs', "SELECT * FROM other WHERE category = 'x'")
    ).rejects.toMatchObject({ code: 'BAD_REQUEST' });
  });

  it('when the binding cannot parse the statement, then its message surfaces as BAD_REQUEST', async () => {
    const backend = await backendWithDocs();

    const error = await backend
      .query('docs', 'SELECT * FROM docs WHERE ???')
      .catch((e: unknown) => e);

    expect(error).toBeInstanceOf(VelesDBError);
    expect(error).toMatchObject({ code: 'BAD_REQUEST' });
    expect((error as Error).message).toContain('VelesQL parse error');
    expect(MockWasmDatabase.last!.executeQuery).not.toHaveBeenCalled();
  });

  it('when the executor rejects the statement, then its message surfaces as BAD_REQUEST', async () => {
    const backend = await backendWithDocs();
    MockWasmDatabase.last!.nextError = 'Unknown column at position 7';

    const error = await backend
      .query('docs', "SELECT * FROM docs WHERE category = 'tech' LIMIT 10")
      .catch((e: unknown) => e);

    expect(error).toMatchObject({ code: 'BAD_REQUEST' });
    expect((error as Error).message).toContain('Unknown column at position 7');
  });

  it('when the executor refuses an unbound $param, then it surfaces as QueryParameterError with core\'s VELES-038', async () => {
    const backend = await backendWithDocs();
    // velesdb-wasm throws an `Error` whose `code` property is core's code,
    // its message core's text.
    MockWasmDatabase.last!.nextError = Object.assign(
      new Error('[VELES-038] Query parameter $c: not bound'),
      { code: 'VELES-038' }
    );

    const error = await backend
      .query('docs', 'SELECT * FROM docs WHERE vector NEAR $v AND category = $c LIMIT 5', { v: [1, 0] })
      .catch((e: unknown) => e);

    expect(error).toBeInstanceOf(QueryParameterError);
    expect(error).toBeInstanceOf(QueryError);
    expect(error).toMatchObject({ code: 'VELES-038', message: '[VELES-038] Query parameter $c: not bound' });
  });

  it.each([
    ['an aggregate keyword inside a string literal', "SELECT * FROM docs WHERE title = 'Having fun'"],
    ['"from" inside a quoted column name', 'SELECT "from archive" FROM docs'],
  ])('when the statement has %s, then the parser routes it to the executor', async (_what, sql) => {
    const backend = await backendWithDocs();

    await backend.query('docs', sql);

    expect(MockWasmDatabase.last!.executeQuery).toHaveBeenCalledWith(sql, null);
  });

  it('when a statement was parsed, then its parse handle is freed', async () => {
    const backend = await backendWithDocs();
    parseFree.mockClear();

    await backend.query('docs', "SELECT * FROM docs WHERE category = 'tech' LIMIT 10");

    expect(parseFree).toHaveBeenCalledTimes(1);
  });

  it('given a binding whose parser predates isAggregation, when query() runs a WHERE, then it keeps the previous behaviour', async () => {
    wasmModule = {
      ...moduleWithExecutor,
      VelesQL: { parse: () => ({ isSelect: true, collectionName: 'docs', free: vi.fn() }) },
    };
    const backend = await backendWithDocs();

    await expect(
      backend.query('docs', "SELECT * FROM docs WHERE category = 'tech' LIMIT 10")
    ).rejects.toMatchObject({ code: 'NOT_SUPPORTED' });
  });

  it.each([
    ['an aggregation', 'SELECT category, COUNT(*) FROM docs GROUP BY category'],
    ['a MATCH', 'MATCH (a)-[:R]->(b) RETURN a'],
    ['a mutation', "INSERT INTO docs (id, vector) VALUES (1, $v)"],
  ])('when query() gets %s, then it stays NOT_SUPPORTED until its own mapping lands', async (_what, sql) => {
    const backend = await backendWithDocs();

    await expect(backend.query('docs', sql)).rejects.toMatchObject({ code: 'NOT_SUPPORTED' });
    expect(MockWasmDatabase.last!.executeQuery).not.toHaveBeenCalled();
  });

  it('when the backend closes, then the database is freed', async () => {
    const backend = await backendWithDocs();
    const db = MockWasmDatabase.last!;

    await backend.close();

    expect(db.free).toHaveBeenCalledTimes(1);
  });

  it('given velesdb-wasm 8.0.0 (no WasmDatabase), when query() runs a WHERE, then it is NOT_SUPPORTED as before', async () => {
    wasmModule = { default: vi.fn(() => Promise.resolve()), VectorStore: MockVectorStore };
    const backend = await backendWithDocs();

    await expect(
      backend.query('docs', "SELECT * FROM docs WHERE category = 'tech'")
    ).rejects.toMatchObject({ code: 'NOT_SUPPORTED' });
  });
});
