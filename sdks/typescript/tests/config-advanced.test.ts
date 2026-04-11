/**
 * Advanced CollectionConfig Tests (Sprint 2 Wave 4 — #18 PROP-CONFIG-ADVANCED)
 *
 * Verifies that the TypeScript SDK exposes every advanced create-time
 * option accepted by `velesdb-core::api_types::requests::CreateCollectionRequest`
 * and surfaces every advanced field returned by
 * `velesdb-core::api_types::responses::CollectionConfigResponse`.
 *
 * Checked fields:
 * - Create-time: pqRescoreOversampling, deferredIndexing, asyncIndexBuilder
 * - Read-time: schemaVersion, pqRescoreOversampling, hnswParams,
 *              deferredIndexing, asyncIndexBuilder
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { RestBackend } from '../src/backends/rest';
import type {
  CollectionConfig,
  CollectionConfigResponse,
  DeferredIndexerOptions,
  AsyncIndexBuilderOptions,
} from '../src/types';

// Minimal fetch mock following the pattern used in rest-backend.test.ts
const mockFetch = vi.fn();
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).fetch = mockFetch;

describe('CreateCollectionRequest — advanced fields forwarded to REST', () => {
  let backend: RestBackend;

  beforeEach(async () => {
    mockFetch.mockReset();
    backend = new RestBackend('http://localhost:8080');
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ status: 'ok' }),
    });
    await backend.init();
    mockFetch.mockReset();
  });

  it('forwards pqRescoreOversampling', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({}),
    });

    const config: CollectionConfig = {
      dimension: 768,
      storageMode: 'pq',
      pqRescoreOversampling: 8,
    };
    await backend.createCollection('docs', config);

    const call = mockFetch.mock.calls[0];
    const body = JSON.parse(call[1].body as string);
    expect(body.pq_rescore_oversampling).toBe(8);
  });

  it('forwards deferredIndexing as snake_case JSON object', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({}),
    });

    const deferred: DeferredIndexerOptions = {
      enabled: true,
      mergeThreshold: 5000,
      maxBufferAgeMs: 30_000,
    };
    const config: CollectionConfig = {
      dimension: 384,
      deferredIndexing: deferred,
    };
    await backend.createCollection('streams', config);

    const body = JSON.parse(mockFetch.mock.calls[0][1].body as string);
    expect(body.deferred_indexing).toEqual({
      enabled: true,
      merge_threshold: 5000,
      max_buffer_age_ms: 30_000,
    });
  });

  it('forwards asyncIndexBuilder as snake_case JSON object', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({}),
    });

    const asyncBuilder: AsyncIndexBuilderOptions = {
      mergeThreshold: 50_000,
      segmentCount: 8,
    };
    const config: CollectionConfig = {
      dimension: 1536,
      asyncIndexBuilder: asyncBuilder,
    };
    await backend.createCollection('bulk', config);

    const body = JSON.parse(mockFetch.mock.calls[0][1].body as string);
    expect(body.async_index_builder).toEqual({
      merge_threshold: 50_000,
      segment_count: 8,
    });
  });

  it('omits advanced fields from the request body when not provided', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({}),
    });

    const config: CollectionConfig = { dimension: 128 };
    await backend.createCollection('basic', config);

    const body = JSON.parse(mockFetch.mock.calls[0][1].body as string);
    // Must NOT be present (undefined → JSON.stringify drops the key)
    expect(body).not.toHaveProperty('pq_rescore_oversampling');
    expect(body).not.toHaveProperty('deferred_indexing');
    expect(body).not.toHaveProperty('async_index_builder');
  });

  it('handles asyncIndexBuilder with segmentCount=undefined (default to cpu count)', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({}),
    });

    const config: CollectionConfig = {
      dimension: 128,
      asyncIndexBuilder: { mergeThreshold: 20_000 },
    };
    await backend.createCollection('no-seg', config);

    const body = JSON.parse(mockFetch.mock.calls[0][1].body as string);
    // segment_count omitted — server applies the num_cpus default
    expect(body.async_index_builder).toEqual({ merge_threshold: 20_000 });
    expect(body.async_index_builder.segment_count).toBeUndefined();
  });
});

// ============================================================================
// CollectionConfigResponse — advanced read-time fields surfaced camelCase
// ============================================================================

describe('CollectionConfigResponse — advanced fields parsed from REST', () => {
  let backend: RestBackend;

  beforeEach(async () => {
    mockFetch.mockReset();
    backend = new RestBackend('http://localhost:8080');
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ status: 'ok' }),
    });
    await backend.init();
    mockFetch.mockReset();
  });

  it('maps schema_version → schemaVersion', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          name: 'docs',
          dimension: 768,
          metric: 'cosine',
          storage_mode: 'full',
          point_count: 42,
          metadata_only: false,
          schema_version: 3,
        }),
    });

    const config: CollectionConfigResponse = await backend.getCollectionConfig('docs');
    expect(config.schemaVersion).toBe(3);
  });

  it('maps pq_rescore_oversampling → pqRescoreOversampling', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          name: 'docs',
          dimension: 768,
          metric: 'cosine',
          storage_mode: 'pq',
          point_count: 100,
          metadata_only: false,
          schema_version: 1,
          pq_rescore_oversampling: 12,
        }),
    });

    const config = await backend.getCollectionConfig('docs');
    expect(config.pqRescoreOversampling).toBe(12);
  });

  it('maps hnsw_params, deferred_indexing, async_index_builder as raw JSON objects', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          name: 'stream',
          dimension: 384,
          metric: 'cosine',
          storage_mode: 'full',
          point_count: 0,
          metadata_only: false,
          schema_version: 1,
          hnsw_params: { m: 32, ef_construction: 400 },
          deferred_indexing: {
            enabled: true,
            merge_threshold: 1000,
            max_buffer_age_ms: 60000,
          },
          async_index_builder: {
            merge_threshold: 100_000,
            segment_count: 16,
          },
        }),
    });

    const config = await backend.getCollectionConfig('stream');
    expect(config.hnswParams).toEqual({ m: 32, ef_construction: 400 });
    expect(config.deferredIndexing).toEqual({
      enabled: true,
      merge_threshold: 1000,
      max_buffer_age_ms: 60000,
    });
    expect(config.asyncIndexBuilder).toEqual({
      merge_threshold: 100_000,
      segment_count: 16,
    });
  });

  it('leaves advanced fields undefined when the server omits them', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          name: 'basic',
          dimension: 128,
          metric: 'cosine',
          storage_mode: 'full',
          point_count: 0,
          metadata_only: false,
          schema_version: 1,
          // No advanced fields in the payload
        }),
    });

    const config = await backend.getCollectionConfig('basic');
    expect(config.schemaVersion).toBe(1);
    expect(config.pqRescoreOversampling).toBeUndefined();
    expect(config.hnswParams).toBeUndefined();
    expect(config.deferredIndexing).toBeUndefined();
    expect(config.asyncIndexBuilder).toBeUndefined();
  });

  it('preserves existing fields (name, dimension, metric, storageMode, pointCount)', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          name: 'docs',
          dimension: 768,
          metric: 'euclidean',
          storage_mode: 'sq8',
          point_count: 1234,
          metadata_only: false,
          schema_version: 1,
          embedding_dimension: 768,
        }),
    });

    const config = await backend.getCollectionConfig('docs');
    expect(config.name).toBe('docs');
    expect(config.dimension).toBe(768);
    expect(config.metric).toBe('euclidean');
    expect(config.storageMode).toBe('sq8');
    expect(config.pointCount).toBe(1234);
    expect(config.embeddingDimension).toBe(768);
  });
});
