/**
 * Typed Error Hierarchy Tests (Sprint 2 Wave 4 — #20 PROP-ERR-TSSDK)
 *
 * Verifies that every VELES-XXX error code from `velesdb_core::error::Error`
 * has a corresponding typed TypeScript class, preserves the verbatim code,
 * and can be discriminated via `instanceof` AND via `parseVelesError`.
 */

import { describe, it, expect } from 'vitest';
import {
  VelesError,
  CollectionExistsError,
  CollectionNotFoundError,
  PointNotFoundError,
  DimensionMismatchError,
  InvalidVectorError,
  StorageError,
  IndexError,
  IndexCorruptedError,
  ConfigError,
  QueryError,
  IoError,
  SerializationError,
  InternalError,
  VectorNotAllowedError,
  SearchNotSupportedError,
  VectorRequiredError,
  SchemaValidationError,
  GraphNotSupportedError,
  EdgeExistsError,
  EdgeNotFoundError,
  InvalidEdgeLabelError,
  NodeNotFoundError,
  OverflowError,
  ColumnStoreError,
  GpuError,
  EpochMismatchError,
  GuardRailError,
  InvalidQuantizerConfigError,
  TrainingFailedError,
  SparseIndexError,
  DatabaseLockedError,
  InvalidDimensionError,
  AllocationFailedError,
  InvalidCollectionNameError,
  SnapshotBuildFailedError,
  IncompatibleSchemaVersionError,
  parseVelesError,
  VELES_ERROR_CODES,
} from '../src/errors';
import { VelesDBError } from '../src/types';

// ============================================================================
// Structural contract: 36 codes, 36 classes, all extend VelesError
// ============================================================================

describe('VELES error codes — structural contract', () => {
  it('exports exactly 36 codes (VELES-001 to VELES-036)', () => {
    expect(VELES_ERROR_CODES).toHaveLength(36);
    expect(VELES_ERROR_CODES[0]).toBe('VELES-001');
    expect(VELES_ERROR_CODES[35]).toBe('VELES-036');
  });

  it('every VELES error class extends VelesError (and VelesDBError)', () => {
    const sampleMessage = 'test';
    const sampleErrors = [
      new CollectionExistsError(sampleMessage),
      new CollectionNotFoundError(sampleMessage),
      new PointNotFoundError(sampleMessage),
      new DimensionMismatchError(sampleMessage),
      new StorageError(sampleMessage),
      new QueryError(sampleMessage),
      new GraphNotSupportedError(sampleMessage),
      new GuardRailError(sampleMessage),
      new IncompatibleSchemaVersionError(sampleMessage),
    ];
    for (const err of sampleErrors) {
      expect(err).toBeInstanceOf(VelesError);
      expect(err).toBeInstanceOf(VelesDBError);
      expect(err).toBeInstanceOf(Error);
    }
  });
});

// ============================================================================
// Per-class code preservation (the whole point of this commit)
// ============================================================================

describe('VELES typed classes — verbatim code preservation', () => {
  const cases: Array<[new (msg: string) => VelesError, string, string]> = [
    [CollectionExistsError, 'VELES-001', 'CollectionExistsError'],
    [CollectionNotFoundError, 'VELES-002', 'CollectionNotFoundError'],
    [PointNotFoundError, 'VELES-003', 'PointNotFoundError'],
    [DimensionMismatchError, 'VELES-004', 'DimensionMismatchError'],
    [InvalidVectorError, 'VELES-005', 'InvalidVectorError'],
    [StorageError, 'VELES-006', 'StorageError'],
    [IndexError, 'VELES-007', 'IndexError'],
    [IndexCorruptedError, 'VELES-008', 'IndexCorruptedError'],
    [ConfigError, 'VELES-009', 'ConfigError'],
    [QueryError, 'VELES-010', 'QueryError'],
    [IoError, 'VELES-011', 'IoError'],
    [SerializationError, 'VELES-012', 'SerializationError'],
    [InternalError, 'VELES-013', 'InternalError'],
    [VectorNotAllowedError, 'VELES-014', 'VectorNotAllowedError'],
    [SearchNotSupportedError, 'VELES-015', 'SearchNotSupportedError'],
    [VectorRequiredError, 'VELES-016', 'VectorRequiredError'],
    [SchemaValidationError, 'VELES-017', 'SchemaValidationError'],
    [GraphNotSupportedError, 'VELES-018', 'GraphNotSupportedError'],
    [EdgeExistsError, 'VELES-019', 'EdgeExistsError'],
    [EdgeNotFoundError, 'VELES-020', 'EdgeNotFoundError'],
    [InvalidEdgeLabelError, 'VELES-021', 'InvalidEdgeLabelError'],
    [NodeNotFoundError, 'VELES-022', 'NodeNotFoundError'],
    [OverflowError, 'VELES-023', 'OverflowError'],
    [ColumnStoreError, 'VELES-024', 'ColumnStoreError'],
    [GpuError, 'VELES-025', 'GpuError'],
    [EpochMismatchError, 'VELES-026', 'EpochMismatchError'],
    [GuardRailError, 'VELES-027', 'GuardRailError'],
    [InvalidQuantizerConfigError, 'VELES-028', 'InvalidQuantizerConfigError'],
    [TrainingFailedError, 'VELES-029', 'TrainingFailedError'],
    [SparseIndexError, 'VELES-030', 'SparseIndexError'],
    [DatabaseLockedError, 'VELES-031', 'DatabaseLockedError'],
    [InvalidDimensionError, 'VELES-032', 'InvalidDimensionError'],
    [AllocationFailedError, 'VELES-033', 'AllocationFailedError'],
    [InvalidCollectionNameError, 'VELES-034', 'InvalidCollectionNameError'],
    [SnapshotBuildFailedError, 'VELES-035', 'SnapshotBuildFailedError'],
    [IncompatibleSchemaVersionError, 'VELES-036', 'IncompatibleSchemaVersionError'],
  ];

  it.each(cases)('%s preserves code %s and name %s', (Cls, expectedCode, expectedName) => {
    const err = new Cls('boom');
    expect(err.code).toBe(expectedCode);
    expect(err.name).toBe(expectedName);
    expect(err.message).toBe('boom');
  });
});

// ============================================================================
// parseVelesError — the factory used by throwOnError / transport layer
// ============================================================================

describe('parseVelesError — code → typed class routing', () => {
  it('routes VELES-002 to CollectionNotFoundError', () => {
    const err = parseVelesError('VELES-002', "Collection 'docs' not found");
    expect(err).toBeInstanceOf(CollectionNotFoundError);
    expect(err.code).toBe('VELES-002');
    expect(err.message).toBe("Collection 'docs' not found");
  });

  it('routes VELES-004 to DimensionMismatchError', () => {
    const err = parseVelesError('VELES-004', 'expected 768, got 512');
    expect(err).toBeInstanceOf(DimensionMismatchError);
  });

  it('routes VELES-027 to GuardRailError', () => {
    const err = parseVelesError('VELES-027', 'rate limit exceeded');
    expect(err).toBeInstanceOf(GuardRailError);
  });

  it('routes VELES-036 to IncompatibleSchemaVersionError', () => {
    const err = parseVelesError('VELES-036', 'schema v5 > supported v3');
    expect(err).toBeInstanceOf(IncompatibleSchemaVersionError);
  });

  it('returns a generic VelesError for unknown codes (forward-compat)', () => {
    const err = parseVelesError('VELES-999', 'future error');
    expect(err).toBeInstanceOf(VelesError);
    expect(err.code).toBe('VELES-999');
    // Must not match any specific sub-class
    expect(err).not.toBeInstanceOf(CollectionNotFoundError);
  });

  it('returns a generic VelesError when code is null or undefined', () => {
    const errNull = parseVelesError(null, 'legacy error');
    expect(errNull).toBeInstanceOf(VelesError);
    expect(errNull.code).toBe('VELES-UNKNOWN');

    const errUndef = parseVelesError(undefined, 'legacy error');
    expect(errUndef).toBeInstanceOf(VelesError);
    expect(errUndef.code).toBe('VELES-UNKNOWN');
  });

  it('preserves original server message verbatim', () => {
    const serverMsg = "[VELES-002] Collection 'foo-bar' not found";
    const err = parseVelesError('VELES-002', serverMsg);
    expect(err.message).toBe(serverMsg);
  });
});

// ============================================================================
// Backward compatibility: the 4 legacy client-side error classes still exist
// ============================================================================

describe('Legacy client-side error classes (backward compat)', () => {
  it('ConnectionError, ValidationError, NotFoundError, BackpressureError still exist', async () => {
    const { ConnectionError, ValidationError, NotFoundError, BackpressureError } =
      await import('../src/types');
    expect(new ConnectionError('conn')).toBeInstanceOf(VelesDBError);
    expect(new ValidationError('bad')).toBeInstanceOf(VelesDBError);
    expect(new NotFoundError('x')).toBeInstanceOf(VelesDBError);
    expect(new BackpressureError()).toBeInstanceOf(VelesDBError);
  });

  it('Legacy errors are NOT instances of VelesError (they have no VELES code)', async () => {
    const { ConnectionError, ValidationError } = await import('../src/types');
    expect(new ConnectionError('x')).not.toBeInstanceOf(VelesError);
    expect(new ValidationError('y')).not.toBeInstanceOf(VelesError);
  });
});

// ============================================================================
// Catching contract: user can catch by code or by class
// ============================================================================

describe('Catching contract — users can narrow errors ergonomically', () => {
  it('catch by specific class', () => {
    let caught: unknown;
    try {
      throw parseVelesError('VELES-002', 'not found');
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(CollectionNotFoundError);
  });

  it('catch by VelesError base (catches all VELES-XXX)', () => {
    const errors = [
      parseVelesError('VELES-001', 'exists'),
      parseVelesError('VELES-010', 'parse'),
      parseVelesError('VELES-020', 'edge not found'),
    ];
    for (const e of errors) {
      expect(e).toBeInstanceOf(VelesError);
    }
  });

  it('error.code is the single source of truth — never overwrites with "NOT_FOUND"', () => {
    // Regression guard: the pre-v1.13 SDK mapped server codes to generic
    // 'NOT_FOUND'/'VALIDATION_ERROR' strings. This test ensures we never
    // regress into that anti-pattern.
    const err = parseVelesError('VELES-002', "Collection 'x' not found");
    expect(err.code).toBe('VELES-002');
    expect(err.code).not.toBe('NOT_FOUND');
  });
});
