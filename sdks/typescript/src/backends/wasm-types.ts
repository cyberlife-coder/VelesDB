/**
 * WASM Backend — Shared type definitions
 *
 * Internal context interface used by wasm-search.ts and wasm-stubs.ts
 * to access WasmBackend internals without circular dependencies.
 */

import type { CollectionConfig } from '../types';
import type { SparseVector } from '../types';

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type WasmModule = any;

/** In-memory collection storage */
export interface CollectionData {
  config: CollectionConfig;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  store: any;
  payloads: Map<string, Record<string, unknown>>;
  createdAt: Date;
}

/**
 * Internal context passed from WasmBackend to extracted search/stub modules.
 *
 * Exposes the minimum surface needed by helper functions without leaking the
 * full class. All methods mirror private WasmBackend helpers.
 */
export interface WasmContext {
  wasmModule: WasmModule;
  getCollection(name: string): CollectionData | undefined;
  canonicalPayloadKeyFromResultId(id: bigint | number | string): string;
  canonicalPayloadKey(id: string | number): string;
  sparseVectorToArrays(sv: SparseVector): { indices: number[]; values: number[] };
  toNumericId(id: string | number): number;
}
