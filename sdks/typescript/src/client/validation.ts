/**
 * VelesDB Client - Shared validation helpers
 *
 * Extracted from client.ts to reduce file size. These stateless
 * helpers are used across multiple client method groups.
 * @packageDocumentation
 */

import type { BatchSearchItem, VectorDocument, VelesDBConfig } from '../types';
import { ValidationError, VelesDBError } from '../types';
import { InvalidDimensionError, InvalidVectorError } from '../errors';
import { parseRestPointId } from '../backends/crud-backend';

/** Validate that a value is a non-empty string, throwing with the given label. */
export function requireNonEmptyString(value: unknown, label: string): void {
  if (!value || typeof value !== 'string') {
    throw new ValidationError(`${label} must be a non-empty string`);
  }
}

/**
 * Core's `MAX_DIMENSION` (`validation.rs`): the largest vector dimension a
 * collection may declare. Mirrors core so the client refuses the same values
 * instead of letting a backend accept what core would reject.
 */
export const MAX_DIMENSION = 65_536;

/**
 * Core's `validate_dimension`: an integer in `1..=MAX_DIMENSION`, refused with
 * VELES-032. Applied client-side so the WASM path (whose binding does not call
 * `validate_dimension` at creation) refuses the same values core does.
 */
export function requireDimensionInRange(dimension: unknown): void {
  if (
    typeof dimension !== 'number' ||
    !Number.isInteger(dimension) ||
    dimension < 1 ||
    dimension > MAX_DIMENSION
  ) {
    throw new InvalidDimensionError(
      `dimension must be an integer in 1..=${MAX_DIMENSION} (core validate_dimension, VELES-032); got: ${String(dimension)}`
    );
  }
}

/** Validate that a value is a finite vector (number[] or Float32Array). */
export function requireVector(value: unknown, label: string): void {
  if (!value || (!Array.isArray(value) && !(value instanceof Float32Array))) {
    throw new ValidationError(`${label} must be an array or Float32Array`);
  }
  // Same cold-boundary rule as core's `validate_vector_is_finite` (VELES-005):
  // refuse NaN/±Inf here rather than let a backend accept what core rejects.
  for (let i = 0; i < value.length; i++) {
    const component = value[i] as number;
    if (!Number.isFinite(component)) {
      throw new InvalidVectorError(
        `${label} component at index ${i} is not finite: ${component}`
      );
    }
  }
}

/** Validate a docs array and each document within it. */
export function validateDocsBatch(
  docs: VectorDocument[],
  validateDoc: (doc: VectorDocument) => void
): void {
  if (!Array.isArray(docs)) {
    throw new ValidationError('Documents must be an array');
  }
  for (const doc of docs) {
    validateDoc(doc);
  }
}

/** Validate a single vector document (id + vector presence). */
export function validateDocument(doc: VectorDocument, config: VelesDBConfig): void {
  // Runtime guard against untyped JS callers: the compile-time type marks
  // `id` as required, so check it as an untyped value.
  const id: unknown = doc.id;
  if (id === undefined || id === null) {
    throw new ValidationError('Document ID is required');
  }

  requireVector(doc.vector, 'Vector');

  validateRestPointId(doc.id, config);
}

/** Validate that a document ID is a valid REST point ID when using REST backend. */
export function validateRestPointId(id: string | number, config: VelesDBConfig): void {
  if (config.backend !== 'rest') {
    return;
  }
  // Delegate to the canonical backend gate so the id rules — non-negative
  // integers in the JS safe-integer range, plus verbatim decimal strings up to
  // u64::MAX — live in exactly one place (it throws `ValidationError` on a
  // bad id).
  parseRestPointId(id);
}

// ============================================================================
// Capability guards — refuse options a backend cannot honor
// ============================================================================

/**
 * Refuse REST-only search options on a non-REST backend instead of letting
 * the backend silently drop them (these fields have no WASM equivalent).
 */
export function refuseRestOnlyOptions(
  backend: string,
  options: object | undefined,
  fields: readonly string[]
): void {
  if (backend === 'rest' || !options) {
    return;
  }
  const bag = options as Record<string, unknown>;
  for (const field of fields) {
    if (bag[field] !== undefined) {
      throw new VelesDBError(
        `${field} is only supported by the REST backend (current backend: '${backend}')`,
        'NOT_SUPPORTED'
      );
    }
  }
}

/**
 * Refuse per-entry batch fields the batch kernel never applies, on every
 * backend: `run_batch_search` (server) uses only each entry's `vector`,
 * `top_k` and `filter` — `mode`/`ef_search`/`timeout_ms`/sparse/`fusion`
 * are validated but ignored, and the WASM dispatcher drops sparse too.
 * Accepting them would return dense-only rankings while implying otherwise.
 */
export function refuseUnsupportedBatchFields(searches: BatchSearchItem[]): void {
  const ignored = [
    'quality',
    'efSearch',
    'timeoutMs',
    'fusion',
    'sparseVector',
    'sparseVectors',
    'sparseIndexName',
  ] as const;
  for (const s of searches) {
    const bag = s as unknown as Record<string, unknown>;
    for (const field of ignored) {
      if (bag[field] !== undefined) {
        throw new VelesDBError(
          `${field} is not applied to batch entries (only vector, k and filter are); use search() for it`,
          'NOT_SUPPORTED'
        );
      }
    }
  }
}

/**
 * Refuse named sparse vectors (`sparseVectors`) on a backend without named
 * sparse indexes — the WASM build has a single default sparse index, so a
 * named map cannot be represented there and would be silently dropped.
 */
export function refuseNamedSparseOnDocs(
  docs: VectorDocument[],
  namedSparseIndexes: boolean,
  backend: string
): void {
  if (namedSparseIndexes) {
    return;
  }
  for (const doc of docs) {
    if (doc.sparseVectors !== undefined) {
      throw new VelesDBError(
        `sparseVectors requires named sparse indexes (current backend: '${backend}')`,
        'NOT_SUPPORTED'
      );
    }
  }
}

/**
 * Refuse sparse vectors on the single-point streaming insert: the server's
 * `StreamInsertRequest` is `{id, vector, payload}` only, so a sparse field
 * would be silently dropped by serde. Use `upsert`/`upsertBatch` (or the
 * NDJSON `streamUpsertPoints`, whose `Point` accepts sparse fields).
 */
export function refuseSparseOnStreamInsert(docs: VectorDocument[]): void {
  for (const doc of docs) {
    if (doc.sparseVector !== undefined || doc.sparseVectors !== undefined) {
      throw new VelesDBError(
        'sparse vectors are not accepted by the streaming insert endpoint; use upsert()/upsertBatch() or streamUpsertPoints()',
        'NOT_SUPPORTED'
      );
    }
  }
}
