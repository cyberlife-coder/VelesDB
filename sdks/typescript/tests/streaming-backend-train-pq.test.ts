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
import {
  CollectionNotFoundError,
  InvalidCollectionNameError,
} from '../src/errors';
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

  // Issue #597 (closed): trainPq now validates the collection name before
  // interpolating it into the VelesQL query, preventing VelesQL injection
  // via crafted names. Rules mirror the core Rust validator.
  describe('collection name validation (#597)', () => {
    const validNames = [
      'docs',
      'docs_v2',
      'docs-v2',
      'A1_b2-C3',
      'collection',
      'x',
      'x'.repeat(128), // max length
    ];

    for (const name of validNames) {
      it(`accepts valid collection name: ${name.length > 20 ? `${name.slice(0, 8)}... (len=${name.length})` : name}`, async () => {
        const transport = buildTransport();
        (
          transport.requestJson as ReturnType<typeof vi.fn>
        ).mockResolvedValueOnce({
          data: { message: 'ok' },
        });

        await expect(trainPq(transport, name)).resolves.toBe('ok');
        expect(transport.requestJson).toHaveBeenCalledTimes(1);
      });
    }

    const invalidNames: Array<[string, string]> = [
      ['my collection', 'contains space'],
      ["a'; DROP TABLE users;--", 'contains SQL injection characters'],
      ['docs"', 'contains double quote'],
      ['docs;', 'contains semicolon'],
      ['docs*', 'contains asterisk'],
      ['docs.bak', 'contains dot'],
      ['docs/evil', 'contains forward slash'],
      ['docs\\evil', 'contains backslash'],
      ['-leading-hyphen', 'starts with hyphen'],
      ['', 'is empty'],
      ['.', 'is dot (path traversal)'],
      ['..', 'is dotdot (path traversal)'],
      ['café', 'contains non-ASCII character'],
      ['CON', 'is Windows reserved name'],
      ['x'.repeat(129), 'exceeds max length'],
    ];

    for (const [name, reason] of invalidNames) {
      it(`rejects invalid collection name (${reason})`, async () => {
        const transport = buildTransport();
        // requestJson must NOT be called — validation happens first.
        await expect(trainPq(transport, name)).rejects.toThrow(
          InvalidCollectionNameError
        );
        expect(transport.requestJson).not.toHaveBeenCalled();
      });
    }
  });
});
