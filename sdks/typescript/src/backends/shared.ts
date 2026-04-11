/**
 * Shared helpers for VelesDB REST backend modules.
 *
 * Eliminates duplicated error-handling, URL-building, and vector
 * normalisation across crud-backend, search-backend, graph-backend,
 * query-backend, admin-backend, index-backend, streaming-backend,
 * and agent-memory-backend.
 */

import { VelesDBError } from '../types';
import {
  parseVelesError,
  CollectionNotFoundError,
  PointNotFoundError,
  EdgeNotFoundError,
  NodeNotFoundError,
} from '../errors';

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
 * The error is instantiated via [`parseVelesError`], so:
 * - A server response carrying a `VELES-XXX` code surfaces as the
 *   matching typed sub-class (e.g. `CollectionNotFoundError` for
 *   `VELES-002`) — users can narrow via `instanceof`.
 * - A legacy response with a null/omitted code produces a generic
 *   `VelesError` with code `VELES-UNKNOWN`.
 *
 * The `resourceLabel` parameter is accepted for backward source
 * compatibility with v1.12 callers but is no longer consulted — the
 * server's verbatim message is always preferred over a synthesised
 * label.
 *
 * When no error is present, the function is a no-op.
 */
export function throwOnError(
  response: TransportResponse<unknown>,
  _resourceLabel?: string
): void {
  if (!response.error) {
    return;
  }
  throw parseVelesError(response.error.code, response.error.message);
}

/**
 * Like `throwOnError`, but returns a sentinel on "not found" instead of
 * throwing. Useful for `getCollection`, `get`, `getCollectionStats`,
 * etc. where `null` is the expected "absent" result.
 *
 * Recognises **two** server response formats (PR #586 Devin fix):
 *
 * - **Typed** — the server emitted a `VELES-XXX` code via
 *   `core_error_response`. Any of `VELES-002` (CollectionNotFound),
 *   `VELES-003` (PointNotFound), `VELES-020` (EdgeNotFound), or
 *   `VELES-022` (NodeNotFound) signals "absent".
 * - **Legacy / status-derived** — the server emitted no `code` field
 *   (via `error_response`), so the transport layer filled in
 *   `'NOT_FOUND'` from the HTTP 404 status. This branch keeps older
 *   handlers that have not yet been migrated working correctly.
 *
 * @returns `true` if the error indicates the resource is missing,
 *          signalling the caller should return `null`; `undefined`
 *          when no error is present.
 * @throws {VelesError} for any non-"not found" error, typed by VELES
 *         code when available.
 */
export function returnNullOnNotFound(
  response: TransportResponse<unknown>
): true | undefined {
  if (!response.error) {
    return undefined;
  }
  if (isNotFoundError(response.error.code)) {
    return true;
  }
  throw parseVelesError(response.error.code, response.error.message);
}

/**
 * Shared "is this a not-found error code?" predicate used by
 * `returnNullOnNotFound` and by individual endpoint wrappers that
 * need to convert a 404 into a boolean/null sentinel (e.g.
 * `removeEdge` → `false`).
 *
 * Accepts both the legacy status-derived `'NOT_FOUND'` string and
 * every typed `VELES-XXX` code that means "resource missing".
 */
export function isNotFoundError(code: string | undefined): boolean {
  if (code === undefined) {
    return false;
  }
  if (code === 'NOT_FOUND') {
    return true;
  }
  const err = parseVelesError(code, '');
  return (
    err instanceof CollectionNotFoundError ||
    err instanceof PointNotFoundError ||
    err instanceof EdgeNotFoundError ||
    err instanceof NodeNotFoundError
  );
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
