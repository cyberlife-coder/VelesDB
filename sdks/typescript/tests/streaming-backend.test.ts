/**
 * Streaming Backend Tests (S4-02)
 *
 * Tests for the streamUpsertPoints batch wrapper that sends NDJSON to
 * POST /collections/{name}/points/stream.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  streamUpsertPoints,
  trainPq,
  streamInsert,
} from '../src/backends/streaming-backend';
import type { StreamingTransport } from '../src/backends/streaming-backend';
import type { TransportResponse } from '../src/backends/shared';
import { BackpressureError, ConnectionError, VelesDBError } from '../src/types';
import { CollectionNotFoundError } from '../src/errors';

const mockFetch = vi.fn();
global.fetch = mockFetch;

function buildTransport(overrides: Partial<StreamingTransport> = {}): StreamingTransport {
  return {
    requestJson: vi.fn(),
    baseUrl: 'http://localhost:8080',
    apiKey: 'test-key',
    timeout: 5000,
    parseRestPointId: (id: string | number) => {
      if (typeof id === 'string') return Number(id);
      return id;
    },
    sparseVectorToRestFormat: (sv: Record<number, number>) => sv,
    mapStatusToErrorCode: (status: number) => {
      const map: Record<number, string> = { 400: 'BAD_REQUEST', 404: 'NOT_FOUND', 500: 'INTERNAL_ERROR' };
      return map[status] ?? 'UNKNOWN_ERROR';
    },
    extractErrorPayload: (data: unknown) => {
      if (!data || typeof data !== 'object') return {};
      const d = data as Record<string, unknown>;
      return {
        code: typeof d.code === 'string' ? d.code : undefined,
        message: typeof d.message === 'string' ? d.message : typeof d.error === 'string' ? d.error : undefined,
      };
    },
    ...overrides,
  };
}

describe('streamUpsertPoints', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('should send NDJSON to /collections/{name}/points/stream', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({
        message: 'Stream processed',
        inserted: 2,
        malformed: 0,
        failed_upserts: 0,
        network_errors: 0,
      }),
    });

    const transport = buildTransport();
    const docs = [
      { id: 1, vector: [1.0, 0.0, 0.0], payload: { title: 'A' } },
      { id: 2, vector: [0.0, 1.0, 0.0], payload: { title: 'B' } },
    ];

    const result = await streamUpsertPoints(transport, 'test-col', docs);

    expect(result.inserted).toBe(2);
    expect(result.malformed).toBe(0);
    expect(result.failedUpserts).toBe(0);
    expect(result.networkErrors).toBe(0);

    expect(mockFetch).toHaveBeenCalledTimes(1);
    const [url, opts] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('http://localhost:8080/collections/test-col/points/stream');
    expect(opts.method).toBe('POST');
    expect((opts.headers as Record<string, string>)['Content-Type']).toBe('application/x-ndjson');
    expect((opts.headers as Record<string, string>)['Authorization']).toBe('Bearer test-key');

    // Body should be NDJSON (one JSON object per line)
    const body = opts.body as string;
    const lines = body.split('\n');
    expect(lines.length).toBe(2);
    expect(JSON.parse(lines[0]!)).toEqual({ id: 1, vector: [1.0, 0.0, 0.0], payload: { title: 'A' } });
    expect(JSON.parse(lines[1]!)).toEqual({ id: 2, vector: [0.0, 1.0, 0.0], payload: { title: 'B' } });
  });

  it('should omit Authorization header when no apiKey', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ message: 'ok', inserted: 1, malformed: 0, failed_upserts: 0, network_errors: 0 }),
    });

    const transport = buildTransport({ apiKey: undefined });
    await streamUpsertPoints(transport, 'col', [{ id: 1, vector: [1.0] }]);

    const [, opts] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect((opts.headers as Record<string, string>)['Authorization']).toBeUndefined();
  });

  it('should include sparse_vector in NDJSON when present', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ message: 'ok', inserted: 1, malformed: 0, failed_upserts: 0, network_errors: 0 }),
    });

    const transport = buildTransport();
    const docs = [
      { id: 1, vector: [1.0, 0.0], sparseVector: { 5: 0.8, 10: 0.3 } },
    ];

    await streamUpsertPoints(transport, 'col', docs);

    const body = (mockFetch.mock.calls[0] as [string, RequestInit])[1].body as string;
    const parsed = JSON.parse(body);
    expect(parsed.sparse_vector).toEqual({ 5: 0.8, 10: 0.3 });
  });

  it('should throw BackpressureError on 429 response', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 429,
      json: () => Promise.resolve({ error: 'Rate limited' }),
    });

    const transport = buildTransport();
    await expect(
      streamUpsertPoints(transport, 'col', [{ id: 1, vector: [1.0] }])
    ).rejects.toThrow(BackpressureError);
  });

  it('should throw VelesDBError on non-ok response', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 404,
      json: () => Promise.resolve({ error: 'Collection not found' }),
    });

    const transport = buildTransport();
    await expect(
      streamUpsertPoints(transport, 'missing', [{ id: 1, vector: [1.0] }])
    ).rejects.toThrow(VelesDBError);
  });

  it('should throw ConnectionError on timeout (AbortError)', async () => {
    const abortError = new Error('Aborted');
    abortError.name = 'AbortError';
    mockFetch.mockRejectedValueOnce(abortError);

    const transport = buildTransport();
    await expect(
      streamUpsertPoints(transport, 'col', [{ id: 1, vector: [1.0] }])
    ).rejects.toThrow(ConnectionError);
  });

  it('should throw ConnectionError on network failure', async () => {
    mockFetch.mockRejectedValueOnce(new Error('Network unreachable'));

    const transport = buildTransport();
    await expect(
      streamUpsertPoints(transport, 'col', [{ id: 1, vector: [1.0] }])
    ).rejects.toThrow(ConnectionError);
  });

  it('should handle empty docs array', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ message: 'Stream processed', inserted: 0, malformed: 0, failed_upserts: 0, network_errors: 0 }),
    });

    const transport = buildTransport();
    const result = await streamUpsertPoints(transport, 'col', []);

    expect(result.inserted).toBe(0);
    // Body should be empty string (no lines)
    const body = (mockFetch.mock.calls[0] as [string, RequestInit])[1].body as string;
    expect(body).toBe('');
  });

  it('should handle Float32Array vectors', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ message: 'ok', inserted: 1, malformed: 0, failed_upserts: 0, network_errors: 0 }),
    });

    const transport = buildTransport();
    const docs = [
      { id: 1, vector: new Float32Array([1.0, 2.0, 3.0]), payload: { x: 1 } },
    ];

    await streamUpsertPoints(transport, 'col', docs);

    const body = (mockFetch.mock.calls[0] as [string, RequestInit])[1].body as string;
    const parsed = JSON.parse(body);
    expect(parsed.vector).toEqual([1.0, 2.0, 3.0]);
  });

  it('should report partial failures from server response', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({
        message: 'Stream processed',
        inserted: 5,
        malformed: 2,
        failed_upserts: 1,
        network_errors: 0,
      }),
    });

    const transport = buildTransport();
    const result = await streamUpsertPoints(transport, 'col', [{ id: 1, vector: [1.0] }]);

    expect(result.inserted).toBe(5);
    expect(result.malformed).toBe(2);
    expect(result.failedUpserts).toBe(1);
    expect(result.networkErrors).toBe(0);
    expect(result.message).toBe('Stream processed');
  });

  it('should encode collection name in URL', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ message: 'ok', inserted: 1, malformed: 0, failed_upserts: 0, network_errors: 0 }),
    });

    const transport = buildTransport();
    await streamUpsertPoints(transport, 'my collection', [{ id: 1, vector: [1.0] }]);

    const [url] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('http://localhost:8080/collections/my%20collection/points/stream');
  });

  it('should set null payload when doc.payload is undefined', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ message: 'ok', inserted: 1, malformed: 0, failed_upserts: 0, network_errors: 0 }),
    });

    const transport = buildTransport();
    await streamUpsertPoints(transport, 'col', [{ id: 1, vector: [1.0] }]);

    const body = (mockFetch.mock.calls[0] as [string, RequestInit])[1].body as string;
    const parsed = JSON.parse(body);
    expect(parsed.payload).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// trainPq — POST /query with a VelesQL TRAIN QUANTIZER statement
// ---------------------------------------------------------------------------

describe('trainPq', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('uses defaults m=8, k=256 without opq', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { message: 'PQ training initiated' },
    } satisfies TransportResponse<{ message: string }>);

    await trainPq(transport, 'docs');

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/query',
      { query: 'TRAIN QUANTIZER ON docs WITH (m=8, k=256)' }
    );
  });

  it('reflects explicit m=16, k=512 in the query', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { message: 'ok' },
    });

    await trainPq(transport, 'docs', { m: 16, k: 512 });

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/query',
      { query: 'TRAIN QUANTIZER ON docs WITH (m=16, k=512)' }
    );
  });

  it('appends opq=true when options.opq is set', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { message: 'ok' },
    });

    await trainPq(transport, 'docs', { m: 8, k: 256, opq: true });

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/query',
      { query: 'TRAIN QUANTIZER ON docs WITH (m=8, k=256, opq=true)' }
    );
  });

  it('returns the server-provided message', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { message: 'Training started for docs (PQ m=8 k=256)' },
    });

    const result = await trainPq(transport, 'docs');
    expect(result).toBe('Training started for docs (PQ m=8 k=256)');
  });

  it('falls back to "PQ training initiated" when data.message is missing', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {} as { message: string },
    });

    const result = await trainPq(transport, 'docs');
    expect(result).toBe('PQ training initiated');
  });

  it('throws a typed VelesError on error payload', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      error: { code: 'VELES-002', message: "Collection 'missing' not found" },
    });

    await expect(trainPq(transport, 'missing')).rejects.toThrow(
      CollectionNotFoundError
    );
  });
});

// ---------------------------------------------------------------------------
// streamInsert — one HTTP POST per document (bounded ingestion channel)
// ---------------------------------------------------------------------------

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

  it('does not throw on HTTP 202 Accepted', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 202,
      json: () => Promise.resolve({}),
    });

    const transport = buildTransport();
    await expect(
      streamInsert(transport, 'docs', [{ id: 1, vector: [0.1] }])
    ).resolves.toBeUndefined();
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
