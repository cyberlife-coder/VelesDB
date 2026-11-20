/**
 * WASM Backend - Internal helper functions
 *
 * ID normalization, payload key computation, sparse vector conversion,
 * and WasmContext construction helpers extracted from the WasmBackend class.
 * @packageDocumentation
 */

import { DimensionMismatchError, parseVelesError, VectorRequiredError } from '../errors';
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

/** A core error code, as velesdb-wasm sets it on the `Error` it throws. */
const VELES_CODE = /^VELES-\d{3}$/;

/**
 * The SDK error for what velesdb-wasm threw: the typed class of the core
 * code it carries as `error.code` (VELES-034 an `InvalidCollectionNameError`,
 * VELES-038 a `QueryParameterError`, …), or a `VelesDBError` with `fallback`
 * when it carries none — a bare string, the binding's own refusals. Total,
 * as {@link describeWasmThrow} is: it runs inside a catch block.
 */
export function wasmThrowToError(thrown: unknown, fallback = 'BAD_REQUEST'): VelesDBError {
  const message = describeWasmThrow(thrown);
  const code = tryRead(() => (thrown as { code?: unknown }).code).value;
  return typeof code === 'string' && VELES_CODE.test(code)
    ? parseVelesError(code, message)
    : new VelesDBError(message, fallback);
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

/** The largest sparse term id: `u32::MAX`. */
const U32_MAX = 4294967295;

/**
 * A sparse key as the term id it names: a run of decimal digits no larger
 * than `u32::MAX`, as core's `SparseVectorInput::convert_dict` parses it on
 * REST. Anything else is refused (`BAD_REQUEST`): the `Uint32Array` the
 * binding takes would otherwise coerce it to another term — `'foo'` to 0,
 * `'-1'` to `u32::MAX`, `'4294967296'` to 0, `'1.5'` to 1.
 */
function sparseTermId(key: string): number {
  const term = /^\d+$/.test(key) ? Number(key) : NaN;
  if (term <= U32_MAX) { return term; }
  throw new VelesDBError(
    `Sparse vector key '${key}' is not a valid u32 term ID (0..${U32_MAX})`,
    'BAD_REQUEST'
  );
}

/**
 * Convert a sparse vector object to parallel index/value arrays, refusing a
 * key that is not a u32 term id ({@link sparseTermId}).
 */
export function sparseVectorToArrays(sv: SparseVector): { indices: number[]; values: number[] } {
  const indices: number[] = [];
  const values: number[] = [];
  for (const [k, v] of Object.entries(sv)) {
    indices.push(sparseTermId(k));
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
  // An untyped caller can leave the id out (REST requires it) or pass any
  // value: only a string or a number can name a u64.
  if (typeof id !== 'string' && typeof id !== 'number') {
    const type = id === null ? 'null' : typeof id;
    throw new VelesDBError(`${what} must be a string or number id, got ${type}`, 'BAD_REQUEST');
  }
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

/**
 * A `VectorStore.query()` row (`{nodeId, vectorScore, …, bindings}`, the
 * point's payload in `bindings`) as core's `SELECT *` row — the one the
 * executor and REST return: the point id, then every payload field but
 * `id` (`project_wildcard`, `collection/search/query/projection.rs`). The
 * id is a number, as core writes it, or its exact decimal string past 2^53.
 */
export function selectStarRow(row: Record<string, unknown>): Record<string, unknown> {
  const nodeId = String(row.nodeId);
  const asNumber = Number(nodeId);
  const projected: Record<string, unknown> = {
    id: Number.isSafeInteger(asNumber) ? asNumber : nodeId,
  };
  const payload = row.bindings;
  if (payload !== null && typeof payload === 'object' && !Array.isArray(payload)) {
    for (const [field, value] of Object.entries(payload)) {
      if (field !== 'id') { projected[field] = value; }
    }
  }
  return projected;
}

/**
 * Refuses a vector written to the collection `collection` of `dimension` as
 * core refuses it, so the WASM backend throws what the REST backend relays
 * for the same input: an empty vector on a collection that stores vectors
 * is a `VectorRequiredError` (VELES-016) — every row of a vector collection
 * stores one — and a vector of another length a `DimensionMismatchError`
 * (VELES-004), its message led by `mismatch`. A sparse-only collection
 * (dimension 0) takes the empty vector.
 */
export function requireVectorOfDimension(
  collection: string,
  length: number,
  dimension: number,
  mismatch = 'Vector dimension mismatch',
): void {
  if (length === 0 && dimension > 0) {
    throw new VectorRequiredError(
      `[VELES-016] Vector required for collection '${collection}' (not metadata-only)`,
    );
  }
  if (length !== dimension) {
    throw new DimensionMismatchError(`${mismatch}: expected ${dimension}, got ${length}`);
  }
}
