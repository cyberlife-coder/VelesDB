/**
 * Shared helpers for VelesDB REST backend modules.
 *
 * Eliminates duplicated error-handling, URL-building, and vector
 * normalisation across crud-backend, search-backend, graph-backend,
 * query-backend, admin-backend, index-backend, streaming-backend,
 * and agent-memory-backend.
 */

import { NotFoundError, VelesDBError } from '../types';
import {
  parseVelesError,
  CollectionNotFoundError,
  PointNotFoundError,
  EdgeNotFoundError,
  NodeNotFoundError,
  InvalidCollectionNameError,
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
 * Routing priority (PR #586 Devin finding #7 — preserve v1.12 catch
 * ergonomics):
 *
 * 1. **Legacy `NotFoundError` compat** — when the server transmits
 *    no VELES code (so the transport layer fills in
 *    `'NOT_FOUND'` via `mapStatusToErrorCode(404)`) AND a
 *    `resourceLabel` was passed by the caller, throw the v1.12
 *    `NotFoundError(resourceLabel)`. This keeps pre-v1.13 handlers
 *    catching `(e instanceof NotFoundError)` on REST 404 responses
 *    working unchanged.
 * 2. **Typed VELES error** — otherwise delegate to
 *    `parseVelesError(code, message)`, which instantiates the
 *    matching `VelesError` sub-class (e.g. `CollectionNotFoundError`
 *    for `VELES-002`) when the server emitted a typed code.
 *
 * When no error is present, the function is a no-op.
 */
export function throwOnError(
  response: TransportResponse<unknown>,
  resourceLabel?: string
): void {
  if (!response.error) {
    return;
  }
  // Legacy path: status-derived 'NOT_FOUND' with a caller-supplied
  // resource label still throws the pre-v1.13 `NotFoundError` for
  // backward source compatibility with handlers that narrow on it.
  if (response.error.code === 'NOT_FOUND' && resourceLabel !== undefined) {
    throw new NotFoundError(resourceLabel);
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
// Collection name validation
// ---------------------------------------------------------------------------

/** Maximum allowed length for a collection name (matches core's
 * `MAX_COLLECTION_NAME_LENGTH`). */
export const MAX_COLLECTION_NAME_LENGTH = 128;

/**
 * Windows reserved device names that are rejected by the core validator.
 * Case-insensitive comparison.
 */
const WINDOWS_RESERVED_NAMES = new Set([
  'CON',
  'PRN',
  'AUX',
  'NUL',
  'COM1',
  'COM2',
  'COM3',
  'COM4',
  'COM5',
  'COM6',
  'COM7',
  'COM8',
  'COM9',
  'LPT1',
  'LPT2',
  'LPT3',
  'LPT4',
  'LPT5',
  'LPT6',
  'LPT7',
  'LPT8',
  'LPT9',
]);

/**
 * Validate a collection name before interpolating it into a VelesQL query.
 *
 * Mirrors `velesdb_core::validation::validate_collection_name` (Rust) so
 * that client-side rejection matches server-side acceptance one-to-one:
 *
 * - Must be a non-empty string.
 * - Must not exceed {@link MAX_COLLECTION_NAME_LENGTH} characters.
 * - Must not be `.` or `..` (path traversal).
 * - Must not start with `-` (avoids CLI flag confusion).
 * - Must contain only ASCII alphanumerics, `_`, or `-`.
 * - Must not be a Windows reserved device name (`CON`, `PRN`, `AUX`, `NUL`,
 *   `COM1`–`COM9`, `LPT1`–`LPT9`), case-insensitive.
 *
 * Used as a defence-in-depth check against VelesQL injection for callers
 * that build queries containing collection names via string interpolation
 * (e.g. `TRAIN QUANTIZER ON ${name}`), since the VelesQL grammar does not
 * support a parameterised collection identifier at that position.
 *
 * @throws {InvalidCollectionNameError} if the name fails any of the rules
 *   above. The error code is `VELES-034`, matching the server-side
 *   response.
 */
export function validateCollectionName(name: string): void {
  if (typeof name !== 'string' || name.length === 0) {
    throw new InvalidCollectionNameError(
      'Collection name must be a non-empty string'
    );
  }

  if (name.length > MAX_COLLECTION_NAME_LENGTH) {
    throw new InvalidCollectionNameError(
      `Collection name '${name}' exceeds maximum length of ${MAX_COLLECTION_NAME_LENGTH} characters`
    );
  }

  if (name === '.' || name === '..') {
    throw new InvalidCollectionNameError(
      `Collection name '${name}' is not allowed (path traversal)`
    );
  }

  if (name.startsWith('-')) {
    throw new InvalidCollectionNameError(
      `Collection name '${name}' must not start with a hyphen`
    );
  }

  if (!/^[A-Za-z0-9_-]+$/.test(name)) {
    throw new InvalidCollectionNameError(
      `Collection name '${name}' contains forbidden characters; only ASCII letters, digits, underscores, and hyphens are allowed`
    );
  }

  if (WINDOWS_RESERVED_NAMES.has(name.toUpperCase())) {
    throw new InvalidCollectionNameError(
      `Collection name '${name}' is a Windows reserved device name`
    );
  }
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
