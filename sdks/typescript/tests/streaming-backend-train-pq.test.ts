/**
 * Streaming Backend Tests — trainPq (S4-02 / S4-07)
 *
 * Tests for the trainPq helper that POSTs a VelesQL TRAIN QUANTIZER
 * statement to /query.
 *
 * Split from the original streaming-backend.test.ts to keep each test
 * file under the 500-line file-size limit. Sibling files:
 *   - streaming-backend.test.ts (streamUpsertPoints)
 *   - streaming-backend-insert.test.ts (streamInsert)
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { trainPq } from '../src/backends/streaming-backend';
import type { TransportResponse } from '../src/backends/shared';
import { CollectionNotFoundError } from '../src/errors';
import { buildTransport } from './helpers/build-streaming-transport';

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

  // NOTE: trainPq interpolates collection name without escaping — tracked in
  // TODO(US-S4-07): trainPq escape — follow-up source-level fix. This
  // test pins the current behavior so future escaping is caught as a
  // breaking change.
  it('interpolates collection name raw into the VelesQL query (pre-existing limitation)', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { message: 'ok' },
    });

    await trainPq(transport, 'my collection');

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/query',
      { query: 'TRAIN QUANTIZER ON my collection WITH (m=8, k=256)' }
    );
  });
});
