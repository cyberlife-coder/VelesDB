/**
 * Admin Backend operations for VelesDB REST API.
 *
 * Extracted from rest.ts to keep file size manageable.
 * Implements: getCollectionStats, analyzeCollection, getCollectionConfig.
 */

import type {
  CollectionStatsResponse,
  CollectionConfigResponse,
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
}

export function mapStatsResponse(data: StatsApiResponse): CollectionStatsResponse {
  return {
    totalPoints: data.total_points,
    totalSizeBytes: data.total_size_bytes,
    rowCount: data.row_count,
    deletedCount: data.deleted_count,
    avgRowSizeBytes: data.avg_row_size_bytes,
    payloadSizeBytes: data.payload_size_bytes,
    lastAnalyzedEpochMs: data.last_analyzed_epoch_ms,
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

export async function getCollectionConfig(
  transport: AdminTransport,
  collection: string
): Promise<CollectionConfigResponse> {
  const response = await transport.requestJson<{
    name: string;
    dimension: number;
    metric: string;
    storage_mode: string;
    point_count: number;
    metadata_only: boolean;
    graph_schema?: Record<string, unknown>;
    embedding_dimension?: number;
  }>('GET', `${collectionPath(collection)}/config`);

  throwOnError(response, `Collection '${collection}'`);

  const data = response.data!;
  return {
    name: data.name,
    dimension: data.dimension,
    metric: data.metric,
    storageMode: data.storage_mode,
    pointCount: data.point_count,
    metadataOnly: data.metadata_only,
    graphSchema: data.graph_schema,
    embeddingDimension: data.embedding_dimension,
  };
}
