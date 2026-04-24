/**
 * Admin Backend Tests (#598)
 *
 * Covers `src/backends/admin-backend.ts`: getCollectionStats,
 * analyzeCollection, getCollectionConfig, and the `mapStatsResponse`
 * export. Exercises happy paths, snake_case to camelCase mapping of
 * the column-stats hashmap, the `returnNullOnNotFound` sentinel on
 * GET /stats, and typed error routing on POST /analyze + GET /config.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  getCollectionStats,
  analyzeCollection,
  getCollectionConfig,
  mapStatsResponse,
  type AdminTransport,
} from '../src/backends/admin-backend';
import type { TransportResponse } from '../src/backends/shared';
import { CollectionNotFoundError } from '../src/errors';

function buildTransport(
  overrides: Partial<AdminTransport> = {}
): AdminTransport {
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

const baseStats = {
  total_points: 100,
  total_size_bytes: 1024,
  row_count: 99,
  deleted_count: 1,
  avg_row_size_bytes: 10,
  payload_size_bytes: 512,
  last_analyzed_epoch_ms: 1_700_000_000_000,
};

describe('mapStatsResponse', () => {
  it('maps snake_case top-level fields to camelCase', () => {
    const mapped = mapStatsResponse({ ...baseStats });
    expect(mapped).toEqual({
      totalPoints: 100,
      totalSizeBytes: 1024,
      rowCount: 99,
      deletedCount: 1,
      avgRowSizeBytes: 10,
      payloadSizeBytes: 512,
      lastAnalyzedEpochMs: 1_700_000_000_000,
      columnStats: undefined,
    });
  });

  it('maps the column_stats hashmap entries to camelCase', () => {
    const mapped = mapStatsResponse({
      ...baseStats,
      column_stats: {
        age: {
          name: 'age',
          null_count: 2,
          distinct_count: 50,
          min_value: 18,
          max_value: 99,
          avg_size_bytes: 4,
          histogram_buckets: 10,
          histogram_stale: false,
        },
        email: {
          name: 'email',
          null_count: 0,
          distinct_count: 100,
          min_value: null,
          max_value: null,
          avg_size_bytes: 32,
          histogram_buckets: null,
          histogram_stale: null,
        },
      },
    });

    expect(mapped.columnStats).toEqual({
      age: {
        name: 'age',
        nullCount: 2,
        distinctCount: 50,
        minValue: 18,
        maxValue: 99,
        avgSizeBytes: 4,
        histogramBuckets: 10,
        histogramStale: false,
      },
      email: {
        name: 'email',
        nullCount: 0,
        distinctCount: 100,
        minValue: null,
        maxValue: null,
        avgSizeBytes: 32,
        histogramBuckets: null,
        histogramStale: null,
      },
    });
  });
});

describe('getCollectionStats', () => {
  beforeEach(() => vi.clearAllMocks());

  it('GETs /collections/{name}/stats and maps the payload', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: baseStats,
    } satisfies TransportResponse<unknown>);

    const result = await getCollectionStats(transport, 'docs');

    expect(transport.requestJson).toHaveBeenCalledWith(
      'GET',
      '/collections/docs/stats'
    );
    expect(result).not.toBeNull();
    expect(result!.totalPoints).toBe(100);
  });

  it('URL-encodes collection name with special chars', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: baseStats,
    });

    await getCollectionStats(transport, 'my docs/v2');

    const call = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]!;
    expect(call[1]).toBe('/collections/my%20docs%2Fv2/stats');
  });

  it('returns null on VELES-002 (collection not found)', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    const result = await getCollectionStats(transport, 'missing');
    expect(result).toBeNull();
  });

  it('returns null on legacy NOT_FOUND status code', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      error: { code: 'NOT_FOUND', message: 'no' },
    });

    const result = await getCollectionStats(transport, 'missing');
    expect(result).toBeNull();
  });

  it('rethrows non-not-found errors', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      error: { code: 'VELES-999', message: 'boom' },
    });

    await expect(getCollectionStats(transport, 'docs')).rejects.toThrow(/boom/);
  });
});

describe('analyzeCollection', () => {
  beforeEach(() => vi.clearAllMocks());

  it('POSTs to /collections/{name}/analyze and maps the payload', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: baseStats,
    });

    const result = await analyzeCollection(transport, 'docs');

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/docs/analyze'
    );
    expect(result.totalPoints).toBe(100);
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(analyzeCollection(transport, 'missing')).rejects.toThrow(
      CollectionNotFoundError
    );
  });
});

describe('getCollectionConfig', () => {
  beforeEach(() => vi.clearAllMocks());

  const wireConfig = {
    name: 'docs',
    dimension: 768,
    metric: 'cosine',
    storage_mode: 'InMemory',
    point_count: 42,
    metadata_only: false,
    graph_schema: { nodes: ['Person'] },
    embedding_dimension: 768,
    schema_version: 1,
    pq_rescore_oversampling: 2,
    hnsw_params: { ef: 64 },
    deferred_indexing: { enabled: true },
    async_index_builder: { workers: 2 },
  };

  it('GETs /collections/{name}/config and maps snake_case to camelCase', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: wireConfig,
    });

    const result = await getCollectionConfig(transport, 'docs');

    expect(transport.requestJson).toHaveBeenCalledWith(
      'GET',
      '/collections/docs/config'
    );
    expect(result).toEqual({
      name: 'docs',
      dimension: 768,
      metric: 'cosine',
      storageMode: 'InMemory',
      pointCount: 42,
      metadataOnly: false,
      graphSchema: { nodes: ['Person'] },
      embeddingDimension: 768,
      schemaVersion: 1,
      pqRescoreOversampling: 2,
      hnswParams: { ef: 64 },
      deferredIndexing: { enabled: true },
      asyncIndexBuilder: { workers: 2 },
    });
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(getCollectionConfig(transport, 'missing')).rejects.toThrow(
      CollectionNotFoundError
    );
  });

  it('maps a minimal response without the optional fields', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {
        name: 'docs',
        dimension: 128,
        metric: 'euclidean',
        storage_mode: 'Mmap',
        point_count: 0,
        metadata_only: true,
      },
    });

    const result = await getCollectionConfig(transport, 'docs');
    expect(result.metadataOnly).toBe(true);
    expect(result.graphSchema).toBeUndefined();
    expect(result.embeddingDimension).toBeUndefined();
    expect(result.hnswParams).toBeUndefined();
  });
});
