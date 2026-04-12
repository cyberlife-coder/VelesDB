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
  if (doc.id === undefined || doc.id === null) {
    throw new ValidationError('Document ID is required');
  }

  requireVector(doc.vector, 'Vector');

  validateRestPointId(doc.id, config);
}

/** Validate that a document ID is a valid REST point ID when using REST backend. */
export function validateRestPointId(id: string | number, config: VelesDBConfig): void {
  if (
    config.backend === 'rest' &&
    (
      typeof id !== 'number' ||
      !Number.isInteger(id) ||
      id < 0 ||
      id > Number.MAX_SAFE_INTEGER
    )
  ) {
    throw new ValidationError(
      `REST backend requires numeric u64-compatible document IDs in JS safe integer range (0..${Number.MAX_SAFE_INTEGER})`
    );
  }
}
