/**
 * WASM Backend — Shared type definitions
 *
 * Internal context interface used by wasm-search.ts and wasm-stubs.ts
 * to access WasmBackend internals without circular dependencies.
 */

import type { CollectionConfig } from '../types';
import type { SparseVector } from '../types';
import type { FilterInput } from '../filter';

// ---------------------------------------------------------------------------
// WASM result types — mirror the shapes returned by velesdb-wasm
// ---------------------------------------------------------------------------

/** Dense search result: [id, score] tuple returned by VectorStore.search(). */
export type WasmDenseResult = [bigint, number];

/** Sparse/hybrid search result returned by sparse_search / hybrid_search_fuse. */
export interface WasmSparseResult {
  doc_id: bigint | number;
  score: number;
}

/** Filtered search result returned by VectorStore.search_with_filter(). */
export interface WasmFilteredResult {
  id: bigint;
  score: number;
  payload?: Record<string, unknown> | null;
}

/** Hybrid search result returned by VectorStore.hybrid_search(). */
export interface WasmHybridResult {
  id: bigint | number;
  score: number;
  payload?: Record<string, unknown>;
}

/** Point returned by VectorStore.get(). */
export interface WasmPoint {
  id: bigint | number;
  vector: number[] | Float32Array;
  payload?: Record<string, unknown> | null;
}

/** Generic search result (tuple or object) returned by text_search / multi_query_search. */
export type WasmSearchResultItem =
  | WasmDenseResult
  | WasmHybridResult;

// ---------------------------------------------------------------------------
// VectorStore — typed interface for the WASM VectorStore class
// ---------------------------------------------------------------------------

/** Typed interface for the velesdb-wasm VectorStore class instance. */
export interface WasmVectorStore {
  /** Release WASM memory. */
  free(): void;

  /** Insert a vector by ID. */
  insert(id: bigint, vector: Float32Array): void;

  /** Insert a vector with JSON payload. */
  insert_with_payload(id: bigint, vector: Float32Array, payload: unknown): void;

  /** Batch insert: array of [id, vector] pairs. */
  insert_batch(batch: Array<[bigint, number[]]>): void;

  /** Pre-allocate memory for additional vectors. */
  reserve(additional: number): void;

  /** Remove a vector by ID. Returns true if found. */
  remove(id: bigint): boolean;

  /** Get a point by ID. Returns point object or null. */
  get(id: bigint): WasmPoint | null;

  /** Whether the store is empty (getter property). */
  readonly is_empty: boolean;

  /** Number of vectors in the store (getter property). */
  readonly len: number;

  /** k-NN dense search. Returns array of [id, score] tuples. */
  search(query: Float32Array, k: number): WasmDenseResult[];

  /** k-NN search with metadata filter. Returns array of {id, score, payload}. */
  search_with_filter(
    query: Float32Array,
    k: number,
    filter: FilterInput
  ): WasmFilteredResult[];

  /** Sparse index search. Returns array of {doc_id, score}. */
  sparse_search(
    indices: Uint32Array,
    values: Float32Array,
    k: number
  ): WasmSparseResult[];

  /** Text search on payload fields. Returns mixed result items. */
  text_search(
    query: string,
    k: number,
    field: string | undefined
  ): WasmSearchResultItem[];

  /** Hybrid vector + text search. Returns array of {id, score, payload}. */
  hybrid_search(
    queryVector: Float32Array,
    textQuery: string,
    k: number,
    vectorWeight: number | undefined
  ): WasmHybridResult[];

  /** Multi-query search with fusion strategy. Returns mixed result items. */
  multi_query_search(
    vectors: Float32Array,
    numVectors: number,
    k: number,
    strategy: string,
    rrfK: number
  ): WasmSearchResultItem[];

  /** VelesQL-style query returning multi-model results. */
  query(queryVector: Float32Array, k: number): Record<string, unknown>[];
}

// ---------------------------------------------------------------------------
// WasmModule — typed interface for the imported WASM package
// ---------------------------------------------------------------------------

/** Constructor signature for the VectorStore class exported by velesdb-wasm. */
export interface WasmVectorStoreConstructor {
  new (dimension: number, metric: string): WasmVectorStore;
}

/** Typed interface for the @wiscale/velesdb-wasm module. */
export interface WasmModule {
  /** WASM initialization function (must be called once before use). */
  default(): Promise<void>;

  /** VectorStore class constructor. */
  VectorStore: WasmVectorStoreConstructor;

  /** Fuse dense + sparse search results via Reciprocal Rank Fusion. */
  hybrid_search_fuse(
    denseResults: Array<[number, number]>,
    sparseResults: Array<[number, number]>,
    rrfK: number,
    k?: number
  ): WasmSparseResult[];
}

/** In-memory collection storage */
export interface CollectionData {
  config: CollectionConfig;
  store: WasmVectorStore;
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
