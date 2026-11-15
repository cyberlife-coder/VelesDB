/**
 * REST Backend tests for the maintenance / ops routes exposed by the parity
 * audit: graph/edges/batch, vacuum, compact, locality/reorder, diagnostics,
 * health, ready. Fakes reproduce the observed server wire shapes.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { RestBackend } from '../src/backends/rest';
import { VelesDB } from '../src/client';
import { VelesDBError } from '../src/types';

const mockFetch = vi.fn();
global.fetch = mockFetch;

async function initBackend(backend: RestBackend): Promise<void> {
  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: () => Promise.resolve({ status: 'ok', version: '8.2.0' }),
  });
  await backend.init();
  vi.clearAllMocks();
}

describe('RestBackend maintenance routes', () => {
  let backend: RestBackend;

  beforeEach(() => {
    vi.clearAllMocks();
    backend = new RestBackend('http://localhost:8080', 'test-api-key');
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('addEdgesBatch posts the batch and maps { added }', async () => {
    await initBackend(backend);
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ added: 2 }),
    });

    const res = await backend.addEdgesBatch('kg', [
      { id: 1, source: 10, target: 11, label: 'KNOWS' },
      { id: 2, source: 11, target: 12, label: 'LIKES', properties: { since: 2024 } },
    ]);

    expect(res).toEqual({ added: 2 });
    expect(mockFetch).toHaveBeenCalledWith(
      'http://localhost:8080/collections/kg/graph/edges/batch',
      expect.objectContaining({
        method: 'POST',
        body: JSON.stringify({
          edges: [
            { id: 1, source: 10, target: 11, label: 'KNOWS', properties: {} },
            { id: 2, source: 11, target: 12, label: 'LIKES', properties: { since: 2024 } },
          ],
        }),
      })
    );
  });

  it('addEdgesBatch defaults added to 0 when the body is empty', async () => {
    await initBackend(backend);
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({}),
    });
    const res = await backend.addEdgesBatch('kg', []);
    expect(res).toEqual({ added: 0 });
  });

  it('vacuumCollection maps compacted_entries', async () => {
    await initBackend(backend);
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({ message: 'Index vacuumed', collection: 'docs', compacted_entries: 42 }),
    });

    const res = await backend.vacuumCollection('docs');
    expect(res).toEqual({ message: 'Index vacuumed', collection: 'docs', compactedEntries: 42 });
    expect(mockFetch).toHaveBeenCalledWith(
      'http://localhost:8080/collections/docs/vacuum',
      expect.objectContaining({ method: 'POST' })
    );
  });

  it('compactCollection maps bytes_reclaimed', async () => {
    await initBackend(backend);
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({ message: 'Storage compacted', collection: 'docs', bytes_reclaimed: 2048 }),
    });

    const res = await backend.compactCollection('docs');
    expect(res).toEqual({ message: 'Storage compacted', collection: 'docs', bytesReclaimed: 2048 });
    expect(mockFetch).toHaveBeenCalledWith(
      'http://localhost:8080/collections/docs/compact',
      expect.objectContaining({ method: 'POST' })
    );
  });

  it('reorderForLocality posts the reorder route', async () => {
    await initBackend(backend);
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ message: 'Locality reordered', collection: 'docs' }),
    });

    const res = await backend.reorderForLocality('docs');
    expect(res).toEqual({ message: 'Locality reordered', collection: 'docs' });
    expect(mockFetch).toHaveBeenCalledWith(
      'http://localhost:8080/collections/docs/locality/reorder',
      expect.objectContaining({ method: 'POST' })
    );
  });

  it('collectionDiagnostics maps the snake_case wire', async () => {
    await initBackend(backend);
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          has_vectors: true,
          search_ready: true,
          dimension_configured: true,
          point_count: 100,
          index_health: 'needs_rebuild',
          index_health_detail: 'tombstone ratio above threshold',
        }),
    });

    const res = await backend.collectionDiagnostics('docs');
    expect(res).toEqual({
      hasVectors: true,
      searchReady: true,
      dimensionConfigured: true,
      pointCount: 100,
      indexHealth: 'needs_rebuild',
      indexHealthDetail: 'tombstone ratio above threshold',
    });
    expect(mockFetch).toHaveBeenCalledWith(
      'http://localhost:8080/collections/docs/diagnostics',
      expect.objectContaining({ method: 'GET' })
    );
  });

  it('healthCheck returns the probe body', async () => {
    await initBackend(backend);
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ status: 'ok', version: '8.2.0' }),
    });

    const res = await backend.healthCheck();
    expect(res).toEqual({ status: 'ok', version: '8.2.0' });
    expect(mockFetch).toHaveBeenCalledWith(
      'http://localhost:8080/health',
      expect.objectContaining({ method: 'GET' })
    );
  });

  it('readyCheck throws a typed SERVICE_UNAVAILABLE while not ready', async () => {
    await initBackend(backend);
    // GET is idempotent, so the transport retries 503 up to maxRetries (2):
    // the last attempt's error is the one returned.
    for (let i = 0; i < 3; i++) {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        status: 503,
        headers: { get: () => null },
        json: () => Promise.resolve({ status: 'not_ready', version: '8.2.0' }),
      });
    }

    await expect(backend.readyCheck()).rejects.toMatchObject({ code: 'SERVICE_UNAVAILABLE' });
  });
});

describe('VelesDB client maintenance delegation', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  function clientWithBackend(backend: unknown): VelesDB {
    const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
    (db as unknown as { backend: unknown }).backend = backend;
    (db as unknown as { initialized: boolean }).initialized = true;
    return db;
  }

  it('refuses with NOT_SUPPORTED when the backend lacks the route', async () => {
    const db = clientWithBackend({ capabilities: () => ({}) });
    await expect(db.vacuumCollection('docs')).rejects.toMatchObject({ code: 'NOT_SUPPORTED' });
    await expect(db.compactCollection('docs')).rejects.toMatchObject({ code: 'NOT_SUPPORTED' });
    await expect(db.reorderForLocality('docs')).rejects.toMatchObject({ code: 'NOT_SUPPORTED' });
    await expect(db.collectionDiagnostics('docs')).rejects.toMatchObject({ code: 'NOT_SUPPORTED' });
    await expect(db.healthCheck()).rejects.toMatchObject({ code: 'NOT_SUPPORTED' });
    await expect(db.readyCheck()).rejects.toMatchObject({ code: 'NOT_SUPPORTED' });
    await expect(
      db.addEdgesBatch('kg', [{ id: 1, source: 1, target: 2, label: 'KNOWS' }])
    ).rejects.toBeInstanceOf(VelesDBError);
  });

  it('delegates every maintenance method to the backend when present', async () => {
    const ops = {
      capabilities: () => ({}),
      vacuumCollection: vi.fn().mockResolvedValue({ message: 'ok', collection: 'docs', compactedEntries: 1 }),
      compactCollection: vi.fn().mockResolvedValue({ message: 'ok', collection: 'docs', bytesReclaimed: 8 }),
      reorderForLocality: vi.fn().mockResolvedValue({ message: 'ok', collection: 'docs' }),
      collectionDiagnostics: vi.fn().mockResolvedValue({ indexHealth: 'healthy', pointCount: 1 }),
      healthCheck: vi.fn().mockResolvedValue({ status: 'ok', version: '8.2.0' }),
      readyCheck: vi.fn().mockResolvedValue({ status: 'ready', version: '8.2.0' }),
      addEdgesBatch: vi.fn().mockResolvedValue({ added: 3 }),
    };
    const db = clientWithBackend(ops);

    expect((await db.vacuumCollection('docs')).compactedEntries).toBe(1);
    expect((await db.compactCollection('docs')).bytesReclaimed).toBe(8);
    expect((await db.reorderForLocality('docs')).collection).toBe('docs');
    expect((await db.collectionDiagnostics('docs')).indexHealth).toBe('healthy');
    expect((await db.healthCheck()).status).toBe('ok');
    expect((await db.readyCheck()).status).toBe('ready');
    expect((await db.addEdgesBatch('kg', [{ id: 1, source: 1, target: 2, label: 'KNOWS' }])).added).toBe(3);

    expect(ops.vacuumCollection).toHaveBeenCalledWith('docs');
    expect(ops.compactCollection).toHaveBeenCalledWith('docs');
    expect(ops.reorderForLocality).toHaveBeenCalledWith('docs');
    expect(ops.collectionDiagnostics).toHaveBeenCalledWith('docs');
    expect(ops.addEdgesBatch).toHaveBeenCalledWith('kg', [{ id: 1, source: 1, target: 2, label: 'KNOWS' }]);
  });

  it('validates the collection name and edge shape before delegating', async () => {
    const db = clientWithBackend({ capabilities: () => ({}), addEdgesBatch: vi.fn() });
    await expect(db.vacuumCollection('')).rejects.toMatchObject({ name: 'ValidationError' });
    await expect(db.addEdgesBatch('', [{ id: 1, source: 1, target: 2, label: 'KNOWS' }])).rejects.toMatchObject({
      name: 'ValidationError',
    });
    await expect(db.addEdgesBatch('kg', 'nope' as never)).rejects.toMatchObject({ name: 'ValidationError' });
    await expect(
      db.addEdgesBatch('kg', [{ id: 1, source: 1, target: 2 } as never])
    ).rejects.toMatchObject({ name: 'ValidationError' });
    await expect(
      db.addEdgesBatch('kg', [{ id: 1, source: { bad: true }, target: 2, label: 'KNOWS' } as never])
    ).rejects.toMatchObject({ name: 'ValidationError' });
  });
});
