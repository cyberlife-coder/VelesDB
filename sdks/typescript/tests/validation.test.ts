/**
 * Tests for client-layer REST point-id validation.
 *
 * #1047: `validateRestPointId` guards `db.delete` / `db.get` / upsert on the
 * REST backend. It must accept the u64-safe decimal-string ids returned by the
 * agent-memory helpers (consistent with the backend's `parseRestPointId`), and
 * reject malformed strings rather than silently coercing them.
 */

import { describe, it, expect } from 'vitest';
import { requireDimensionInRange, requireVector, validateRestPointId } from '../src/client/validation';
import { InvalidDimensionError, InvalidVectorError } from '../src/errors';
import { ValidationError } from '../src/types';
import type { VelesDBConfig } from '../src/types';

const rest = { backend: 'rest', url: 'http://localhost:8080' } as VelesDBConfig;
const wasm = { backend: 'wasm' } as VelesDBConfig;

describe('validateRestPointId', () => {
  it('accepts numeric ids in range', () => {
    expect(() => validateRestPointId(0, rest)).not.toThrow();
    expect(() => validateRestPointId(12345, rest)).not.toThrow();
  });

  it('accepts decimal-string ids (as returned by recordEvent/learnProcedure)', () => {
    expect(() => validateRestPointId('0', rest)).not.toThrow();
    expect(() => validateRestPointId('12345', rest)).not.toThrow();
  });

  it('accepts decimal-string ids above 2^53 up to u64::MAX (store/delete symmetry)', () => {
    // recordEvent/learnProcedure accept and return these ids; delete/get must
    // accept the very same strings or the memory becomes write-only.
    expect(() => validateRestPointId('9007199254740993', rest)).not.toThrow();
    expect(() => validateRestPointId('18446744073709551615', rest)).not.toThrow();
  });

  it('rejects decimal-string ids beyond u64::MAX', () => {
    expect(() => validateRestPointId('18446744073709551616', rest)).toThrow(ValidationError);
  });

  it('rejects malformed string ids instead of coercing them', () => {
    for (const bad of ['', '   ', '1e3', '0x10', '-5', '12.5', '12abc']) {
      expect(() => validateRestPointId(bad, rest)).toThrow(ValidationError);
    }
  });

  it('rejects out-of-range and non-integer numbers', () => {
    expect(() => validateRestPointId(-1, rest)).toThrow(ValidationError);
    expect(() => validateRestPointId(1.5, rest)).toThrow(ValidationError);
    expect(() => validateRestPointId(Number.MAX_SAFE_INTEGER + 1, rest)).toThrow(ValidationError);
  });

  it('skips validation entirely for non-REST backends', () => {
    expect(() => validateRestPointId('anything', wasm)).not.toThrow();
    expect(() => validateRestPointId(-1, wasm)).not.toThrow();
  });
});

describe('requireVector / requireDimensionInRange (core parity, VELES-005 / VELES-032)', () => {
  it('requireVector rejects non-finite components with VELES-005', () => {
    expect(() => requireVector([0.1, NaN], 'Vector')).toThrow(InvalidVectorError);
    expect(() => requireVector([0.1, Infinity], 'Vector')).toThrow(/index 1/);
    expect(() => requireVector([0.1, 0.2], 'Vector')).not.toThrow();
    expect(() => requireVector(new Float32Array([0.1, NaN]), 'Vector')).toThrow(InvalidVectorError);
    expect(() => requireVector('nope', 'Vector')).toThrow(ValidationError);
  });

  it('requireDimensionInRange enforces core 1..=65536', () => {
    expect(() => requireDimensionInRange(1)).not.toThrow();
    expect(() => requireDimensionInRange(65_536)).not.toThrow();
    expect(() => requireDimensionInRange(0)).toThrow(InvalidDimensionError);
    expect(() => requireDimensionInRange(65_537)).toThrow(InvalidDimensionError);
    expect(() => requireDimensionInRange(2.5)).toThrow(InvalidDimensionError);
    expect(() => requireDimensionInRange('128')).toThrow(InvalidDimensionError);
  });
});
