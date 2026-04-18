/**
 * Streaming Backend Tests — streamInsert (S4-02 / S4-07)
 *
 * Tests for the streamInsert helper: one HTTP POST per document
 * (bounded ingestion channel).
 *
 * Split from the original streaming-backend.test.ts to keep each test
 * file under the 500-line file-size limit. Sibling files:
 *   - streaming-backend.test.ts (streamUpsertPoints)
 *   - streaming-backend-train-pq.test.ts (trainPq)
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { streamInsert } from '../src/backends/streaming-backend';
import { BackpressureError, ConnectionError, VelesDBError } from '../src/types';
import { buildTransport } from './helpers/build-streaming-transport';

const mockFetch = vi.fn();
global.fetch = mockFetch;

describe('streamInsert', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('POSTs one document to /collections/{name}/stream/insert with JSON body', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({}),
    });

    const transport = buildTransport();
    await streamInsert(transport, 'docs', [
      { id: 1, vector: [0.1, 0.2], payload: { title: 'A' } },
    ]);

    expect(mockFetch).toHaveBeenCalledTimes(1);
    const [url, opts] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('http://localhost:8080/collections/docs/stream/insert');
    expect(opts.method).toBe('POST');
    expect((opts.headers as Record<string, string>)['Content-Type']).toBe(
      'application/json'
    );
    const body = JSON.parse(opts.body as string) as {
      id: number;
      vector: number[];
      payload: Record<string, unknown>;
    };
    expect(body.id).toBe(1);
    expect(body.vector).toEqual([0.1, 0.2]);
    expect(body.payload).toEqual({ title: 'A' });
  });

  it('omits Authorization header when apiKey is undefined', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({}),
    });

    const transport = buildTransport({ apiKey: undefined });
    await streamInsert(transport, 'docs', [{ id: 1, vector: [0.1] }]);

    const [, opts] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect(
      (opts.headers as Record<string, string>)['Authorization']
    ).toBeUndefined();
  });

  it('sets Authorization: Bearer <key> when apiKey is set', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({}),
    });

    const transport = buildTransport({ apiKey: 'secret-42' });
    await streamInsert(transport, 'docs', [{ id: 1, vector: [0.1] }]);

    const [, opts] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect((opts.headers as Record<string, string>)['Authorization']).toBe(
      'Bearer secret-42'
    );
  });

  it('includes sparse_vector in body when doc.sparseVector is provided', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({}),
    });

    const transport = buildTransport();
    await streamInsert(transport, 'docs', [
      { id: 1, vector: [0.1], sparseVector: { 5: 0.8, 10: 0.3 } },
    ]);

    const [, opts] = mockFetch.mock.calls[0] as [string, RequestInit];
    const body = JSON.parse(opts.body as string) as {
      sparse_vector: Record<number, number>;
    };
    expect(body.sparse_vector).toEqual({ 5: 0.8, 10: 0.3 });
  });

  it('converts Float32Array input to a plain number array', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({}),
    });

    const transport = buildTransport();
    await streamInsert(transport, 'docs', [
      { id: 1, vector: new Float32Array([1.0, 2.0, 3.0]) },
    ]);

    const [, opts] = mockFetch.mock.calls[0] as [string, RequestInit];
    const body = JSON.parse(opts.body as string) as { vector: number[] };
    expect(Array.isArray(body.vector)).toBe(true);
    expect(body.vector).toEqual([1.0, 2.0, 3.0]);
  });

  // NOTE: streamInsert omits payload from JSON body when undefined, unlike
  // streamUpsertPoints which serializes it as null. Tracked in
  // TODO(US-S4-07): streamInsert payload alignment — follow-up source-level
  // fix. This test pins the current behavior.
  it('omits payload key from JSON body when doc.payload is undefined (pre-existing limitation)', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({}),
    });

    const transport = buildTransport();
    await streamInsert(transport, 'docs', [{ id: 1, vector: [0.1] }]);

    const [, opts] = mockFetch.mock.calls[0] as [string, RequestInit];
    const rawBody = opts.body as string;
    expect(rawBody).not.toContain('"payload"');

    const body = JSON.parse(rawBody) as Record<string, unknown>;
    expect('payload' in body).toBe(false);
  });

  it('does not throw on HTTP 202 Accepted', async () => {
    // Real fetch() sets Response.ok = true for all 2xx statuses (including 202),
    // so we mock ok: true, status: 202 to match actual runtime behavior.
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 202,
      json: () => Promise.resolve({}),
    });

    const transport = buildTransport();
    await expect(
      streamInsert(transport, 'docs', [{ id: 1, vector: [0.1] }])
    ).resolves.toBeUndefined();
  });

  it('returns without reading JSON body on realistic 202 success path', async () => {
    // Realistic 202 path: ok: true, status: 202. The function should return
    // early (the !response.ok guard is skipped, and 202 matches the success
    // path), so response.json() must NOT be called.
    const jsonSpy = vi.fn(() => Promise.resolve({}));
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 202,
      json: jsonSpy,
    });

    const transport = buildTransport();
    await expect(
      streamInsert(transport, 'docs', [{ id: 1, vector: [0.1] }])
    ).resolves.toBeUndefined();

    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(jsonSpy).not.toHaveBeenCalled();
  });

  it('throws BackpressureError on HTTP 429', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 429,
      json: () => Promise.resolve({}),
    });

    const transport = buildTransport();
    await expect(
      streamInsert(transport, 'docs', [{ id: 1, vector: [0.1] }])
    ).rejects.toThrow(BackpressureError);
  });

  it('throws VelesDBError on non-ok / non-429 / non-202 response', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 500,
      json: () =>
        Promise.resolve({
          code: 'VELES-009',
          message: 'Invalid payload',
        }),
    });

    const transport = buildTransport();
    await expect(
      streamInsert(transport, 'docs', [{ id: 1, vector: [0.1] }])
    ).rejects.toThrow(VelesDBError);
  });

  it('throws ConnectionError when fetch aborts (AbortError)', async () => {
    const abortError = new Error('Aborted');
    abortError.name = 'AbortError';
    mockFetch.mockRejectedValueOnce(abortError);

    const transport = buildTransport();
    await expect(
      streamInsert(transport, 'docs', [{ id: 1, vector: [0.1] }])
    ).rejects.toThrow(ConnectionError);
  });

  it('throws ConnectionError on generic network failure', async () => {
    mockFetch.mockRejectedValueOnce(new Error('DNS failure'));

    const transport = buildTransport();
    await expect(
      streamInsert(transport, 'docs', [{ id: 1, vector: [0.1] }])
    ).rejects.toThrow(ConnectionError);
  });

  it('calls fetch once per document for multi-doc input', async () => {
    mockFetch
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: () => Promise.resolve({}),
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: () => Promise.resolve({}),
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: () => Promise.resolve({}),
      });

    const transport = buildTransport();
    await streamInsert(transport, 'docs', [
      { id: 1, vector: [0.1] },
      { id: 2, vector: [0.2] },
      { id: 3, vector: [0.3] },
    ]);

    expect(mockFetch).toHaveBeenCalledTimes(3);
  });
});
