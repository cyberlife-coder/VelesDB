/**
 * WASM Backend - Internal helper functions
 *
 * ID normalization, payload key computation, sparse vector conversion,
 * and WasmContext construction helpers extracted from the WasmBackend class.
 * @packageDocumentation
 */

import type { SparseVector } from '../types';
import { VelesDBError } from '../types';
import type { CollectionData, WasmModule, WasmContext } from './wasm-types';

/**
 * A guarded read: captures the result, or the fact that reading threw.
 *
 * The one primitive every total reader of a foreign value is built on — a
 * revoked proxy throws on the prototype walk, a poisoned getter throws on
 * the property read, and a prototype-less object throws on coercion.
 */
export function tryRead<T>(read: () => T): { ok: boolean; value: T | undefined } {
  try {
    return { ok: true, value: read() };
  } catch {
    return { ok: false, value: undefined };
  }
}

/**
 * Read what velesdb-wasm threw, without assuming it threw an `Error`.
 *
 * wasm-bindgen raises a `Result::Err(String)` by throwing **the string
 * itself**: probed against `@wiscale/velesdb-wasm` 6.0.0, an unparseable
 * search quality gives `typeof thrown === 'string'` and
 * `thrown instanceof Error === false`. So `thrown.message` reads
 * `undefined` and `thrown instanceof Error` never holds — every catch
 * around a binding call goes through this one reader, and re-raises a typed
 * SDK error of its own so a caller can still narrow on the class.
 *
 * **Total by construction.** A reader that runs *inside* a catch block must
 * never throw, or the reason it was written to salvage is replaced by its
 * own `TypeError` — `thrown instanceof Error` detonates on a revoked proxy
 * and `String(thrown)` detonates on a prototype-less object or a hostile
 * `toString`, both shapes a wasm binding can hand back. Every step is
 * guarded, and the last resort names the value by its type rather than
 * coercing it, as {@link describeValue} does in `wasm-search.ts`.
 */
export function describeWasmThrow(thrown: unknown): string {
  const isError = tryRead(() => thrown instanceof Error);
  if (isError.value === true) {
    const message = tryRead(() => (thrown as Error).message);
    if (typeof message.value === 'string') {
      return message.value;
    }
  }
  const text = tryRead(() => String(thrown));
  return typeof text.value === 'string' ? text.value : `a value of type ${typeof thrown}`;
}

/** Normalize a string ID that looks like a pure integer. */
export function normalizeIdString(id: string): string | null {
  const trimmed = id.trim();
  return /^\d+$/.test(trimmed) ? trimmed : null;
}

/**
 * Convert an arbitrary result ID (bigint/number/string) to a canonical
 * payload key. A result id is a document id plus the `bigint` the binding
 * hands back, so this is {@link canonicalPayloadKey} with that one extra
 * arm — written as such rather than copied, which is how the two came to
 * be a clone of each other.
 */
export function canonicalPayloadKeyFromResultId(id: bigint | number | string): string {
  return typeof id === 'bigint' ? id.toString() : canonicalPayloadKey(id);
}

/** Convert a document ID to a canonical string key for the payload map. */
export function canonicalPayloadKey(id: string | number): string {
  if (typeof id === 'number') {
    return String(Math.trunc(id));
  }
  const normalized = normalizeIdString(id);
  if (normalized !== null) {
    return normalized.replace(/^0+(?=\d)/, '');
  }
  return String(toPointId(id));
}

/** Convert a sparse vector object to parallel index/value arrays. */
export function sparseVectorToArrays(sv: SparseVector): { indices: number[]; values: number[] } {
  const indices: number[] = [];
  const values: number[] = [];
  for (const [k, v] of Object.entries(sv)) {
    indices.push(Number(k));
    values.push(v);
  }
  return { indices, values };
}

/**
 * A graph id (edge or node) as the binding takes it: its exact decimal
 * string. Unlike {@link toPointId}, which hashes a document id that is not
 * a number, a graph id is a `u64` and nothing else: a derived edge id is
 * routinely above 2^53, so a lossy or hashed stand-in would address another
 * edge. Anything but a non-negative integer is refused.
 */
export function exactGraphId(id: string | number, what: string): string {
  if (typeof id === 'number') {
    if (isExactU64Number(id)) { return String(id); }
  } else {
    const normalized = normalizeIdString(id);
    if (normalized !== null) { return normalized; }
  }
  throw new VelesDBError(`${what} must be a non-negative integer id, got ${JSON.stringify(id)}`, 'BAD_REQUEST');
}

/**
 * Whether a number names exactly one `u64`: a non-negative safe integer.
 * Past 2^53 a number no longer names one integer, a negative or fractional
 * one names none — and wasm-bindgen would pass it as raw `i64` bits anyway
 * (`-1` arrives as `u64::MAX`, 2^64 as 0) or throw a native `RangeError`.
 */
function isExactU64Number(id: number): boolean {
  return Number.isSafeInteger(id) && id >= 0;
}

/** The largest id a `u64` holds. */
const U64_MAX = 2n ** 64n - 1n;

/**
 * A document id as the binding takes it: the exact `u64` it names, as a
 * `BigInt`.
 *
 * A number is that integer, and must be a non-negative safe integer
 * (`BAD_REQUEST` otherwise, as on REST). A decimal string is its exact value, past
 * `Number.MAX_SAFE_INTEGER` too: the binding returns every point id as its
 * decimal string, so such an id comes back from a search or a `get` as it
 * went in, as on REST. One past `u64::MAX` names no point and is refused
 * (`BAD_REQUEST`) rather than hashed into another. Any other string keeps
 * its stable hash.
 */
export function toPointId(id: string | number): bigint {
  if (typeof id === 'number') {
    if (isExactU64Number(id)) { return BigInt(id); }
    throw new VelesDBError(
      `Point id must be a non-negative integer no larger than Number.MAX_SAFE_INTEGER (or its decimal string, up to u64::MAX), got ${String(id)}`,
      'BAD_REQUEST'
    );
  }
  const normalized = normalizeIdString(id);
  if (normalized === null) {
    return BigInt(stringHash(id));
  }
  const exact = BigInt(normalized);
  if (exact > U64_MAX) {
    throw new VelesDBError(
      `Point id ${normalized} is past u64::MAX (${U64_MAX}); no point can carry it`,
      'BAD_REQUEST'
    );
  }
  return exact;
}

/** The stable non-negative hash a non-numeric document id is stored under. */
function stringHash(id: string): number {
  let hash = 0;
  for (let i = 0; i < id.length; i++) {
    const char = id.charCodeAt(i);
    hash = ((hash << 5) - hash) + char;
    hash = hash & hash;
  }
  return Math.abs(hash);
}

/** Build a WasmContext from the backend's internal state. */
export function buildWasmContext(
  wasmModule: WasmModule,
  collections: Map<string, CollectionData>
): WasmContext {
  return {
    wasmModule,
    getCollection: (name: string) => collections.get(name),
    canonicalPayloadKeyFromResultId: (id) => canonicalPayloadKeyFromResultId(id),
    canonicalPayloadKey: (id) => canonicalPayloadKey(id),
    sparseVectorToArrays: (sv) => sparseVectorToArrays(sv),
    toPointId: (id) => toPointId(id),
  };
}

/** Build a Collection info object from internal CollectionData. */
export function buildCollectionInfo(
  name: string,
  data: CollectionData
): {
  name: string;
  dimension: number;
  metric: 'cosine' | 'euclidean' | 'dot' | 'hamming' | 'jaccard';
  count: number;
  createdAt: Date;
} {
  return {
    name,
    dimension: data.config.dimension ?? 0,
    metric: data.config.metric ?? 'cosine',
    count: data.store.len,
    createdAt: data.createdAt,
  };
}

/** Serialize `params` for the executor: typed arrays become JSON arrays. */
export function paramsJson(params: Record<string, unknown> | undefined): string | undefined {
  if (params === undefined) { return undefined; }
  return JSON.stringify(params, (_key, value: unknown) =>
    ArrayBuffer.isView(value) && !(value instanceof DataView)
      ? Array.from(value as unknown as ArrayLike<number>)
      : value
  );
}
