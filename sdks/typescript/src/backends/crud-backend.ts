/**
 * CRUD Backend operations for VelesDB REST API.
 *
 * Extracted from rest.ts to keep file size manageable.
 * Implements: createCollection, deleteCollection, getCollection,
 * listCollections, upsert, upsertBatch, delete, get, isEmpty, flush.
 */

import type {
  CollectionConfig,
  Collection,
  VectorDocument,
  RestPointId,
  SparseVector,
} from '../types';
import { ValidationError } from '../types';
import type { BaseTransport } from './shared';
import {
  throwOnError,
  returnNullOnNotFound,
  collectionPath,
  toNumberArray,
} from './shared';

/** Minimal transport interface for CRUD operations. */
export type CrudTransport = BaseTransport;

/** Largest value a u64 point id can take (`u64::MAX`). */
const U64_MAX = 18446744073709551615n;

/**
 * Coerce a decimal-string id for the shared gate in {@link parseRestPointId}.
 *
 * Only a plain run of digits is accepted — no sign, whitespace, decimal
 * point, exponent, or hex — so '' / '  ' / '1e3' / '0x10' map to `NaN`
 * (rejected by the caller) instead of silently coercing (`Number('')` would
 * otherwise become 0). Digit-strings within the JS safe-integer range become
 * numbers; digit-strings in (2^53-1, u64::MAX] are returned verbatim so the
 * exact decimal value survives the JavaScript boundary without precision
 * loss. Digit-strings above `u64::MAX` map to `NaN` (rejected).
 */
function coerceDecimalStringId(id: string): number | string {
  if (!/^\d+$/.test(id)) return NaN;
  const big = BigInt(id);
  if (big > U64_MAX) return NaN;
  return big > BigInt(Number.MAX_SAFE_INTEGER) ? id : Number(id);
}

/**
 * Single validation gate for REST point ids — used by the CRUD/streaming
 * backends, the client layer (`validateRestPointId`) and the agent-memory
 * helpers.
 *
 * Numeric ids must be non-negative integers within the JS safe-integer range.
 * Decimal-string ids (e.g. the u64-safe strings returned by the agent-memory
 * record/learn helpers) are coerced to numbers when exactly representable and
 * kept as verbatim strings above 2^53-1: since #1004 the server deserialises
 * point ids in request bodies from either JSON numbers or strings, and path
 * params (`/points/{id}`) parse the full u64 range, so every id that
 * `storeFact` / `recordEvent` / `learnProcedure` accepts round-trips through
 * `get` / `delete` symmetrically.
 */
export function parseRestPointId(id: string | number): RestPointId {
  const coerced = typeof id === 'string' ? coerceDecimalStringId(id) : id;
  if (typeof coerced === 'string') {
    // Precision-critical decimal string — keep it verbatim on the wire.
    return coerced;
  }
  if (
    !Number.isFinite(coerced) ||
    coerced < 0 ||
    !Number.isInteger(coerced) ||
    coerced > Number.MAX_SAFE_INTEGER
  ) {
    throw new ValidationError(
      `REST backend requires numeric u64-compatible IDs: a non-negative integer in the JS safe integer range (0..${Number.MAX_SAFE_INTEGER}) or a decimal string up to u64::MAX (${U64_MAX}). Received: ${String(id)}`
    );
  }
  return coerced;
}

export function sparseVectorToRestFormat(sv: SparseVector): Record<string, number> {
  const result: Record<string, number> = {};
  for (const [k, v] of Object.entries(sv)) {
    result[String(k)] = v;
  }
  return result;
}

/**
 * Convert a TypeScript `DeferredIndexerOptions` into the snake_case JSON
 * shape expected by `velesdb_core::collection::streaming::DeferredIndexerConfig`.
 * Returns `undefined` when the caller did not supply the option, so the
 * field is dropped from the request body entirely.
 */
function toDeferredIndexingWire(
  opts: CollectionConfig['deferredIndexing']
): Record<string, unknown> | undefined {
  if (!opts) return undefined;
  const wire: Record<string, unknown> = {};
  if (opts.enabled !== undefined) wire.enabled = opts.enabled;
  if (opts.mergeThreshold !== undefined) wire.merge_threshold = opts.mergeThreshold;
  if (opts.maxBufferAgeMs !== undefined) wire.max_buffer_age_ms = opts.maxBufferAgeMs;
  return wire;
}

/**
 * Convert a TypeScript `AsyncIndexBuilderOptions` into the snake_case JSON
 * shape expected by `velesdb_core::collection::streaming::AsyncIndexBuilderConfig`.
 */
function toAsyncIndexBuilderWire(
  opts: CollectionConfig['asyncIndexBuilder']
): Record<string, unknown> | undefined {
  if (!opts) return undefined;
  const wire: Record<string, unknown> = {};
  if (opts.mergeThreshold !== undefined) wire.merge_threshold = opts.mergeThreshold;
  if (opts.segmentCount !== undefined) wire.segment_count = opts.segmentCount;
  return wire;
}

export async function createCollection(
  transport: CrudTransport,
  name: string,
  config: CollectionConfig
): Promise<void> {
  const body: Record<string, unknown> = {
    name,
    dimension: config.dimension,
    metric: config.metric ?? 'cosine',
    storage_mode: config.storageMode ?? 'full',
    collection_type: config.collectionType ?? 'vector',
    description: config.description,
    hnsw_m: config.hnsw?.m,
    hnsw_ef_construction: config.hnsw?.efConstruction,
    hnsw_alpha: config.hnsw?.alpha,
    hnsw_max_elements: config.hnsw?.maxElements,
  };

  // Advanced options — omit the key entirely when undefined so
  // `JSON.stringify` produces a minimal payload and the server falls
  // back to defaults.
  if (config.pqRescoreOversampling !== undefined) {
    body.pq_rescore_oversampling = config.pqRescoreOversampling;
  }
  const deferredWire = toDeferredIndexingWire(config.deferredIndexing);
  if (deferredWire !== undefined) {
    body.deferred_indexing = deferredWire;
  }
  const asyncWire = toAsyncIndexBuilderWire(config.asyncIndexBuilder);
  if (asyncWire !== undefined) {
    body.async_index_builder = asyncWire;
  }

  const response = await transport.requestJson('POST', '/collections', body);
  throwOnError(response);
}

export async function deleteCollection(
  transport: CrudTransport,
  name: string
): Promise<void> {
  const response = await transport.requestJson(
    'DELETE',
    collectionPath(name)
  );
  throwOnError(response, `Collection '${name}'`);
}

export async function getCollection(
  transport: CrudTransport,
  name: string
): Promise<Collection | null> {
  const response = await transport.requestJson<Collection>(
    'GET',
    collectionPath(name)
  );
  if (returnNullOnNotFound(response)) {
    return null;
  }
  return response.data ?? null;
}

export async function listCollections(
  transport: CrudTransport
): Promise<Collection[]> {
  const response = await transport.requestJson<Collection[]>('GET', '/collections');
  throwOnError(response);
  return response.data ?? [];
}

export async function upsert(
  transport: CrudTransport,
  collection: string,
  doc: VectorDocument
): Promise<void> {
  const restId = parseRestPointId(doc.id);
  const vector = toNumberArray(doc.vector);

  const response = await transport.requestJson(
    'POST',
    `${collectionPath(collection)}/points`,
    { points: [{ id: restId, vector, payload: doc.payload }] }
  );
  throwOnError(response, `Collection '${collection}'`);
}

export async function upsertBatch(
  transport: CrudTransport,
  collection: string,
  docs: VectorDocument[]
): Promise<void> {
  const vectors = docs.map(doc => ({
    id: parseRestPointId(doc.id),
    vector: toNumberArray(doc.vector),
    payload: doc.payload,
  }));

  const response = await transport.requestJson(
    'POST',
    `${collectionPath(collection)}/points`,
    { points: vectors }
  );
  throwOnError(response, `Collection '${collection}'`);
}

export async function deletePoint(
  transport: CrudTransport,
  collection: string,
  id: string | number
): Promise<boolean> {
  const restId = parseRestPointId(id);
  const response = await transport.requestJson<{ deleted: boolean }>(
    'DELETE',
    `${collectionPath(collection)}/points/${encodeURIComponent(String(restId))}`
  );
  if (returnNullOnNotFound(response)) {
    return false;
  }
  return response.data?.deleted ?? false;
}

export async function get(
  transport: CrudTransport,
  collection: string,
  id: string | number
): Promise<VectorDocument | null> {
  const restId = parseRestPointId(id);
  const response = await transport.requestJson<VectorDocument>(
    'GET',
    `${collectionPath(collection)}/points/${encodeURIComponent(String(restId))}`
  );
  if (returnNullOnNotFound(response)) {
    return null;
  }
  return response.data ?? null;
}

export async function isEmpty(
  transport: CrudTransport,
  collection: string
): Promise<boolean> {
  const response = await transport.requestJson<{ is_empty: boolean }>(
    'GET',
    `${collectionPath(collection)}/empty`
  );
  throwOnError(response, `Collection '${collection}'`);
  return response.data?.is_empty ?? true;
}

export async function flush(
  transport: CrudTransport,
  collection: string
): Promise<void> {
  const response = await transport.requestJson(
    'POST',
    `${collectionPath(collection)}/flush`
  );
  throwOnError(response, `Collection '${collection}'`);
}
