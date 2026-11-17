/**
 * WASM Helpers Tests (#598)
 *
 * Covers the pure helpers in `src/backends/wasm-helpers.ts`:
 * normalizeIdString, canonicalPayloadKey, canonicalPayloadKeyFromResultId,
 * toPointId, exactGraphId, sparseVectorToArrays, buildWasmContext,
 * buildCollectionInfo.
 * These are all pure/synchronous — no fetch or WASM module instantiation
 * required.
 */

import { describe, it, expect } from 'vitest';
import {
  normalizeIdString,
  canonicalPayloadKey,
  canonicalPayloadKeyFromResultId,
  toPointId,
  exactGraphId,
  sparseVectorToArrays,
  buildWasmContext,
  buildCollectionInfo,
  describeWasmThrow,
} from '../src/backends/wasm-helpers';
import { newSparseIds } from '../src/backends/wasm-sparse';
import { VelesDBError } from '../src/types';
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

  it('falls back to toPointId hash for non-integer strings', () => {
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

describe('toPointId', () => {
  it('takes a number as that integer', () => {
    expect(toPointId(42)).toBe(42n);
    expect(toPointId(0)).toBe(0n);
  });

  it('parses a decimal string, trimmed and without leading zeros', () => {
    expect(toPointId('42')).toBe(42n);
    expect(toPointId(' 0042 ')).toBe(42n);
  });

  it('hashes non-integer strings to a non-negative integer', () => {
    const h = toPointId('hello');
    expect(typeof h).toBe('bigint');
    expect(h >= 0n).toBe(true);
    expect(String(h)).toBe(canonicalPayloadKey('hello'));
  });

  it('returns same hash for same string (deterministic)', () => {
    expect(toPointId('hello')).toBe(toPointId('hello'));
  });

  it('keeps a decimal id past MAX_SAFE_INTEGER exactly, up to u64::MAX', () => {
    // The binding returns every point id as its decimal string, so the id
    // comes back as it went in: no hash, no rounding, no refusal.
    expect(toPointId('9007199254740993')).toBe(9007199254740993n);
    expect(toPointId('18446744073709551615')).toBe(18446744073709551615n);
  });

  it('refuses a decimal id past u64::MAX instead of hashing it into another point', () => {
    expect(() => toPointId('18446744073709551616')).toThrow(VelesDBError);
    expect(() => toPointId('18446744073709551616')).toThrow(/u64::MAX/);
  });
});

describe('exactGraphId', () => {
  it('keeps a decimal id past 2^53 exactly', () => {
    expect(exactGraphId('9007199254740993', 'Edge id')).toBe('9007199254740993');
  });

  it('writes a safe integer as its decimal string and trims a string id', () => {
    expect(exactGraphId(42, 'Edge id')).toBe('42');
    expect(exactGraphId(' 42 ', 'Edge id')).toBe('42');
  });

  it('refuses anything a u64 id cannot be, naming the field', () => {
    for (const bad of ['doc-a', '', '-1', '1.5', -1, 1.5, 2 ** 53, Number.NaN]) {
      expect(() => exactGraphId(bad, 'Edge source')).toThrow(/Edge source/);
    }
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
      sparseIds: newSparseIds(),
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
    expect(ctx.toPointId('7')).toBe(7n);
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
      sparseIds: newSparseIds(),
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
      sparseIds: newSparseIds(),
      createdAt: new Date(),
    };

    const info = buildCollectionInfo('empty', data);
    expect(info.dimension).toBe(0);
    expect(info.metric).toBe('cosine');
    expect(info.count).toBe(0);
  });
});

/**
 * `describeWasmThrow` runs INSIDE a catch block, so it must be total: if it
 * throws, the reason it was written to salvage is replaced by its own
 * `TypeError` and the caller reads a message about primitive conversion
 * instead of what the binding said. Every shape below is one a wasm binding
 * can hand back — `Result::Err(String)` is the common one, and the exotic
 * ones (prototype-less object, poisoned `toString`) are the same hazard
 * `describeValue` (`wasm-search.ts`) and `toTypedError` (`memory.ts`)
 * already guard against elsewhere in this SDK.
 */
describe('describeWasmThrow is total (#2282)', () => {
  it("returns a bare string verbatim — wasm-bindgen's Result::Err(String)", () => {
    expect(describeWasmThrow('expected magic word 00 61 73 6d')).toBe(
      'expected magic word 00 61 73 6d'
    );
  });

  it("returns an Error's message", () => {
    expect(describeWasmThrow(new Error('boom'))).toBe('boom');
  });

  it("returns a WebAssembly.RuntimeError's message", () => {
    expect(describeWasmThrow(new WebAssembly.RuntimeError('unreachable'))).toBe(
      'unreachable'
    );
  });

  it.each([
    ['undefined', undefined, 'undefined'],
    ['null', null, 'null'],
    ['a number', 42, '42'],
    ['a plain object', { a: 1 }, '[object Object]'],
  ] as const)('coerces %s', (_label, thrown, expected) => {
    expect(describeWasmThrow(thrown)).toBe(expected);
  });

  it('names a prototype-less object by its type instead of coercing it', () => {
    // `String(Object.create(null))` throws "Cannot convert object to
    // primitive value": the value has no `toString`, and no prototype to
    // inherit one from.
    const thrown: unknown = Object.create(null);
    expect(() => describeWasmThrow(thrown)).not.toThrow();
    expect(describeWasmThrow(thrown)).toBe('a value of type object');
  });

  it('survives a toString that throws', () => {
    const thrown = {
      toString() {
        throw new TypeError('poisoned');
      },
    };
    expect(() => describeWasmThrow(thrown)).not.toThrow();
    expect(describeWasmThrow(thrown)).toBe('a value of type object');
  });

  it('survives an Error whose message getter throws', () => {
    const thrown = new Error('unread');
    Object.defineProperty(thrown, 'message', {
      get() {
        throw new TypeError('poisoned');
      },
    });
    expect(() => describeWasmThrow(thrown)).not.toThrow();
    expect(describeWasmThrow(thrown)).toBe('a value of type object');
  });

  it('survives a revoked proxy, whose prototype walk throws', () => {
    const { proxy, revoke } = Proxy.revocable({}, {});
    revoke();
    expect(() => describeWasmThrow(proxy)).not.toThrow();
    expect(describeWasmThrow(proxy)).toBe('a value of type object');
  });

  it('survives a symbol, which interpolation refuses to coerce', () => {
    expect(() => describeWasmThrow(Symbol('s'))).not.toThrow();
  });
});

