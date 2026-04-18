/**
 * Scroll Backend Tests (S4-07)
 *
 * Covers the cursor-based pagination wrapper in
 * `src/backends/scroll-backend.ts`. Verifies:
 * - camelCase → snake_case body mapping for each optional field
 * - empty-request path (no body keys)
 * - next_cursor → nextCursor response mapping (keeps null)
 * - URL encoding of the collection segment
 * - VELES-002 error routing to CollectionNotFoundError
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { scroll } from '../src/backends/scroll-backend';
import type { ScrollTransport } from '../src/backends/scroll-backend';
import type { TransportResponse } from '../src/backends/shared';
import { CollectionNotFoundError } from '../src/errors';

function buildTransport(overrides: Partial<ScrollTransport> = {}): ScrollTransport {
  return {
    requestJson: vi.fn(),
    ...overrides,
  };
}

function mockSuccess(
  transport: ScrollTransport,
  data: Record<string, unknown>
): void {
  (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
    data,
  } satisfies TransportResponse<unknown>);
}

describe('scroll', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('sends all fields snake-cased when cursor, batchSize, and filter are provided', async () => {
    const transport = buildTransport();
    mockSuccess(transport, { points: [], next_cursor: null });

    await scroll(transport, 'docs', {
      cursor: 'abc',
      batchSize: 50,
      filter: { category: 'tech' },
    });

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/docs/points/scroll',
      { cursor: 'abc', batch_size: 50, filter: { category: 'tech' } }
    );
  });

  it('sends an empty body {} when request is omitted', async () => {
    const transport = buildTransport();
    mockSuccess(transport, { points: [], next_cursor: null });

    await scroll(transport, 'docs');

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/docs/points/scroll',
      {}
    );
  });

  it('sends only cursor when only cursor is set', async () => {
    const transport = buildTransport();
    mockSuccess(transport, { points: [], next_cursor: null });

    await scroll(transport, 'docs', { cursor: 42 });

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/docs/points/scroll',
      { cursor: 42 }
    );
  });

  it('sends only batch_size when only batchSize is set', async () => {
    const transport = buildTransport();
    mockSuccess(transport, { points: [], next_cursor: null });

    await scroll(transport, 'docs', { batchSize: 25 });

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/docs/points/scroll',
      { batch_size: 25 }
    );
  });

  it('sends only filter when only filter is set', async () => {
    const transport = buildTransport();
    mockSuccess(transport, { points: [], next_cursor: null });

    await scroll(transport, 'docs', { filter: { active: true } });

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/docs/points/scroll',
      { filter: { active: true } }
    );
  });

  it('maps next_cursor to nextCursor and passes points through', async () => {
    const transport = buildTransport();
    mockSuccess(transport, {
      points: [
        { id: 1, vector: [0.1, 0.2], payload: { title: 'A' } },
        { id: 2, vector: [0.3, 0.4], payload: { title: 'B' } },
      ],
      next_cursor: 'page-2',
    });

    const result = await scroll(transport, 'docs');

    expect(result.points).toHaveLength(2);
    expect(result.points[0]).toEqual({
      id: 1,
      vector: [0.1, 0.2],
      payload: { title: 'A' },
    });
    expect(result.nextCursor).toBe('page-2');
  });

  it('preserves null next_cursor (end-of-stream signal)', async () => {
    const transport = buildTransport();
    mockSuccess(transport, { points: [], next_cursor: null });

    const result = await scroll(transport, 'docs');
    expect(result.nextCursor).toBeNull();
  });

  it('URL-encodes collection name containing a space', async () => {
    const transport = buildTransport();
    mockSuccess(transport, { points: [], next_cursor: null });

    await scroll(transport, 'my col');

    const call = (transport.requestJson as ReturnType<typeof vi.fn>).mock.calls[0]!;
    expect(call[1]).toBe('/collections/my%20col/points/scroll');
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      error: { code: 'VELES-002', message: "Collection 'missing' not found" },
    });

    await expect(scroll(transport, 'missing')).rejects.toThrow(
      CollectionNotFoundError
    );
  });
});
