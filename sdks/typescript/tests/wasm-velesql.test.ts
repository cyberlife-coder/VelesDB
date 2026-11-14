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
import { VelesDBError } from '../src/types';

const calls: string[] = [];

class MockVectorStore {
  insert = vi.fn();
  insert_with_payload = vi.fn();
  query = vi.fn(() => [{ id: 1, score: 0.9 }]);
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
  nextError: string | null = null;
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

const moduleWithExecutor = {
  default: vi.fn(() => Promise.resolve()),
  VectorStore: MockVectorStore,
  WasmDatabase: MockWasmDatabase,
};
let wasmModule: Record<string, unknown> = moduleWithExecutor;

vi.mock('@wiscale/velesdb-wasm', () => ({
  get default() { return wasmModule.default; },
  get VectorStore() { return wasmModule.VectorStore; },
  get WasmDatabase() { return wasmModule.WasmDatabase; },
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

  it('when params hold a Float32Array, then they reach the executor as a JSON array', async () => {
    const backend = await backendWithDocs();
    const sql = 'SELECT * FROM docs WHERE vector NEAR $v AND category = $c LIMIT 5';

    await backend.query('docs', sql, { v: new Float32Array([1, 0]), c: 'tech' });

    const json = MockWasmDatabase.last!.executeQuery.mock.calls[0]![1] as string;
    expect(JSON.parse(json)).toEqual({ v: [1, 0], c: 'tech' });
  });

  it('when the query is the pure NEAR shape, then it keeps the store.query fast path', async () => {
    const backend = await backendWithDocs();

    await backend.query('docs', 'SELECT * FROM docs WHERE vector NEAR $v LIMIT 3', { v: [1, 0] });

    expect(MockWasmDatabase.last!.executeQuery).not.toHaveBeenCalled();
  });

  it('when the SELECT targets another collection, then it is refused as BAD_REQUEST', async () => {
    const backend = await backendWithDocs();

    await expect(
      backend.query('docs', "SELECT * FROM other WHERE category = 'x'")
    ).rejects.toMatchObject({ code: 'BAD_REQUEST' });
  });

  it('when the executor rejects the statement, then its message surfaces as BAD_REQUEST', async () => {
    const backend = await backendWithDocs();
    MockWasmDatabase.last!.nextError = 'VelesQL parse error at position 7';

    const error = await backend
      .query('docs', 'SELECT * FROM docs WHERE ???')
      .catch((e: unknown) => e);

    expect(error).toBeInstanceOf(VelesDBError);
    expect(error).toMatchObject({ code: 'BAD_REQUEST' });
    expect((error as Error).message).toContain('VelesQL parse error at position 7');
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
