/**
 * Search Backend Tests (#598)
 *
 * Covers `src/backends/search-backend.ts`: search, searchBatch,
 * textSearch, hybridSearch, multiQuerySearch, multiQuerySearchIds,
 * searchIds, sparseSearchNamed. Exercises
 * body shape (top_k, filter, include_vectors, fusion params), vector
 * normalisation (Float32Array → number[]), sparse-vector routing via
 * `transport.sparseToRest`, the `searchQualityToMode` integration, and
 * error routing.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  search,
  searchBatch,
  textSearch,
  hybridSearch,
  multiQuerySearch,
  multiQuerySearchIds,
  searchIds,
  sparseSearchNamed,
  type SearchTransport,
} from '../src/backends/search-backend';
import type { TransportResponse } from '../src/backends/shared';
import { CollectionNotFoundError } from '../src/errors';

function buildTransport(
  overrides: Partial<SearchTransport> = {}
): SearchTransport {
  return {
    requestJson: vi.fn(),
    sparseToRest: vi.fn((sv: Record<number, number>) => {
      // default: map keys to strings
      const out: Record<string, number> = {};
      for (const [k, v] of Object.entries(sv)) {
        out[k] = v;
      }
      return out;
    }),
    ...overrides,
  };
}

function typedError(
  code = 'VELES-002',
  message = "Collection 'missing' not found"
): TransportResponse<never> {
  return { error: { code, message } };
}

describe('search', () => {
  beforeEach(() => vi.clearAllMocks());

  it('POSTs to /search with defaults (top_k=10, include_vectors=false)', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [{ id: 1, score: 0.9 }] },
    });

    const result = await search(transport, 'docs', [0.1, 0.2]);

    const call = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]!;
    expect(call[0]).toBe('POST');
    expect(call[1]).toBe('/collections/docs/search');
    const body = call[2] as Record<string, unknown>;
    expect(body.vector).toEqual([0.1, 0.2]);
    expect(body.top_k).toBe(10);
    expect(body.filter).toBeUndefined();
    expect(body.include_vectors).toBe(false);
    expect(result).toEqual([{ id: 1, score: 0.9 }]);
  });

  it('normalises Float32Array to number[]', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [] },
    });

    await search(transport, 'docs', new Float32Array([0.5, 0.5]));

    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as Record<string, unknown>;
    expect(Array.isArray(body.vector)).toBe(true);
    expect(body.vector).toHaveLength(2);
  });

  it('forwards k / filter / includeVectors / quality', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [] },
    });

    await search(transport, 'docs', [0.1], {
      k: 42,
      filter: { category: 'x' },
      includeVectors: true,
      quality: 'fast',
    });

    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as Record<string, unknown>;
    expect(body.top_k).toBe(42);
    expect(body.filter).toEqual({ category: 'x' });
    expect(body.include_vectors).toBe(true);
    // searchQualityToMode spreads additional fields; verify it merged
    expect(body).toHaveProperty('mode');
  });

  it('calls sparseToRest when sparseVector option is present', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [] },
    });

    await search(transport, 'docs', [0.1], {
      sparseVector: { 1: 0.5, 2: 0.3 },
    });

    expect(transport.sparseToRest).toHaveBeenCalledWith({ 1: 0.5, 2: 0.3 });
    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as Record<string, unknown>;
    expect(body.sparse_vector).toBeDefined();
  });

  it('forwards sparseIndexName as sparse_index', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [] },
    });

    await search(transport, 'docs', [0.1], {
      sparseVector: { 1: 0.9 },
      sparseIndexName: 'splade_v2',
    });

    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as Record<string, unknown>;
    expect(body.sparse_index).toBe('splade_v2');
  });

  it('omits sparse_index when sparseIndexName is not set', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [] },
    });

    await search(transport, 'docs', [0.1]);

    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as Record<string, unknown>;
    expect(body.sparse_index).toBeUndefined();
  });

  it('returns [] when data.results is missing', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      { data: {} } as TransportResponse<{ results: [] }>
    );

    const result = await search(transport, 'docs', [0.1]);
    expect(result).toEqual([]);
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(search(transport, 'missing', [0.1])).rejects.toThrow(
      CollectionNotFoundError
    );
  });
});

describe('searchBatch', () => {
  beforeEach(() => vi.clearAllMocks());

  it('POSTs to /search/batch with snake_case top_k and quality mapping', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {
        results: [
          { results: [{ id: 1, score: 0.9 }] },
          { results: [{ id: 2, score: 0.8 }] },
        ],
      },
    });

    const results = await searchBatch(transport, 'docs', [
      { vector: [0.1] },
      { vector: new Float32Array([0.2]), k: 5, quality: 'accurate' },
    ]);

    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as { searches: Array<Record<string, unknown>> };
    expect(body.searches).toHaveLength(2);
    expect(body.searches[0]!.top_k).toBe(10);
    expect(body.searches[1]!.top_k).toBe(5);
    expect(results).toHaveLength(2);
    expect(results[0]).toEqual([{ id: 1, score: 0.9 }]);
  });

  it('returns [] when data is absent', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      {} as TransportResponse<unknown>
    );

    const result = await searchBatch(transport, 'docs', [{ vector: [0.1] }]);
    expect(result).toEqual([]);
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(
      searchBatch(transport, 'missing', [{ vector: [0.1] }])
    ).rejects.toThrow(CollectionNotFoundError);
  });
});

describe('textSearch', () => {
  beforeEach(() => vi.clearAllMocks());

  it('POSTs to /search/text with query, default top_k=10 and optional filter', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [{ id: 1, score: 0.7 }] },
    });

    await textSearch(transport, 'docs', 'hello world');

    const call = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]!;
    expect(call[1]).toBe('/collections/docs/search/text');
    expect(call[2]).toEqual({
      query: 'hello world',
      top_k: 10,
      filter: undefined,
    });
  });

  it('forwards k and filter', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [] },
    });

    await textSearch(transport, 'docs', 'q', { k: 3, filter: { f: 1 } });
    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as Record<string, unknown>;
    expect(body.top_k).toBe(3);
    expect(body.filter).toEqual({ f: 1 });
  });

  it('returns [] when data.results is missing', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      { data: {} } as TransportResponse<unknown>
    );

    const result = await textSearch(transport, 'docs', 'q');
    expect(result).toEqual([]);
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(textSearch(transport, 'missing', 'q')).rejects.toThrow(
      CollectionNotFoundError
    );
  });
});

describe('hybridSearch', () => {
  beforeEach(() => vi.clearAllMocks());

  it('POSTs to /search/hybrid with vector_weight default 0.5', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [] },
    });

    await hybridSearch(transport, 'docs', [0.1], 'text');

    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as Record<string, unknown>;
    expect(body.vector_weight).toBe(0.5);
    expect(body.top_k).toBe(10);
    expect(body.query).toBe('text');
  });

  it('forwards k / vectorWeight / filter and normalises Float32Array', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [] },
    });

    await hybridSearch(
      transport,
      'docs',
      new Float32Array([0.3, 0.7]),
      'q',
      { k: 5, vectorWeight: 0.8, filter: { a: 1 } }
    );

    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as Record<string, unknown>;
    expect(body.top_k).toBe(5);
    expect(body.vector_weight).toBe(0.8);
    expect(body.filter).toEqual({ a: 1 });
    expect(Array.isArray(body.vector)).toBe(true);
  });

  it('returns [] when data is missing', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      { data: {} } as TransportResponse<unknown>
    );

    const result = await hybridSearch(transport, 'docs', [0.1], 'q');
    expect(result).toEqual([]);
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(hybridSearch(transport, 'missing', [0.1], 'q')).rejects.toThrow(
      CollectionNotFoundError
    );
  });
});

describe('multiQuerySearch', () => {
  beforeEach(() => vi.clearAllMocks());

  it('POSTs to /search/multi with defaults strategy=rrf and rrf_k=60', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [] },
    });

    await multiQuerySearch(transport, 'docs', [[0.1], [0.2]]);

    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as Record<string, unknown>;
    expect(body.strategy).toBe('rrf');
    expect(body.rrf_k).toBe(60);
    expect(body.top_k).toBe(10);
    expect(body.vectors).toEqual([[0.1], [0.2]]);
  });

  it('forwards fusion / fusionParams / filter', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [{ id: 1, score: 1 }] },
    });

    const result = await multiQuerySearch(
      transport,
      'docs',
      [new Float32Array([0.1]), [0.2]],
      {
        k: 5,
        fusion: 'avg',
        fusionParams: {
          k: 99,
          avgWeight: 0.3,
          maxWeight: 0.7,
          hitWeight: 0.5,
        },
        filter: { a: 1 },
      }
    );

    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as Record<string, unknown>;
    expect(body.strategy).toBe('avg');
    expect(body.rrf_k).toBe(99);
    expect(body.avg_weight).toBe(0.3);
    expect(body.max_weight).toBe(0.7);
    expect(body.hit_weight).toBe(0.5);
    expect(body.filter).toEqual({ a: 1 });
    expect(result).toEqual([{ id: 1, score: 1 }]);
  });

  it('forwards denseWeight and sparseWeight for relative_score fusion', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [] },
    });

    await multiQuerySearch(transport, 'docs', [[0.1], [0.2]], {
      fusion: 'relative_score',
      fusionParams: { denseWeight: 0.7, sparseWeight: 0.3 },
    });

    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as Record<string, unknown>;
    expect(body.strategy).toBe('relative_score');
    expect(body.dense_weight).toBe(0.7);
    expect(body.sparse_weight).toBe(0.3);
  });

  it('returns [] when data.results is missing', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      { data: {} } as TransportResponse<unknown>
    );

    const result = await multiQuerySearch(transport, 'docs', [[0.1]]);
    expect(result).toEqual([]);
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(
      multiQuerySearch(transport, 'missing', [[0.1]])
    ).rejects.toThrow(CollectionNotFoundError);
  });
});

describe('multiQuerySearchIds', () => {
  beforeEach(() => vi.clearAllMocks());

  it('POSTs to /search/multi/ids with defaults strategy=rrf and rrf_k=60', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [{ id: 1, score: 0.9 }] },
    });

    const result = await multiQuerySearchIds(transport, 'docs', [[0.1], [0.2]]);

    const call = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]!;
    expect(call[1]).toBe('/collections/docs/search/multi/ids');
    const body = call[2] as Record<string, unknown>;
    expect(body.strategy).toBe('rrf');
    expect(body.rrf_k).toBe(60);
    expect(body.top_k).toBe(10);
    expect(body.vectors).toEqual([[0.1], [0.2]]);
    // ids-only endpoint must not forward a filter.
    expect(body.filter).toBeUndefined();
    expect(result).toEqual([{ id: 1, score: 0.9 }]);
  });

  it('forwards fusion / fusionParams and normalises Float32Array vectors', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [] },
    });

    await multiQuerySearchIds(
      transport,
      'docs',
      [new Float32Array([0.1]), [0.2]],
      { k: 5, fusion: 'avg', fusionParams: { avgWeight: 0.3, maxWeight: 0.7, hitWeight: 0.5 } }
    );

    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as Record<string, unknown>;
    expect(body.strategy).toBe('avg');
    expect(body.top_k).toBe(5);
    expect(body.avg_weight).toBe(0.3);
    expect(Array.isArray(body.vectors)).toBe(true);
  });

  it('returns [] when data.results is missing', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      { data: {} } as TransportResponse<unknown>
    );

    const result = await multiQuerySearchIds(transport, 'docs', [[0.1]]);
    expect(result).toEqual([]);
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(
      multiQuerySearchIds(transport, 'missing', [[0.1]])
    ).rejects.toThrow(CollectionNotFoundError);
  });
});

describe('searchIds', () => {
  beforeEach(() => vi.clearAllMocks());

  it('POSTs to /search/ids with defaults', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [{ id: 1, score: 0.9 }] },
    });

    const result = await searchIds(transport, 'docs', [0.1]);

    const call = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]!;
    expect(call[1]).toBe('/collections/docs/search/ids');
    const body = call[2] as Record<string, unknown>;
    expect(body.top_k).toBe(10);
    expect(result).toEqual([{ id: 1, score: 0.9 }]);
  });

  it('forwards k / filter / quality', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [] },
    });

    await searchIds(transport, 'docs', new Float32Array([0.1]), {
      k: 3,
      filter: { a: 1 },
      quality: 'accurate',
    });

    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as Record<string, unknown>;
    expect(body.top_k).toBe(3);
    expect(body.filter).toEqual({ a: 1 });
    expect(Array.isArray(body.vector)).toBe(true);
  });

  it('returns [] when data.results is missing', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      { data: {} } as TransportResponse<unknown>
    );

    const result = await searchIds(transport, 'docs', [0.1]);
    expect(result).toEqual([]);
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(searchIds(transport, 'missing', [0.1])).rejects.toThrow(
      CollectionNotFoundError
    );
  });
});

describe('sparseSearchNamed', () => {
  beforeEach(() => vi.clearAllMocks());

  it('POSTs to /search with sparse_vectors keyed by index name (defaults)', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [{ id: 1, score: 0.8 }] },
    });

    const result = await sparseSearchNamed(transport, 'docs', { 5: 0.9 }, 'splade');

    const call = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]!;
    expect(call[0]).toBe('POST');
    expect(call[1]).toBe('/collections/docs/search');
    const body = call[2] as Record<string, unknown>;
    expect(body.sparse_vectors).toEqual({ splade: { 5: 0.9 } });
    expect(body.sparse_index).toBe('splade');
    expect(body.top_k).toBe(10);
    expect(body.vector).toBeUndefined();
    expect(result).toEqual([{ id: 1, score: 0.8 }]);
  });

  it('forwards k / filter / quality and a dense vector for hybrid mode', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { results: [] },
    });

    await sparseSearchNamed(transport, 'docs', { 5: 0.9 }, 'splade', {
      k: 3,
      filter: { a: 1 },
      quality: 'accurate',
      vector: new Float32Array([0.25]),
    });

    const body = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]![2] as Record<string, unknown>;
    expect(body.top_k).toBe(3);
    expect(body.filter).toEqual({ a: 1 });
    expect(body.mode).toBe('accurate');
    expect(body.vector).toEqual([0.25]);
  });

  it('returns [] when data.results is missing', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      { data: {} } as TransportResponse<unknown>
    );

    const result = await sparseSearchNamed(transport, 'docs', { 1: 1 }, 'idx');
    expect(result).toEqual([]);
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(
      sparseSearchNamed(transport, 'missing', { 1: 1 }, 'idx')
    ).rejects.toThrow(CollectionNotFoundError);
  });
});
