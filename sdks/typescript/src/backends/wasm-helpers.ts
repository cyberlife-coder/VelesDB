/**
 * WASM Backend - Internal helper functions
 *
 * ID normalization, payload key computation, sparse vector conversion,
 * and WasmContext construction helpers extracted from the WasmBackend class.
 * @packageDocumentation
 */

import type { SparseVector } from '../types';
import type { CollectionData, WasmModule, WasmContext } from './wasm-types';

/** Normalize a string ID that looks like a pure integer. */
export function normalizeIdString(id: string): string | null {
  const trimmed = id.trim();
  return /^\d+$/.test(trimmed) ? trimmed : null;
}

/** Convert an arbitrary result ID (bigint/number/string) to a canonical payload key. */
export function canonicalPayloadKeyFromResultId(id: bigint | number | string): string {
  if (typeof id === 'bigint') {
    return id.toString();
  }
  if (typeof id === 'number') {
    return String(Math.trunc(id));
  }
  const normalized = normalizeIdString(id);
  if (normalized !== null) {
    return normalized.replace(/^0+(?=\d)/, '');
  }
  return String(toNumericId(id));
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
