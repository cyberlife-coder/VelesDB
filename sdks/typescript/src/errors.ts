/**
 * VelesDB Typed Error Hierarchy
 *
 * One TypeScript class per `velesdb_core::error::Error` variant, preserving
 * the verbatim `VELES-XXX` code for ergonomic catch-by-instance narrowing.
 *
 * Motivation: the pre-v1.13 SDK mapped all server errors to a handful of
 * generic classes (`NotFoundError`, `VelesDBError`) and clobbered the real
 * `VELES-XXX` code with strings like `'NOT_FOUND'`. Client code had no way
 * to distinguish "collection not found" (VELES-002) from "edge not found"
 * (VELES-020) without string-sniffing the message. This module fixes that.
 *
 * @example Catch by specific class
 * ```typescript
 * try {
 *   await db.search('docs', vec, { k: 10 });
 * } catch (e) {
 *   if (e instanceof CollectionNotFoundError) { ... }
 *   else if (e instanceof DimensionMismatchError) { ... }
 *   else if (e instanceof VelesError) { ... } // catches any VELES-XXX
 *   else throw e;                              // not ours, rethrow
 * }
 * ```
 *
 * @packageDocumentation
 */

import { VelesDBError } from './types';

// ============================================================================
// Base class — every typed error below extends VelesError (which extends
// VelesDBError for backward-compat with `catch (e instanceof VelesDBError)`).
// ============================================================================

/**
 * Base class for every server-originated VelesDB error carrying a
 * `VELES-XXX` code. All 36 typed sub-classes extend this.
 *
 * Also a direct sub-class of `VelesDBError` so that legacy handlers
 * that catch `VelesDBError` continue to receive typed errors too.
 */
export class VelesError extends VelesDBError {
  constructor(message: string, code: string, cause?: Error) {
    super(message, code, cause);
    this.name = 'VelesError';
  }
}

// ============================================================================
// The 36 typed sub-classes — one per VELES-XXX variant in velesdb-core
// ============================================================================

/** Collection already exists (VELES-001). */
export class CollectionExistsError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-001');
    this.name = 'CollectionExistsError';
  }
}

/** Collection not found (VELES-002). */
export class CollectionNotFoundError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-002');
    this.name = 'CollectionNotFoundError';
  }
}

/** Point with the given ID not found (VELES-003). */
export class PointNotFoundError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-003');
    this.name = 'PointNotFoundError';
  }
}

/** Vector dimension mismatch (VELES-004). */
export class DimensionMismatchError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-004');
    this.name = 'DimensionMismatchError';
  }
}

/** Invalid vector (NaN, wrong length, etc.) (VELES-005). */
export class InvalidVectorError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-005');
    this.name = 'InvalidVectorError';
  }
}

/** Storage layer error (mmap, WAL, I/O) (VELES-006). */
export class StorageError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-006');
    this.name = 'StorageError';
  }
}

/** HNSW / BM25 / secondary index error (VELES-007). */
export class IndexError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-007');
    this.name = 'IndexError';
  }
}

/** Index files corrupted and need rebuild (VELES-008). */
export class IndexCorruptedError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-008');
    this.name = 'IndexCorruptedError';
  }
}

/** Configuration error (invalid settings) (VELES-009). */
export class ConfigError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-009');
    this.name = 'ConfigError';
  }
}

/** VelesQL parse or execution error (VELES-010). */
export class QueryError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-010');
    this.name = 'QueryError';
  }
}

/** Low-level I/O error (wraps `std::io::Error`) (VELES-011). */
export class IoError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-011');
    this.name = 'IoError';
  }
}

/** Serialization / deserialization error (VELES-012). */
export class SerializationError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-012');
    this.name = 'SerializationError';
  }
}

/** Internal error — please report if encountered (VELES-013). */
export class InternalError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-013');
    this.name = 'InternalError';
  }
}

/** Vector not allowed on metadata-only collection (VELES-014). */
export class VectorNotAllowedError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-014');
    this.name = 'VectorNotAllowedError';
  }
}

/** Vector search not supported on metadata-only collection (VELES-015). */
export class SearchNotSupportedError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-015');
    this.name = 'SearchNotSupportedError';
  }
}

/** Vector required for vector collection (VELES-016). */
export class VectorRequiredError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-016');
    this.name = 'VectorRequiredError';
  }
}

/** Schema validation error (VELES-017). */
export class SchemaValidationError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-017');
    this.name = 'SchemaValidationError';
  }
}

/** Graph operation not supported on this collection type (VELES-018). */
export class GraphNotSupportedError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-018');
    this.name = 'GraphNotSupportedError';
  }
}

/** Edge with the given ID already exists (VELES-019). */
export class EdgeExistsError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-019');
    this.name = 'EdgeExistsError';
  }
}

/** Edge with the given ID not found (VELES-020). */
export class EdgeNotFoundError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-020');
    this.name = 'EdgeNotFoundError';
  }
}

/** Invalid edge label (empty, too long, forbidden chars) (VELES-021). */
export class InvalidEdgeLabelError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-021');
    this.name = 'InvalidEdgeLabelError';
  }
}

/** Node with the given ID not found (VELES-022). */
export class NodeNotFoundError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-022');
    this.name = 'NodeNotFoundError';
  }
}

/** Numeric overflow / cast truncation (VELES-023). */
export class OverflowError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-023');
    this.name = 'OverflowError';
  }
}

/** Column store schema or primary-key validation failed (VELES-024). */
export class ColumnStoreError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-024');
    this.name = 'ColumnStoreError';
  }
}

/** GPU parameter validation or operation failure (VELES-025). */
export class GpuError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-025');
    this.name = 'GpuError';
  }
}

/** Epoch mismatch — stale mmap guard, not recoverable (VELES-026). */
export class EpochMismatchError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-026');
    this.name = 'EpochMismatchError';
  }
}

/** Guard-rail violation: timeout, depth, cardinality, memory, rate limit (VELES-027). */
export class GuardRailError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-027');
    this.name = 'GuardRailError';
  }
}

/** Invalid quantizer config (PQ subspaces, empty training set, etc.) (VELES-028). */
export class InvalidQuantizerConfigError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-028');
    this.name = 'InvalidQuantizerConfigError';
  }
}

/** Quantizer training failed (convergence, insufficient data) (VELES-029). */
export class TrainingFailedError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-029');
    this.name = 'TrainingFailedError';
  }
}

/** Sparse index error (VELES-030). */
export class SparseIndexError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-030');
    this.name = 'SparseIndexError';
  }
}

/** Database already locked by another process (VELES-031). */
export class DatabaseLockedError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-031');
    this.name = 'DatabaseLockedError';
  }
}

/** Vector dimension outside the valid range (VELES-032). */
export class InvalidDimensionError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-032');
    this.name = 'InvalidDimensionError';
  }
}

/** Memory allocation failure (out of memory / invalid layout) (VELES-033). */
export class AllocationFailedError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-033');
    this.name = 'AllocationFailedError';
  }
}

/** Collection name contains forbidden characters or path separators (VELES-034). */
export class InvalidCollectionNameError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-034');
    this.name = 'InvalidCollectionNameError';
  }
}

/** CSR snapshot build failed (allocation failure during rebuild) (VELES-035). */
export class SnapshotBuildFailedError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-035');
    this.name = 'SnapshotBuildFailedError';
  }
}

/** Collection was created with a newer schema version than this binary supports (VELES-036). */
export class IncompatibleSchemaVersionError extends VelesError {
  constructor(message: string) {
    super(message, 'VELES-036');
    this.name = 'IncompatibleSchemaVersionError';
  }
}

// ============================================================================
// Code registry + factory discriminator
// ============================================================================

/**
 * Every VELES code known to this SDK version, in ascending order.
 *
 * Used by tests to verify the 36-code contract and by tooling to emit
 * doc/type metadata.
 */
export const VELES_ERROR_CODES = [
  'VELES-001', 'VELES-002', 'VELES-003', 'VELES-004', 'VELES-005',
  'VELES-006', 'VELES-007', 'VELES-008', 'VELES-009', 'VELES-010',
  'VELES-011', 'VELES-012', 'VELES-013', 'VELES-014', 'VELES-015',
  'VELES-016', 'VELES-017', 'VELES-018', 'VELES-019', 'VELES-020',
  'VELES-021', 'VELES-022', 'VELES-023', 'VELES-024', 'VELES-025',
  'VELES-026', 'VELES-027', 'VELES-028', 'VELES-029', 'VELES-030',
  'VELES-031', 'VELES-032', 'VELES-033', 'VELES-034', 'VELES-035',
  'VELES-036',
] as const;

/** Union type of every known VELES code. */
export type VelesErrorCode = (typeof VELES_ERROR_CODES)[number];

/**
 * Internal mapping from code → constructor. Kept in one place so that
 * `parseVelesError` dispatch is a single object lookup instead of a
 * 36-arm switch (lower cyclomatic complexity).
 */
const CODE_TO_CLASS: Record<string, new (message: string) => VelesError> = {
  'VELES-001': CollectionExistsError,
  'VELES-002': CollectionNotFoundError,
  'VELES-003': PointNotFoundError,
  'VELES-004': DimensionMismatchError,
  'VELES-005': InvalidVectorError,
  'VELES-006': StorageError,
  'VELES-007': IndexError,
  'VELES-008': IndexCorruptedError,
  'VELES-009': ConfigError,
  'VELES-010': QueryError,
  'VELES-011': IoError,
  'VELES-012': SerializationError,
  'VELES-013': InternalError,
  'VELES-014': VectorNotAllowedError,
  'VELES-015': SearchNotSupportedError,
  'VELES-016': VectorRequiredError,
  'VELES-017': SchemaValidationError,
  'VELES-018': GraphNotSupportedError,
  'VELES-019': EdgeExistsError,
  'VELES-020': EdgeNotFoundError,
  'VELES-021': InvalidEdgeLabelError,
  'VELES-022': NodeNotFoundError,
  'VELES-023': OverflowError,
  'VELES-024': ColumnStoreError,
  'VELES-025': GpuError,
  'VELES-026': EpochMismatchError,
  'VELES-027': GuardRailError,
  'VELES-028': InvalidQuantizerConfigError,
  'VELES-029': TrainingFailedError,
  'VELES-030': SparseIndexError,
  'VELES-031': DatabaseLockedError,
  'VELES-032': InvalidDimensionError,
  'VELES-033': AllocationFailedError,
  'VELES-034': InvalidCollectionNameError,
  'VELES-035': SnapshotBuildFailedError,
  'VELES-036': IncompatibleSchemaVersionError,
};

/**
 * Instantiate the correct typed error class from a server-provided
 * VELES code and message.
 *
 * - If `code` matches one of the 36 known VELES-XXX codes, returns
 *   the matching typed sub-class.
 * - If `code` is an unknown VELES code (e.g. `VELES-999` from a
 *   newer server), returns a generic `VelesError` preserving the
 *   code verbatim — forward-compatible with future core versions.
 * - If `code` is null/undefined (legacy `error_response` path in
 *   server that omits the code field), returns a generic
 *   `VelesError` with code `'VELES-UNKNOWN'`.
 *
 * **Never** fabricates a fake code like `'NOT_FOUND'` — that was the
 * pre-v1.13 anti-pattern this function exists to replace.
 */
export function parseVelesError(
  code: string | null | undefined,
  message: string
): VelesError {
  if (code === null || code === undefined) {
    return new VelesError(message, 'VELES-UNKNOWN');
  }
  const Cls = CODE_TO_CLASS[code];
  if (Cls !== undefined) {
    return new Cls(message);
  }
  // Unknown but syntactically valid VELES code — preserve verbatim.
  return new VelesError(message, code);
}
