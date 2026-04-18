/**
 * Index Backend Tests (S4-07)
 *
 * Covers the four index management functions in
 * `src/backends/index-backend.ts`: createIndex, listIndexes, hasIndex,
 * dropIndex. Exercises happy paths, URL encoding, snake_case to
 * camelCase mapping, the BUG-2 fallback for empty 204 bodies, and the
 * error routing (VELES-002 → CollectionNotFoundError or sentinel).
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  createIndex,
  listIndexes,
  hasIndex,
  dropIndex,
} from '../src/backends/index-backend';
import type { IndexTransport } from '../src/backends/index-backend';
import type { TransportResponse } from '../src/backends/shared';
import { CollectionNotFoundError } from '../src/errors';

function buildTransport(overrides: Partial<IndexTransport> = {}): IndexTransport {
  return {
    requestJson: vi.fn(),
    ...overrides,
  };
}

function typedError(
  code = 'VELES-002',
  message = "Collection 'missing' not found"
): TransportResponse<never> {
  return { error: { code, message } };
}

describe('createIndex', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('posts to /collections/{name}/indexes with default index_type=hash', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { ok: true },
    } satisfies TransportResponse<unknown>);

    await createIndex(transport, 'docs', { label: 'Person', property: 'email' });

    expect(transport.requestJson).toHaveBeenCalledTimes(1);
    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/docs/indexes',
      { label: 'Person', property: 'email', index_type: 'hash' }
    );
  });

  it('forwards explicit indexType=range in the body', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { ok: true },
    });

    await createIndex(transport, 'docs', {
      label: 'Person',
      property: 'age',
      indexType: 'range',
    });

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/docs/indexes',
      { label: 'Person', property: 'age', index_type: 'range' }
    );
  });

  it('URL-encodes collection name containing a space', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { ok: true },
    });

    await createIndex(transport, 'my col', { label: 'P', property: 'n' });

    const call = (transport.requestJson as ReturnType<typeof vi.fn>).mock.calls[0]!;
    expect(call[1]).toBe('/collections/my%20col/indexes');
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(
      createIndex(transport, 'missing', { label: 'P', property: 'n' })
    ).rejects.toThrow(CollectionNotFoundError);
  });
});

describe('listIndexes', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('maps snake_case response fields to camelCase', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {
        indexes: [
          {
            label: 'Person',
            property: 'email',
            index_type: 'hash',
            cardinality: 1234,
            memory_bytes: 8192,
          },
          {
            label: 'Person',
            property: 'age',
            index_type: 'range',
            cardinality: 100,
            memory_bytes: 4096,
          },
        ],
        total: 2,
      },
    });

    const result = await listIndexes(transport, 'docs');

    expect(result).toHaveLength(2);
    expect(result[0]).toEqual({
      label: 'Person',
      property: 'email',
      indexType: 'hash',
      cardinality: 1234,
      memoryBytes: 8192,
    });
    expect(result[1]).toEqual({
      label: 'Person',
      property: 'age',
      indexType: 'range',
      cardinality: 100,
      memoryBytes: 4096,
    });
  });

  it('returns [] when data.indexes is missing', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { total: 0 },
    });

    const result = await listIndexes(transport, 'docs');
    expect(result).toEqual([]);
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(listIndexes(transport, 'missing')).rejects.toThrow(
      CollectionNotFoundError
    );
  });
});

describe('hasIndex', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('returns true when both label and property match', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {
        indexes: [
          {
            label: 'Person',
            property: 'email',
            index_type: 'hash',
            cardinality: 1,
            memory_bytes: 1,
          },
        ],
        total: 1,
      },
    });

    const result = await hasIndex(transport, 'docs', 'Person', 'email');
    expect(result).toBe(true);
  });

  it('returns false when label matches but property differs', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {
        indexes: [
          {
            label: 'Person',
            property: 'email',
            index_type: 'hash',
            cardinality: 1,
            memory_bytes: 1,
          },
        ],
        total: 1,
      },
    });

    const result = await hasIndex(transport, 'docs', 'Person', 'age');
    expect(result).toBe(false);
  });

  it('returns false when property matches but label differs', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {
        indexes: [
          {
            label: 'Person',
            property: 'email',
            index_type: 'hash',
            cardinality: 1,
            memory_bytes: 1,
          },
        ],
        total: 1,
      },
    });

    const result = await hasIndex(transport, 'docs', 'Company', 'email');
    expect(result).toBe(false);
  });
});

describe('dropIndex', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('returns true when server replies { dropped: true }', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { dropped: true },
    });

    const result = await dropIndex(transport, 'docs', 'Person', 'email');
    expect(result).toBe(true);
  });

  it('returns true when body is empty (BUG-2 fallback, 204 path)', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({});

    const result = await dropIndex(transport, 'docs', 'Person', 'email');
    expect(result).toBe(true);
  });

  it('URL-encodes label and property containing special characters', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { dropped: true },
    });

    await dropIndex(transport, 'docs', 'Per son', 'e/mail');

    const call = (transport.requestJson as ReturnType<typeof vi.fn>).mock.calls[0]!;
    expect(call[1]).toBe('/collections/docs/indexes/Per%20son/e%2Fmail');
  });

  it('returns false on VELES-002 (not-found sentinel)', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    const result = await dropIndex(transport, 'missing', 'Person', 'email');
    expect(result).toBe(false);
  });
});
