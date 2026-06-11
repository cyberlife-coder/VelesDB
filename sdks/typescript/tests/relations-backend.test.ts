/**
 * Relation Endpoint Wrapper Tests (REST parity)
 *
 * Covers the four relation/TTL wrappers — `relate`, `unrelate`,
 * `getRelations`, `setTtlDurable` — through `RestBackend` with a stubbed
 * `fetch`. Asserts the HTTP method + path, the snake_case wire body, the
 * camelCase response mapping, 404/400 error routing, and the
 * string-vs-number id boundary (u64-safe ids stay verbatim in paths).
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { RestBackend } from '../src/backends/rest';
import { CollectionNotFoundError, VelesError } from '../src/errors';

const mockFetch = vi.fn();
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).fetch = mockFetch;

async function initBackend(): Promise<RestBackend> {
  const backend = new RestBackend('http://localhost:8080');
  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: () => Promise.resolve({ status: 'ok' }),
  });
  await backend.init();
  mockFetch.mockReset();
  return backend;
}

function lastCall(): { url: string; method: string; body?: Record<string, unknown> } {
  const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
  const url = call[0] as string;
  const init = call[1];
  const body = init?.body ? JSON.parse(init.body as string) : undefined;
  return { url, method: init?.method as string, body };
}

function mockReply(data: unknown): void {
  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: () => Promise.resolve(data),
  });
}

function mockErrorReply(status: number, code?: string, message?: string): void {
  mockFetch.mockResolvedValueOnce({
    ok: false,
    status,
    json: () =>
      Promise.resolve(code ? { error: { code, message } } : {}),
  });
}

describe('relate', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('POSTs to /collections/{name}/relations with snake_case body and default properties={}', async () => {
    mockReply({ edge_id: 7 });

    const result = await backend.relate('kg', {
      source: 10,
      target: 20,
      relType: 'KNOWS',
    });

    const { url, method, body } = lastCall();
    expect(method).toBe('POST');
    expect(url).toBe('http://localhost:8080/collections/kg/relations');
    expect(body).toEqual({
      source: 10,
      target: 20,
      rel_type: 'KNOWS',
      properties: {},
    });
    expect(result).toEqual({ edgeId: 7 });
  });

  it('forwards explicit properties and string ids unchanged', async () => {
    mockReply({ edge_id: 8 });

    await backend.relate('kg', {
      source: '9007199254740993',
      target: '9007199254740995',
      relType: 'CITES',
      properties: { weight: 0.5 },
    });

    const { body } = lastCall();
    expect(body).toEqual({
      source: '9007199254740993',
      target: '9007199254740995',
      rel_type: 'CITES',
      properties: { weight: 0.5 },
    });
  });

  it('preserves a string edge_id above Number.MAX_SAFE_INTEGER', async () => {
    mockReply({ edge_id: '9007199254740993' });

    const result = await backend.relate('kg', {
      source: 1,
      target: 2,
      relType: 'KNOWS',
    });

    expect(result.edgeId).toBe('9007199254740993');
  });

  it('throws CollectionNotFoundError on 404 VELES-002', async () => {
    mockErrorReply(404, 'VELES-002', "Collection 'missing' not found");

    await expect(
      backend.relate('missing', { source: 1, target: 2, relType: 'KNOWS' })
    ).rejects.toThrow(CollectionNotFoundError);
  });

  it('throws a typed VelesError on 400 responses', async () => {
    mockErrorReply(400, 'VELES-022', 'Node 99 not found');

    await expect(
      backend.relate('kg', { source: 99, target: 2, relType: 'KNOWS' })
    ).rejects.toThrow(VelesError);
  });
});

describe('unrelate', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('DELETEs /collections/{name}/relations/{edgeId} and returns true', async () => {
    mockReply({ deleted: true });

    const result = await backend.unrelate('kg', 42);

    const { url, method } = lastCall();
    expect(method).toBe('DELETE');
    expect(url).toBe('http://localhost:8080/collections/kg/relations/42');
    expect(result).toBe(true);
  });

  it('keeps a u64 string edge id verbatim in the path', async () => {
    mockReply({ deleted: true });

    await backend.unrelate('kg', '18446744073709551615');

    expect(lastCall().url).toBe(
      'http://localhost:8080/collections/kg/relations/18446744073709551615'
    );
  });

  it('returns false on 404 VELES-020 (edge not found)', async () => {
    mockErrorReply(404, 'VELES-020', 'Edge 42 not found');

    await expect(backend.unrelate('kg', 42)).resolves.toBe(false);
  });

  it('returns false on a code-less 404 (legacy NOT_FOUND)', async () => {
    mockErrorReply(404);

    await expect(backend.unrelate('kg', 42)).resolves.toBe(false);
  });

  it('throws CollectionNotFoundError on 404 VELES-002', async () => {
    mockErrorReply(404, 'VELES-002', "Collection 'missing' not found");

    await expect(backend.unrelate('missing', 42)).rejects.toThrow(
      CollectionNotFoundError
    );
  });
});

describe('getRelations', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('GETs /collections/{name}/points/{id}/relations and maps snake_case edges', async () => {
    mockReply({
      edges: [
        {
          id: 7,
          source: 10,
          target: 20,
          rel_type: 'KNOWS',
          properties: { since: 2024 },
        },
      ],
      count: 1,
    });

    const result = await backend.getRelations('kg', 10);

    const { url, method } = lastCall();
    expect(method).toBe('GET');
    expect(url).toBe(
      'http://localhost:8080/collections/kg/points/10/relations'
    );
    expect(result).toEqual({
      edges: [
        {
          id: 7,
          source: 10,
          target: 20,
          relType: 'KNOWS',
          properties: { since: 2024 },
        },
      ],
      count: 1,
    });
  });

  it('preserves string ids above Number.MAX_SAFE_INTEGER in path and response', async () => {
    const bigId = '9007199254740993';
    mockReply({
      edges: [{ id: bigId, source: bigId, target: 2, rel_type: 'KNOWS' }],
      count: 1,
    });

    const result = await backend.getRelations('kg', bigId);

    expect(lastCall().url).toBe(
      `http://localhost:8080/collections/kg/points/${bigId}/relations`
    );
    expect(result.edges[0]!.id).toBe(bigId);
    expect(result.edges[0]!.source).toBe(bigId);
    expect(result.edges[0]!.properties).toBeUndefined();
  });

  it('throws CollectionNotFoundError on 404 VELES-002', async () => {
    mockErrorReply(404, 'VELES-002', "Collection 'missing' not found");

    await expect(backend.getRelations('missing', 1)).rejects.toThrow(
      CollectionNotFoundError
    );
  });
});

describe('setTtlDurable', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('PATCHes /collections/{name}/points/{id}/ttl with ttl_seconds body', async () => {
    mockReply({ updated: true });

    await backend.setTtlDurable('kg', 10, 3600);

    const { url, method, body } = lastCall();
    expect(method).toBe('PATCH');
    expect(url).toBe('http://localhost:8080/collections/kg/points/10/ttl');
    expect(body).toEqual({ ttl_seconds: 3600 });
  });

  it('keeps a u64 string point id verbatim in the path', async () => {
    mockReply({ updated: true });

    await backend.setTtlDurable('kg', '18446744073709551615', 60);

    expect(lastCall().url).toBe(
      'http://localhost:8080/collections/kg/points/18446744073709551615/ttl'
    );
  });

  it('throws PointNotFound-typed VelesError on 404 VELES-003', async () => {
    mockErrorReply(404, 'VELES-003', 'Point 99 not found');

    await expect(backend.setTtlDurable('kg', 99, 60)).rejects.toThrow(
      VelesError
    );
  });

  it('throws CollectionNotFoundError on 404 VELES-002', async () => {
    mockErrorReply(404, 'VELES-002', "Collection 'missing' not found");

    await expect(backend.setTtlDurable('missing', 1, 60)).rejects.toThrow(
      CollectionNotFoundError
    );
  });
});
