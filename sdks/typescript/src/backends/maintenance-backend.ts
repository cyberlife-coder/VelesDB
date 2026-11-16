/**
 * Maintenance, diagnostics and health operations for the VelesDB REST API.
 *
 * Covers the server routes that the parity audit found unexposed:
 * `POST /collections/{name}/vacuum`, `POST .../compact`,
 * `POST .../locality/reorder`, `GET .../diagnostics`, plus the ops probes
 * `GET /health` and `GET /ready`.
 */

import type {
  HealthStatus,
  VacuumResult,
  CompactResult,
  ReorderResult,
  CollectionDiagnostics,
} from '../types';
import type { BaseTransport } from './shared';
import { throwOnError, collectionPath } from './shared';

/** Minimal transport interface for maintenance operations. */
export type MaintenanceTransport = BaseTransport;

/** Raw wire shapes (snake_case) of the maintenance endpoints. */
interface VacuumWire {
  message: string;
  collection: string;
  compacted_entries: number;
}

interface CompactWire {
  message: string;
  collection: string;
  bytes_reclaimed: number;
}

interface ReorderWire {
  message: string;
  collection: string;
}

interface DiagnosticsWire {
  has_vectors: boolean;
  search_ready: boolean;
  dimension_configured: boolean;
  point_count: number;
  index_health: string;
  index_health_detail?: string | null;
}

/** `GET /health` — liveness probe. */
export async function healthCheck(
  transport: MaintenanceTransport
): Promise<HealthStatus> {
  const response = await transport.requestJson<HealthStatus>('GET', '/health');
  throwOnError(response);
  return response.data!;
}

/**
 * `GET /ready` — readiness probe.
 *
 * A not-ready server answers `503`, which the transport maps to a typed
 * `SERVICE_UNAVAILABLE` error; the body still carries
 * `{ status: "not_ready", version }`, so the error message names the status
 * when the server provided one.
 */
export async function readyCheck(
  transport: MaintenanceTransport
): Promise<HealthStatus> {
  const response = await transport.requestJson<HealthStatus>('GET', '/ready');
  throwOnError(response);
  return response.data!;
}

/** `POST /collections/{name}/vacuum` — compact tombstoned index entries. */
export async function vacuumCollection(
  transport: MaintenanceTransport,
  collection: string
): Promise<VacuumResult> {
  const response = await transport.requestJson<VacuumWire>(
    'POST',
    `${collectionPath(collection)}/vacuum`
  );
  throwOnError(response, `Collection '${collection}'`);
  const data = response.data!;
  return {
    message: data.message,
    collection: data.collection,
    compactedEntries: data.compacted_entries,
  };
}

/** `POST /collections/{name}/compact` — rewrite vector storage contiguously. */
export async function compactCollection(
  transport: MaintenanceTransport,
  collection: string
): Promise<CompactResult> {
  const response = await transport.requestJson<CompactWire>(
    'POST',
    `${collectionPath(collection)}/compact`
  );
  throwOnError(response, `Collection '${collection}'`);
  const data = response.data!;
  return {
    message: data.message,
    collection: data.collection,
    bytesReclaimed: data.bytes_reclaimed,
  };
}

/** `POST /collections/{name}/locality/reorder` — cache-locality reordering. */
export async function reorderForLocality(
  transport: MaintenanceTransport,
  collection: string
): Promise<ReorderResult> {
  const response = await transport.requestJson<ReorderWire>(
    'POST',
    `${collectionPath(collection)}/locality/reorder`
  );
  throwOnError(response, `Collection '${collection}'`);
  const data = response.data!;
  return { message: data.message, collection: data.collection };
}

/** `GET /collections/{name}/diagnostics` — index health snapshot. */
export async function collectionDiagnostics(
  transport: MaintenanceTransport,
  collection: string
): Promise<CollectionDiagnostics> {
  const response = await transport.requestJson<DiagnosticsWire>(
    'GET',
    `${collectionPath(collection)}/diagnostics`
  );
  throwOnError(response, `Collection '${collection}'`);
  const data = response.data!;
  return {
    hasVectors: data.has_vectors,
    searchReady: data.search_ready,
    dimensionConfigured: data.dimension_configured,
    pointCount: data.point_count,
    indexHealth: data.index_health,
    indexHealthDetail: data.index_health_detail ?? undefined,
  };
}
