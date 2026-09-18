/**
 * WASM Backend — Shared type definitions
 *
 * Internal context interface used by wasm-search.ts and wasm-stubs.ts
 * to access WasmBackend internals without circular dependencies.
 */

import type { CollectionConfig } from '../types';
import type { SparseVector } from '../types';
import type {
  VectorStore as BindingVectorStore,
  hybrid_search_fuse as bindingHybridSearchFuse,
} from '@wiscale/velesdb-wasm';

/**
 * The full parameter list of a velesdb-wasm function, read from the
 * binding's declaration file, with its optional parameters made required.
 *
 * wasm-bindgen emits every `Option<T>` as an optional parameter, so a call
 * that leaves one out type-checks against the binding's own declaration: the
 * SDK computed `multi_query_search`'s `weights` and never passed them
 * (#2095). Declared with this list, every binding method the SDK calls must
 * receive every argument (`null` for "none"), so a new or unpassed argument
 * fails the typecheck. `memory.ts` declares `MemoryService` the same way.
 */
export type AllParams<F> = F extends (...args: infer P) => unknown ? Required<P> : never;

/** {@link AllParams} of a velesdb-wasm class's constructor. */
export type AllConstructorParams<C> = C extends abstract new (...args: infer P) => unknown
  ? Required<P>
  : never;

/** {@link AllParams} of a `VectorStore` method. */
type StoreParams<M extends keyof BindingVectorStore> = AllParams<BindingVectorStore[M]>;

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
  // Every method takes the binding's full parameter list (`StoreParams`);
  // only the result shapes, which the binding types as `any`, are stated here.

  /** Release WASM memory. */
  free(...args: StoreParams<'free'>): void;

  /** Insert a vector by ID. */
  insert(...args: StoreParams<'insert'>): void;

  /** Insert a vector with JSON payload. */
  insert_with_payload(...args: StoreParams<'insert_with_payload'>): void;

  /** Batch insert: array of [id, vector] pairs. */
  insert_batch(...args: StoreParams<'insert_batch'>): void;

  /** Pre-allocate memory for additional vectors. */
  reserve(...args: StoreParams<'reserve'>): void;

  /** Remove a vector by ID. Returns true if found. It leaves the ID's sparse postings in place. */
  remove(...args: StoreParams<'remove'>): boolean;

  /** Get a point by ID. Returns point object or null. */
  get(...args: StoreParams<'get'>): WasmPoint | null;

  /** Whether the store is empty (getter property). */
  readonly is_empty: BindingVectorStore['is_empty'];

  /** Number of vectors in the store (getter property). */
  readonly len: BindingVectorStore['len'];

  /** k-NN dense search. Returns array of [id, score] tuples. */
  search(...args: StoreParams<'search'>): WasmDenseResult[];

  /**
   * k-NN dense search under a named quality preset. Returns the same
   * `[id, score]` tuples as {@link search}: WASM search is brute force, so
   * the preset tunes nothing. It is parsed all the same
   * (`parse_search_quality`), which is the only place an unparseable preset
   * is refused — the SDK has no second copy of that grammar.
   */
  search_with_quality(...args: StoreParams<'search_with_quality'>): WasmDenseResult[];

  /** k-NN search with metadata filter. Returns array of {id, score, payload}. */
  search_with_filter(...args: StoreParams<'search_with_filter'>): WasmFilteredResult[];

  /** Add a document's sparse postings. Nothing removes them afterwards (`wasm-sparse.ts`). */
  sparse_insert(...args: StoreParams<'sparse_insert'>): void;

  /** Sparse index search. Returns array of {doc_id, score}. */
  sparse_search(...args: StoreParams<'sparse_search'>): WasmSparseResult[];

  /**
   * Text search on payload fields. The third argument names one payload
   * field to match; this method takes no filter.
   */
  text_search(...args: StoreParams<'text_search'>): WasmSearchResultItem[];

  /** Hybrid vector + text search. Returns array of {id, score, payload}. */
  hybrid_search(...args: StoreParams<'hybrid_search'>): WasmHybridResult[];

  /**
   * Multi-query search with fusion: `(vectors, num_vectors, k, strategy,
   * rrf_k, weights)`, `weights` being the weighted strategy's
   * `[avg, max, hit]`. Returns mixed result items.
   */
  multi_query_search(...args: StoreParams<'multi_query_search'>): WasmSearchResultItem[];

  /** VelesQL-style query returning multi-model results. */
  query(...args: StoreParams<'query'>): Record<string, unknown>[];
}

// ---------------------------------------------------------------------------
// WasmModule — typed interface for the imported WASM package
// ---------------------------------------------------------------------------

/** The static side of the VectorStore class exported by velesdb-wasm. */
export interface WasmVectorStoreConstructor {
  /** Create a store holding its vectors in `mode` (`full`, `sq8`, `binary`, …). */
  new_with_mode(...args: AllParams<typeof BindingVectorStore.new_with_mode>): WasmVectorStore;

  /** Create a store with no vectors: it holds a collection's sparse index (`wasm-sparse.ts`). */
  new_metadata_only(
    ...args: AllParams<typeof BindingVectorStore.new_metadata_only>
  ): WasmVectorStore;
}

/** Typed interface for the @wiscale/velesdb-wasm module. */
export interface WasmModule {
  /** WASM initialization function (must be called once before use).
   *
   * The one binding function not declared with {@link AllParams}: a browser
   * calls it with no argument and Node with the module bytes.
   *
   * Accepts an optional argument that wasm-bindgen forwards to its loader:
   *  - browser: omit, the loader will fetch the .wasm next to the JS module.
   *  - Node.js: pass a `BufferSource` (e.g. `await fs.readFile(...)`) because
   *    Node's stdlib fetch has no `file://` scheme handler. The WasmBackend
   *    helper does this transparently when running under Node.
   */
  default(moduleOrPath?: Uint8Array | URL | string): Promise<void>;

  /** VectorStore class constructor. */
  VectorStore: WasmVectorStoreConstructor;

  /** Fuse dense + sparse search results via Reciprocal Rank Fusion. */
  hybrid_search_fuse(...args: AllParams<typeof bindingHybridSearchFuse>): WasmSparseResult[];
}

/**
 * Sparse bookkeeping for one collection: each sparse upsert is indexed under
 * a fresh sparse id, and a replaced or deleted point's id is retired, since
 * the binding cannot remove postings; the index is rebuilt once retired ids
 * outnumber live ones (`backends/wasm-sparse.ts`).
 */
export interface SparseIds {
  /** The metadata-only store holding the sparse index; `null` until the first sparse upsert. */
  store: WasmVectorStore | null;
  /** Live sparse id of each point that has a sparse vector, by numeric point id. */
  byPoint: Map<number, bigint>;
  /** Point behind each live sparse id. */
  byId: Map<bigint, number>;
  /** Each live sparse id's vector, kept to rebuild the index from. */
  vectors: Map<bigint, { indices: Uint32Array; values: Float32Array }>;
  /** Retired ids the binding still holds; a search over-fetches by this many. */
  dead: number;
  /** Next sparse id to hand out. */
  next: bigint;
}

/** In-memory collection storage */
export interface CollectionData {
  config: CollectionConfig;
  store: WasmVectorStore;
  payloads: Map<string, Record<string, unknown>>;
  sparseIds: SparseIds;
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
