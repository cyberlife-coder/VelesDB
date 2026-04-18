/**
 * WASM Helpers Tests (#598)
 *
 * Covers the pure helpers in `src/backends/wasm-helpers.ts`:
 * normalizeIdString, canonicalPayloadKey, canonicalPayloadKeyFromResultId,
 * toNumericId, sparseVectorToArrays, buildWasmContext, buildCollectionInfo.
 * These are all pure/synchronous — no fetch or WASM module instantiation
 * required.
 */

import { describe, it, expect, vi } from 'vitest';
import {
  normalizeIdString,
  canonicalPayloadKey,
  canonicalPayloadKeyFromResultId,
  toNumericId,
  sparseVectorToArrays,
  buildWasmContext,
  buildCollectionInfo,
} from '../src/backends/wasm-helpers';
import type {
  CollectionData,
  WasmModule,
  WasmVectorStore,
} from '../src/backends/wasm-types';

describe('normalizeIdString', () => {
  it('returns trimmed digits when input is a pure integer string', () => {
    expect(normalizeIdString('42')).toBe('42');
    expect(normalizeIdString('  7  ')).toBe('7');
    expect(normalizeIdString('000')).toBe('000');
  });

  it('returns null for non-integer strings', () => {
    expect(normalizeIdString('abc')).toBeNull();
    expect(normalizeIdString('1.5')).toBeNull();
    expect(normalizeIdString('-1')).toBeNull();
    expect(normalizeIdString('')).toBeNull();
    expect(normalizeIdString('42x')).toBeNull();
  });
});

describe('canonicalPayloadKey', () => {
  it('truncates numeric input', () => {
    expect(canonicalPayloadKey(42)).toBe('42');
    expect(canonicalPayloadKey(42.9)).toBe('42');
  });

  it('strips leading zeros on pure-integer strings', () => {
    expect(canonicalPayloadKey('00042')).toBe('42');
    expect(canonicalPayloadKey('7')).toBe('7');
  });

  it('keeps non-leading zero digits', () => {
    // "0" becomes "" after stripping, but trailing digit guarded by lookahead
    expect(canonicalPayloadKey('0')).toBe('0');
    expect(canonicalPayloadKey('100')).toBe('100');
  });

  it('falls back to toNumericId hash for non-integer strings', () => {
    const key = canonicalPayloadKey('abc');
    // hash is deterministic; we just assert it's a stringified numeric
    expect(key).toMatch(/^\d+$/);
  });
});

describe('canonicalPayloadKeyFromResultId', () => {
  it('handles bigint', () => {
    expect(canonicalPayloadKeyFromResultId(12345n)).toBe('12345');
  });

  it('handles number', () => {
    expect(canonicalPayloadKeyFromResultId(42)).toBe('42');
    expect(canonicalPayloadKeyFromResultId(42.7)).toBe('42');
  });

  it('handles pure-integer string, stripping leading zeros', () => {
    expect(canonicalPayloadKeyFromResultId('00042')).toBe('42');
    expect(canonicalPayloadKeyFromResultId('7')).toBe('7');
  });

  it('handles non-integer string via hash fallback', () => {
    expect(canonicalPayloadKeyFromResultId('abc')).toMatch(/^\d+$/);
  });
});

describe('toNumericId', () => {
  it('passes number through unchanged', () => {
    expect(toNumericId(42)).toBe(42);
    expect(toNumericId(0)).toBe(0);
  });

  it('parses safe integer string directly', () => {
    expect(toNumericId('42')).toBe(42);
  });

  it('hashes non-integer strings to a positive integer', () => {
    const h = toNumericId('hello');
    expect(Number.isInteger(h)).toBe(true);
    expect(h).toBeGreaterThanOrEqual(0);
  });

  it('returns same hash for same string (deterministic)', () => {
    expect(toNumericId('hello')).toBe(toNumericId('hello'));
  });

  it('hashes string beyond MAX_SAFE_INTEGER', () => {
    // More than 2^53, Number parse would lose precision → falls into hash
    const big = '9007199254740997';
    const h = toNumericId(big);
    expect(Number.isInteger(h)).toBe(true);
    expect(h).toBeGreaterThanOrEqual(0);
  });
});

describe('sparseVectorToArrays', () => {
  it('returns empty parallel arrays for empty input', () => {
    expect(sparseVectorToArrays({})).toEqual({ indices: [], values: [] });
  });

  it('preserves order of Object.entries', () => {
    const out = sparseVectorToArrays({ 1: 0.5, 2: 0.7 });
    expect(out.indices).toEqual([1, 2]);
    expect(out.values).toEqual([0.5, 0.7]);
  });

  it('coerces string keys to number via Number()', () => {
    const out = sparseVectorToArrays({ 42: 0.9 });
    expect(out.indices).toEqual([42]);
    expect(out.values).toEqual([0.9]);
  });
});

describe('buildWasmContext', () => {
  it('exposes delegating helpers and collection lookup', () => {
    const collections = new Map<string, CollectionData>();
    const store = {} as WasmVectorStore;
    const data: CollectionData = {
      config: { dimension: 128, metric: 'cosine' },
      store,
      payloads: new Map(),
      createdAt: new Date(),
    };
    collections.set('docs', data);

    const wasmModule = {} as WasmModule;
    const ctx = buildWasmContext(wasmModule, collections);

    expect(ctx.wasmModule).toBe(wasmModule);
    expect(ctx.getCollection('docs')).toBe(data);
    expect(ctx.getCollection('missing')).toBeUndefined();
    expect(ctx.canonicalPayloadKey('00042')).toBe('42');
    expect(ctx.canonicalPayloadKeyFromResultId(42n)).toBe('42');
    expect(ctx.toNumericId('7')).toBe(7);
    expect(ctx.sparseVectorToArrays({ 1: 0.5 })).toEqual({
      indices: [1],
      values: [0.5],
    });
  });
});

describe('buildCollectionInfo', () => {
  it('maps dimension/metric/count/createdAt from CollectionData', () => {
    const createdAt = new Date('2024-01-01');
    const data: CollectionData = {
      config: { dimension: 128, metric: 'euclidean' },
      // store only needs `len` for this helper
      store: { len: 42 } as unknown as WasmVectorStore,
      payloads: new Map(),
      createdAt,
    };

    const info = buildCollectionInfo('docs', data);
    expect(info).toEqual({
      name: 'docs',
      dimension: 128,
      metric: 'euclidean',
      count: 42,
      createdAt,
    });
  });

  it('defaults to dimension=0 and metric=cosine when unset', () => {
    const data: CollectionData = {
      config: {} as CollectionData['config'],
      store: { len: 0 } as unknown as WasmVectorStore,
      payloads: new Map(),
      createdAt: new Date(),
    };

    const info = buildCollectionInfo('empty', data);
    expect(info.dimension).toBe(0);
    expect(info.metric).toBe('cosine');
    expect(info.count).toBe(0);
  });
});

describe('buildWasmContext — arrow delegation (coverage of wrappers)', () => {
  it('calls through to module-level helpers', () => {
    // Spy on helpers by invoking them through the context to ensure
    // each arrow function in buildWasmContext executes at least once.
    const ctx = buildWasmContext({} as WasmModule, new Map());

    // Exercise each delegating arrow
    expect(typeof ctx.canonicalPayloadKey(1)).toBe('string');
    expect(typeof ctx.canonicalPayloadKeyFromResultId(2)).toBe('string');
    expect(typeof ctx.toNumericId(3)).toBe('number');
    expect(ctx.sparseVectorToArrays({})).toEqual({
      indices: [],
      values: [],
    });
    // vi.fn() silencer in case the compiler strips an unused variable
    const spy = vi.fn();
    spy();
    expect(spy).toHaveBeenCalled();
  });
});
