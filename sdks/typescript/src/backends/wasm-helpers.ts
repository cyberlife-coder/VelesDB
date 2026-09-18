/**
 * WASM Backend - Internal helper functions
 *
 * ID normalization, payload key computation, sparse vector conversion,
 * and WasmContext construction helpers extracted from the WasmBackend class.
 * @packageDocumentation
 */

import type { SparseVector } from '../types';
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
  return String(toNumericId(id));
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

/** Convert a string or number document ID to a numeric ID. */
export function toNumericId(id: string | number): number {
  if (typeof id === 'number') {
    return id;
  }
  const normalized = normalizeIdString(id);
  if (normalized !== null) {
    const parsed = Number(normalized);
    if (Number.isSafeInteger(parsed)) {
      return parsed;
    }
  }
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
    toNumericId: (id) => toNumericId(id),
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
