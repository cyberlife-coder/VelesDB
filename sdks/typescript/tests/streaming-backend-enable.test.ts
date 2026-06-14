/**
 * Streaming Backend Tests — enableStreaming
 *
 * Tests for the enableStreaming helper: a single POST to
 * `/collections/{name}/stream/enable` (via the shared `requestJson`
 * transport) that turns on the bounded streaming-ingestion channel. The
 * optional StreamingConfig is converted from camelCase to a snake_case JSON
 * body, omitting undefined fields so the server applies its defaults.
 *
 * Mirrors streaming-backend-train-pq.test.ts (the other requestJson-based
 * helper); shares the buildTransport() factory in
 * tests/helpers/build-streaming-transport.ts. Auth-header, timeout/abort, and
 * status→error-code mapping are owned by the shared `request<T>()` path and
 * covered in rest-http.test.ts.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { enableStreaming } from '../src/backends/streaming-backend';
import type { TransportResponse } from '../src/backends/shared';
import { CollectionNotFoundError } from '../src/errors';
import { buildTransport } from './helpers/build-streaming-transport';

describe('enableStreaming', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('POSTs to /collections/{name}/stream/enable with a snake_case body', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {},
    } satisfies TransportResponse<unknown>);

    await enableStreaming(transport, 'docs', {
      bufferSize: 4096,
      batchSize: 64,
      flushIntervalMs: 25,
    });

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/docs/stream/enable',
      { buffer_size: 4096, batch_size: 64, flush_interval_ms: 25 }
    );
  });

  it('omits undefined config fields so server defaults apply', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {},
    });

    await enableStreaming(transport, 'docs', { batchSize: 64 });

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/docs/stream/enable',
      { batch_size: 64 }
    );
  });

  it('sends an empty body when config is omitted entirely', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {},
    });

    await enableStreaming(transport, 'docs');

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/docs/stream/enable',
      {}
    );
  });

  it('encodes the collection name in the request path', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {},
    });

    await enableStreaming(transport, 'my docs');

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/my%20docs/stream/enable',
      {}
    );
  });

  it('throws a typed VelesError on an error payload', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      error: { code: 'VELES-002', message: "Collection 'docs' not found" },
    });

    await expect(enableStreaming(transport, 'docs')).rejects.toThrow(
      CollectionNotFoundError
    );
  });
});
