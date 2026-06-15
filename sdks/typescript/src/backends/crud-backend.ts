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
import { ValidationError, ConnectionError, VelesDBError } from '../types';
import type { BaseTransport } from './shared';
import {
  throwOnError,
  returnNullOnNotFound,
  collectionPath,
  toNumberArray,
  safeJsonParse,
} from './shared';

/** Minimal transport interface for CRUD operations. */
export type CrudTransport = BaseTransport;

/**
 * Transport fields needed to send a raw binary body via `fetch`.
 *
 * The binary bulk endpoint cannot go through `BaseTransport.requestJson`
 * (which JSON-stringifies the body), so the raw-bulk sender takes the
 * connection primitives directly — mirroring the `StreamingTransport`
 * approach for the NDJSON path.
 */
export interface RawBulkTransport {
  readonly baseUrl: string;
  readonly apiKey: string | undefined;
  readonly timeout: number;
}

/** Magic prefix + fixed header size for the VRB1 binary bulk format. */
const RAW_BULK_HEADER_LEN = 16;
const RAW_BULK_MAGIC = [0x56, 0x52, 0x42, 0x31]; // "VRB1"
const RAW_BULK_ID_WIDTH = 8; // u64

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

// ---------------------------------------------------------------------------
// Binary wire-format bulk upsert (VRB1)
// ---------------------------------------------------------------------------

/**
 * Encode an `(ids, vectors)` batch into the VRB1 binary bulk format.
 *
 * Layout (little-endian, matches the Rust `upsert_points_raw` handler):
 *
 * ```text
 * magic b"VRB1" (4) | count u32 (4) | dim u32 (4) | id_width u8 (1) |
 * reserved (3) | ids [u64; count] | vectors [f32; count*dim]
 * ```
 *
 * The encoding is deterministic: identical inputs always yield identical
 * bytes. `BigUint64Array`/`Float32Array` views are written through a
 * `DataView` so the result is host-endian-independent.
 *
 * @throws {ValidationError} when `ids.length !== vectors.length`, or any
 *   vector's length differs from `dim`.
 */
export function encodeRawBulk(
  ids: number[],
  vectors: Array<number[] | Float32Array>,
  dim: number
): Uint8Array {
  const count = ids.length;
  if (vectors.length !== count) {
    throw new ValidationError(
      `encodeRawBulk: ids length (${count}) must match vectors length (${vectors.length})`
    );
  }
  const buf = new Uint8Array(RAW_BULK_HEADER_LEN + count * 8 + count * dim * 4);
  const view = new DataView(buf.buffer);
  buf.set(RAW_BULK_MAGIC, 0);
  view.setUint32(4, count, true);
  view.setUint32(8, dim, true);
  buf[12] = RAW_BULK_ID_WIDTH;
  // bytes 13..16 stay zero (reserved).
  writeIds(view, ids);
  writeVectors(view, vectors, dim, count);
  return buf;
}

/** Write packed `u64` ids starting at the fixed header offset. */
function writeIds(view: DataView, ids: number[]): void {
  let off = RAW_BULK_HEADER_LEN;
  for (const id of ids) {
    view.setBigUint64(off, BigInt(id), true);
    off += 8;
  }
}

/** Write packed row-major `f32` vectors after the id section. */
function writeVectors(
  view: DataView,
  vectors: Array<number[] | Float32Array>,
  dim: number,
  count: number
): void {
  let off = RAW_BULK_HEADER_LEN + count * 8;
  for (const vec of vectors) {
    if (vec.length !== dim) {
      throw new ValidationError(
        `encodeRawBulk: vector length (${vec.length}) must match dim (${dim})`
      );
    }
    for (let i = 0; i < dim; i++) {
      view.setFloat32(off, vec[i] ?? 0, true);
      off += 4;
    }
  }
}

/**
 * Bulk upsert points via the binary wire format
 * (`POST /collections/{name}/points/raw`, `application/octet-stream`).
 *
 * Encodes the batch with {@link encodeRawBulk} and sends it as a single raw
 * request, avoiding the per-point JSON overhead of {@link upsertBatch}.
 * Payloads are not carried on this path — use {@link upsertBatch} when you
 * need them. All ids must be plain numbers (encoded as `u64`).
 *
 * @returns the number of points the server reports as inserted.
 * @throws {VelesDBError} on a non-OK HTTP response.
 * @throws {ConnectionError} on timeout or transport failure.
 */
export async function upsertBatchRaw(
  transport: RawBulkTransport,
  collection: string,
  docs: VectorDocument[],
  dim: number
): Promise<number> {
  const ids = docs.map(d => coerceNumericId(d.id));
  const vectors = docs.map(d => d.vector);
  const body = encodeRawBulk(ids, vectors, dim);
  return sendRawBulk(transport, collection, body);
}

/** Coerce a doc id to a plain number, rejecting precision-critical strings. */
function coerceNumericId(id: string | number): number {
  const parsed = parseRestPointId(id);
  if (typeof parsed === 'string') {
    throw new ValidationError(
      `upsertBatchRaw requires ids in the JS safe integer range; received: ${parsed}`
    );
  }
  return parsed;
}

/** POST a pre-encoded binary body and parse the `{ count }` response. */
async function sendRawBulk(
  transport: RawBulkTransport,
  collection: string,
  body: Uint8Array
): Promise<number> {
  const url = `${transport.baseUrl}${collectionPath(collection)}/points/raw`;
  const headers: Record<string, string> = {
    'Content-Type': 'application/octet-stream',
  };
  if (transport.apiKey) {
    headers['Authorization'] = `Bearer ${transport.apiKey}`;
  }
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), transport.timeout);
  try {
    // `Uint8Array` is a valid `BodyInit` (ArrayBufferView) at runtime; the
    // cast satisfies the DOM lib's narrower generic `BodyInit` type.
    const response = await fetch(url, {
      method: 'POST',
      headers,
      body: body as BodyInit,
      signal: controller.signal,
    });
    clearTimeout(timeoutId);
    return await parseRawBulkResponse(response);
  } catch (error) {
    clearTimeout(timeoutId);
    throw wrapRawBulkError(error);
  }
}

/** Parse a raw-bulk HTTP response into the inserted count, throwing on error. */
async function parseRawBulkResponse(response: Response): Promise<number> {
  const data = await safeJsonParse(response);
  if (!response.ok) {
    const code = typeof data.code === 'string' ? data.code : `HTTP_${response.status}`;
    const message = typeof data.error === 'string' ? data.error : `HTTP ${response.status}`;
    throw new VelesDBError(message, code);
  }
  return typeof data.count === 'number' ? data.count : 0;
}

/** Normalise a caught raw-bulk error to a typed VelesDB error. */
function wrapRawBulkError(error: unknown): Error {
  if (error instanceof VelesDBError) {
    return error;
  }
  if (error instanceof Error && error.name === 'AbortError') {
    return new ConnectionError('Request timeout');
  }
  const message = error instanceof Error ? error.message : 'Unknown error';
  return new ConnectionError(
    `Raw bulk upsert failed: ${message}`,
    error instanceof Error ? error : undefined
  );
}
