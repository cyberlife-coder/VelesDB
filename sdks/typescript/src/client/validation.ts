/**
 * VelesDB Client - Shared validation helpers
 *
 * Extracted from client.ts to reduce file size. These stateless
 * helpers are used across multiple client method groups.
 * @packageDocumentation
 */

import type { VectorDocument, VelesDBConfig } from '../types';
import { ValidationError } from '../types';

/** Validate that a value is a non-empty string, throwing with the given label. */
export function requireNonEmptyString(value: unknown, label: string): void {
  if (!value || typeof value !== 'string') {
    throw new ValidationError(`${label} must be a non-empty string`);
  }
}

/** Validate that a value is a vector (number[] or Float32Array). */
export function requireVector(value: unknown, label: string): void {
  if (!value || (!Array.isArray(value) && !(value instanceof Float32Array))) {
    throw new ValidationError(`${label} must be an array or Float32Array`);
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
  // Mirror parseRestPointId (backend layer): a string id is valid only as a
  // plain run of decimal digits, coerced to a number for the range checks, so
  // the u64-safe string ids returned by agent memory pass the same gate.
  const numeric = typeof id === 'string' && /^\d+$/.test(id) ? Number(id) : id;
  if (
    typeof numeric !== 'number' ||
    !Number.isFinite(numeric) ||
    !Number.isInteger(numeric) ||
    numeric < 0 ||
    numeric > Number.MAX_SAFE_INTEGER
  ) {
    throw new ValidationError(
      `REST backend requires numeric u64-compatible document IDs in JS safe integer range (0..${Number.MAX_SAFE_INTEGER})`
    );
  }
}
