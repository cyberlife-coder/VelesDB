/**
 * VelesDB Client - Shared validation helpers
 *
 * Extracted from client.ts to reduce file size. These stateless
 * helpers are used across multiple client method groups.
 * @packageDocumentation
 */

import type { VectorDocument, VelesDBConfig } from '../types';
import { ValidationError } from '../types';
import { parseRestPointId } from '../backends/crud-backend';

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
  // Delegate to the canonical backend gate so the string→number coercion and
  // the "non-negative integer within JS safe-integer range" rule for REST point
  // ids live in exactly one place (it throws `ValidationError` on a bad id).
  parseRestPointId(id);
}
