/**
 * Shared helpers for VelesDB REST backend modules.
 *
 * Eliminates duplicated error-handling, URL-building, and vector
 * normalisation across crud-backend, search-backend, graph-backend,
 * query-backend, admin-backend, index-backend, streaming-backend,
 * and agent-memory-backend.
 */

import { NotFoundError, VelesDBError } from '../types';

// ---------------------------------------------------------------------------
// Unified transport interface
// ---------------------------------------------------------------------------

/** Base transport shared by all REST backend modules. */
export interface BaseTransport {
  requestJson<T>(
    method: string,
    path: string,
    body?: unknown
  ): Promise<TransportResponse<T>>;
}

/** Shape returned by every `requestJson` call. */
export interface TransportResponse<T> {
  data?: T;
  error?: TransportError;
}

export interface TransportError {
  code: string;
  message: string;
}

// ---------------------------------------------------------------------------
// Error handling helpers
// ---------------------------------------------------------------------------

/**
 * Throw a typed error when the transport response contains an error payload.
 *
 * - If `code === 'NOT_FOUND'` and a `resourceLabel` is provided, throws
 *   `NotFoundError` with the label.
 * - Otherwise throws `VelesDBError`.
 * - When no error is present, the function is a no-op.
 */
export function throwOnError(
  response: TransportResponse<unknown>,
  resourceLabel?: string
): void {
  if (!response.error) {
    return;
  }
  if (response.error.code === 'NOT_FOUND' && resourceLabel !== undefined) {
    throw new NotFoundError(resourceLabel);
  }
  throw new VelesDBError(response.error.message, response.error.code);
}

/**
 * Like `throwOnError`, but returns `null` on NOT_FOUND instead of throwing.
 * Useful for `getCollection`, `get`, `getCollectionStats`, etc.
 *
 * @returns `true` if there was a NOT_FOUND error (caller should return null),
 *          `undefined` otherwise (no error).
 * @throws {VelesDBError} for any non-NOT_FOUND error.
 */
export function returnNullOnNotFound(
  response: TransportResponse<unknown>
): true | undefined {
  if (!response.error) {
    return undefined;
  }
  if (response.error.code === 'NOT_FOUND') {
    return true;
  }
  throw new VelesDBError(response.error.message, response.error.code);
}

// ---------------------------------------------------------------------------
// URL helpers
// ---------------------------------------------------------------------------

/** Build the URL prefix for a named collection. */
export function collectionPath(collection: string): string {
  return `/collections/${encodeURIComponent(collection)}`;
}

// ---------------------------------------------------------------------------
// Vector helpers
// ---------------------------------------------------------------------------

/** Convert a `Float32Array | number[]` to a plain `number[]`. */
export function toNumberArray(v: number[] | Float32Array): number[] {
  return v instanceof Float32Array ? Array.from(v) : v;
}

// ---------------------------------------------------------------------------
// WASM backend helpers
// ---------------------------------------------------------------------------

/**
 * Throw a standard "not supported in WASM backend" error.
 * Consolidates the repeated pattern across 15+ WASM stubs.
 */
export function wasmNotSupported(feature: string): never {
  throw new VelesDBError(
    `${feature}: not supported in WASM backend. Use REST backend.`,
    'NOT_SUPPORTED'
  );
}
