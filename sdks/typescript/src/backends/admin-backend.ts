/**
 * Admin Backend operations for VelesDB REST API.
 *
 * Extracted from rest.ts to keep file size manageable.
 * Implements: getCollectionStats, analyzeCollection, getCollectionConfig.
 */

import type {
  CollectionStatsResponse,
  CollectionConfigResponse,
  ColumnStatsDetail,
  DistanceMetric,
  StorageMode,
} from '../types';
import type { BaseTransport } from './shared';
import { throwOnError, returnNullOnNotFound, collectionPath } from './shared';

/** Minimal transport interface for admin operations. */
export type AdminTransport = BaseTransport;

/** Raw stats shape returned by the REST API. */
interface StatsApiResponse {
  total_points: number;
  total_size_bytes: number;
  row_count: number;
  deleted_count: number;
  avg_row_size_bytes: number;
  payload_size_bytes: number;
  last_analyzed_epoch_ms: number;
  column_stats?: Record<string, {
    name: string;
    null_count: number;
    distinct_count: number;
    min_value: unknown | null;
    max_value: unknown | null;
    avg_size_bytes: number;
    histogram_buckets: number | null;
    histogram_stale: boolean | null;
  }>;
}

export function mapStatsResponse(data: StatsApiResponse): CollectionStatsResponse {
  let columnStats: Record<string, ColumnStatsDetail> | undefined;

  if (data.column_stats) {
    columnStats = {};
    for (const [key, col] of Object.entries(data.column_stats)) {
      columnStats[key] = {
        name: col.name,
        nullCount: col.null_count,
        distinctCount: col.distinct_count,
        minValue: col.min_value,
        maxValue: col.max_value,
        avgSizeBytes: col.avg_size_bytes,
        histogramBuckets: col.histogram_buckets,
        histogramStale: col.histogram_stale,
      };
    }
  }

  return {
    totalPoints: data.total_points,
    totalSizeBytes: data.total_size_bytes,
    rowCount: data.row_count,
    deletedCount: data.deleted_count,
    avgRowSizeBytes: data.avg_row_size_bytes,
    payloadSizeBytes: data.payload_size_bytes,
    lastAnalyzedEpochMs: data.last_analyzed_epoch_ms,
    columnStats,
  };
}

export async function getCollectionStats(
  transport: AdminTransport,
  collection: string
): Promise<CollectionStatsResponse | null> {
  const response = await transport.requestJson<StatsApiResponse>(
    'GET',
    `${collectionPath(collection)}/stats`
  );

  if (returnNullOnNotFound(response)) {
    return null;
  }

  return mapStatsResponse(response.data!);
}

export async function analyzeCollection(
  transport: AdminTransport,
  collection: string
): Promise<CollectionStatsResponse> {
  const response = await transport.requestJson<StatsApiResponse>(
    'POST',
    `${collectionPath(collection)}/analyze`
  );

  throwOnError(response, `Collection '${collection}'`);

  return mapStatsResponse(response.data!);
}

/** Raw wire shape of `/collections/{name}/config` — matches `velesdb_core::api_types::CollectionConfigResponse`. */
interface CollectionConfigWire {
  name: string;
  dimension: number;
  metric: string;
  storage_mode: string;
  point_count: number;
  metadata_only: boolean;
  graph_schema?: Record<string, unknown>;
  embedding_dimension?: number;
  schema_version?: number;
  pq_rescore_oversampling?: number;
  hnsw_params?: Record<string, unknown>;
  deferred_indexing?: Record<string, unknown>;
  async_index_builder?: Record<string, unknown>;
}

/**
 * Map a raw `CollectionConfigWire` (snake_case from REST) into the
 * camelCase `CollectionConfigResponse` exposed to TS callers.
 * Kept separate from `getCollectionConfig` so the mapping stays
 * unit-testable and the HTTP function remains under the NLOC limit.
 */
function mapConfigResponse(data: CollectionConfigWire): CollectionConfigResponse {
  return {
    name: data.name,
    dimension: data.dimension,
    metric: data.metric as DistanceMetric,
    storageMode: data.storage_mode as StorageMode,
    pointCount: data.point_count,
    metadataOnly: data.metadata_only,
    graphSchema: data.graph_schema,
    embeddingDimension: data.embedding_dimension,
    schemaVersion: data.schema_version,
    pqRescoreOversampling: data.pq_rescore_oversampling,
    hnswParams: data.hnsw_params,
    deferredIndexing: data.deferred_indexing,
    asyncIndexBuilder: data.async_index_builder,
  };
}

export async function getCollectionConfig(
  transport: AdminTransport,
  collection: string
): Promise<CollectionConfigResponse> {
  const response = await transport.requestJson<CollectionConfigWire>(
    'GET',
    `${collectionPath(collection)}/config`
  );

  throwOnError(response, `Collection '${collection}'`);

  return mapConfigResponse(response.data!);
}
