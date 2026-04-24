/**
 * REST HTTP Transport Tests (#598)
 *
 * Covers `src/backends/rest-http.ts`: pure helpers (`mapStatusToErrorCode`,
 * `extractErrorPayload`, `parseNodeId`) and the HTTP request layer
 * (`request`, `buildBaseTransport`, `buildCrudTransport`,
 * `buildSearchTransport`, `buildQueryTransport`, `buildStreamingTransport`,
 * `buildAgentMemoryTransport`) using `global.fetch = vi.fn()` — same
 * pattern as `rest-backend.test.ts`.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  mapStatusToErrorCode,
  extractErrorPayload,
  parseNodeId,
  request,
  buildBaseTransport,
  buildCrudTransport,
  buildSearchTransport,
  buildQueryTransport,
  buildStreamingTransport,
  buildAgentMemoryTransport,
  type RestHttpConfig,
} from '../src/backends/rest-http';
import { ConnectionError } from '../src/types';

const mockFetch = vi.fn();
global.fetch = mockFetch;

const config: RestHttpConfig = {
  baseUrl: 'http://localhost:8080',
  apiKey: 'test-key',
  timeout: 5000,
};

const configNoKey: RestHttpConfig = {
  baseUrl: 'http://localhost:8080',
  timeout: 5000,
};

describe('mapStatusToErrorCode', () => {
  it.each([
    [400, 'BAD_REQUEST'],
    [401, 'UNAUTHORIZED'],
    [403, 'FORBIDDEN'],
    [404, 'NOT_FOUND'],
    [409, 'CONFLICT'],
    [429, 'RATE_LIMITED'],
    [500, 'INTERNAL_ERROR'],
    [503, 'SERVICE_UNAVAILABLE'],
  ])('maps status %d to %s', (status, expected) => {
    expect(mapStatusToErrorCode(status)).toBe(expected);
  });

  it('returns UNKNOWN_ERROR for unmapped status', () => {
    expect(mapStatusToErrorCode(418)).toBe('UNKNOWN_ERROR');
    expect(mapStatusToErrorCode(999)).toBe('UNKNOWN_ERROR');
  });
});

describe('extractErrorPayload', () => {
  it('returns {} for non-objects (null, primitives)', () => {
    expect(extractErrorPayload(null)).toEqual({});
    expect(extractErrorPayload(undefined)).toEqual({});
    expect(extractErrorPayload('str')).toEqual({});
    expect(extractErrorPayload(42)).toEqual({});
  });

  it('extracts top-level code and message', () => {
    expect(
      extractErrorPayload({ code: 'VELES-002', message: 'not found' })
    ).toEqual({ code: 'VELES-002', message: 'not found' });
  });

  it('extracts nested error.code and error.message', () => {
    expect(
      extractErrorPayload({
        error: { code: 'VELES-010', message: 'bad config' },
      })
    ).toEqual({ code: 'VELES-010', message: 'bad config' });
  });

  it('prefers nested fields over top-level', () => {
    expect(
      extractErrorPayload({
        code: 'OUTER',
        message: 'outer',
        error: { code: 'INNER', message: 'inner' },
      })
    ).toEqual({ code: 'INNER', message: 'inner' });
  });

  it('falls back to top-level `error` string field for message', () => {
    expect(extractErrorPayload({ error: 'some error string' })).toEqual({
      code: undefined,
      message: 'some error string',
    });
  });

  it('returns undefined for non-string fields', () => {
    expect(extractErrorPayload({ code: 42, message: {} })).toEqual({
      code: undefined,
      message: undefined,
    });
  });
});

describe('parseNodeId', () => {
  it('returns 0 for null and undefined', () => {
    expect(parseNodeId(null)).toBe(0);
    expect(parseNodeId(undefined)).toBe(0);
  });

  it('passes bigint through unchanged', () => {
    expect(parseNodeId(123n)).toBe(123n);
  });

  it('passes small number through unchanged', () => {
    expect(parseNodeId(42)).toBe(42);
  });

  it('converts string within safe integer range to number', () => {
    expect(parseNodeId('42')).toBe(42);
  });

  it('converts string outside safe integer range to bigint', () => {
    // 2^53 + 5 — beyond MAX_SAFE_INTEGER
    const big = '9007199254740997';
    const out = parseNodeId(big);
    expect(typeof out).toBe('bigint');
    expect(out).toBe(BigInt(big));
  });

  it('returns 0 for booleans and other unsupported types', () => {
    expect(parseNodeId(true)).toBe(0);
    expect(parseNodeId({})).toBe(0);
    expect(parseNodeId([])).toBe(0);
  });
});

describe('request — happy path', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('GET with apiKey sets Authorization header and returns data', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ ok: true }),
    });

    const result = await request(config, 'GET', '/health');

    expect(result).toEqual({ data: { ok: true } });
    const call = mockFetch.mock.calls[0]!;
    expect(call[0]).toBe('http://localhost:8080/health');
    expect(call[1].method).toBe('GET');
    expect(call[1].headers.Authorization).toBe('Bearer test-key');
    expect(call[1].headers['Content-Type']).toBe('application/json');
    expect(call[1].body).toBeUndefined();
  });

  it('POST without apiKey omits Authorization and serialises body', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ id: 1 }),
    });

    await request(configNoKey, 'POST', '/items', { name: 'x' });

    const call = mockFetch.mock.calls[0]!;
    expect(call[1].headers.Authorization).toBeUndefined();
    expect(call[1].body).toBe(JSON.stringify({ name: 'x' }));
  });
});

describe('request — error responses', () => {
  beforeEach(() => vi.clearAllMocks());

  it('maps non-ok response with typed VELES code to error payload', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 404,
      json: () =>
        Promise.resolve({ code: 'VELES-002', message: 'Collection missing' }),
    });

    const result = await request(config, 'GET', '/x');
    expect(result).toEqual({
      error: { code: 'VELES-002', message: 'Collection missing' },
    });
  });

  it('falls back to status-derived code and synthesised message', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 500,
      json: () => Promise.resolve({}),
    });

    const result = await request(config, 'GET', '/x');
    expect(result).toEqual({
      error: { code: 'INTERNAL_ERROR', message: 'HTTP 500' },
    });
  });

  it('tolerates a body that is not valid JSON (json() throws)', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 503,
      json: () => Promise.reject(new Error('bad json')),
    });

    const result = await request(config, 'GET', '/x');
    expect(result).toEqual({
      error: { code: 'SERVICE_UNAVAILABLE', message: 'HTTP 503' },
    });
  });
});

describe('request — connection errors', () => {
  beforeEach(() => vi.clearAllMocks());

  it('wraps AbortError (timeout) as ConnectionError', async () => {
    const abort = new Error('timeout');
    abort.name = 'AbortError';
    mockFetch.mockRejectedValue(abort);

    await expect(request(config, 'GET', '/x')).rejects.toBeInstanceOf(
      ConnectionError
    );
    await expect(request(config, 'GET', '/x')).rejects.toThrow(
      /Request timeout/
    );
  });

  it('wraps a generic Error as ConnectionError with the inner message', async () => {
    mockFetch.mockRejectedValue(new Error('network down'));

    await expect(request(config, 'GET', '/x')).rejects.toBeInstanceOf(
      ConnectionError
    );
    await expect(request(config, 'GET', '/x')).rejects.toThrow(
      /network down/
    );
  });

  it('wraps a non-Error thrown value as ConnectionError "Unknown error"', async () => {
    mockFetch.mockRejectedValueOnce('weird');

    await expect(request(config, 'GET', '/x')).rejects.toThrow(/Unknown error/);
  });
});

describe('transport adapter factories', () => {
  beforeEach(() => vi.clearAllMocks());

  it('buildBaseTransport returns an object with requestJson that delegates to fetch', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ ok: true }),
    });

    const transport = buildBaseTransport(config);
    const result = await transport.requestJson('GET', '/x');

    expect(result).toEqual({ data: { ok: true } });
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('buildCrudTransport delegates to fetch', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({}),
    });

    const transport = buildCrudTransport(config);
    await transport.requestJson('POST', '/x', { a: 1 });

    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it('buildSearchTransport exposes a sparseToRest helper that works', () => {
    const transport = buildSearchTransport(config);
    const rest = transport.sparseToRest({ 1: 0.5, 2: 0.7 });
    expect(rest).toEqual({ '1': 0.5, '2': 0.7 });
  });

  it('buildQueryTransport exposes a parseNodeId helper', () => {
    const transport = buildQueryTransport(config);
    expect(transport.parseNodeId('42')).toBe(42);
  });

  it('buildStreamingTransport copies baseUrl/apiKey/timeout and exposes the parsers', () => {
    const transport = buildStreamingTransport(config);
    expect(transport.baseUrl).toBe(config.baseUrl);
    expect(transport.apiKey).toBe(config.apiKey);
    expect(transport.timeout).toBe(config.timeout);
    expect(transport.mapStatusToErrorCode(404)).toBe('NOT_FOUND');
    expect(transport.extractErrorPayload({ code: 'X' })).toEqual({
      code: 'X',
      message: undefined,
    });
    expect(typeof transport.parseRestPointId).toBe('function');
    expect(typeof transport.sparseVectorToRestFormat).toBe('function');
  });

  it('buildAgentMemoryTransport wires searchVectors through the supplied searchFn', async () => {
    const searchFn = vi.fn().mockResolvedValueOnce([{ id: 1, score: 0.5 }]);
    const transport = buildAgentMemoryTransport(config, searchFn);

    const out = await transport.searchVectors('c', [0.1], 5, { k: '1' });

    expect(searchFn).toHaveBeenCalledWith('c', [0.1], {
      k: 5,
      filter: { k: '1' },
    });
    expect(out).toEqual([{ id: 1, score: 0.5 }]);
  });
});
