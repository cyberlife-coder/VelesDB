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
  WasmDatabase as BindingWasmDatabase,
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

/**
 * A point id as the binding returns it: its exact decimal string, as REST
 * sends `SearchResultResponse.id`. Ids go into the binding as `BigInt`. A
 * binding before this release returned a plain number, which threw past
 * 2^53; every reader here takes either (`String(id)`, `BigInt(id)`,
 * {@link WasmContext.canonicalPayloadKeyFromResultId}).
 */
export type WasmPointId = string;

/**
 * Dense search result: [id, score] tuple returned by VectorStore.search(),
 * search_with_quality() and multi_query_search().
 */
export type WasmDenseResult = [WasmPointId, number];

/** Sparse/hybrid search result returned by sparse_search / hybrid_search_fuse. */
export interface WasmSparseResult {
  doc_id: WasmPointId;
  score: number;
}

/** Filtered search result returned by VectorStore.search_with_filter(). */
export interface WasmFilteredResult {
  id: WasmPointId;
  score: number;
  payload?: Record<string, unknown> | null;
}

/**
 * Record returned by VectorStore.hybrid_search(): a plain object since
 * velesdb-wasm 8.0.0 (#2332), `payload` null when the point has none.
 */
export interface WasmHybridResult {
  id: WasmPointId;
  score: number;
  payload?: Record<string, unknown> | null;
}

/**
 * Record returned by VectorStore.text_search() since velesdb-wasm 8.0.0: a
 * case-insensitive substring match selects records and ranks none, so
 * `score` is always null.
 */
export interface WasmTextResult {
  id: WasmPointId;
  score: null;
  payload: Record<string, unknown>;
}

/** Point returned by VectorStore.get(). */
export interface WasmPoint {
  id: WasmPointId;
  vector: number[] | Float32Array;
  payload?: Record<string, unknown> | null;
}

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

  /**
   * Remove a vector by ID. Returns true if found. velesdb-wasm 8.2.0 leaves
   * the ID's sparse postings in place; later releases remove them too.
   */
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

  /**
   * Index a document's sparse vector, replacing the one it had
   * (`wasm-sparse.ts`). Throws the binding's refusal of a non-finite weight
   * before changing anything.
   */
  sparse_insert(...args: StoreParams<'sparse_insert'>): void;

  /**
   * Remove a document's sparse postings; `true` when it had any. Present
   * since velesdb-wasm 8.2.0, the floor of the SDK's dependency range;
   * declared by hand rather than via `StoreParams` so the type does not
   * depend on which binding happens to be installed.
   */
  sparse_remove(doc_id: bigint): boolean;

  /** Sparse index search. Returns array of {doc_id, score}. */
  sparse_search(...args: StoreParams<'sparse_search'>): WasmSparseResult[];

  /**
   * Text search on payload fields. The third argument names one payload
   * field to match; this method takes no filter.
   */
  text_search(...args: StoreParams<'text_search'>): WasmTextResult[];

  /** Hybrid vector + text search. Returns array of {id, score, payload}. */
  hybrid_search(...args: StoreParams<'hybrid_search'>): WasmHybridResult[];

  /**
   * Multi-query search with fusion: `(vectors, num_vectors, k, strategy,
   * rrf_k, weights)`, `weights` being the weighted strategy's
   * `[avg, max, hit]`. Returns `[id, score]` tuples.
   */
  multi_query_search(...args: StoreParams<'multi_query_search'>): WasmDenseResult[];

  /** VelesQL-style query returning multi-model results; `nodeId` is a {@link WasmPointId}. */
  query(...args: StoreParams<'query'>): Record<string, unknown>[];
}

// ---------------------------------------------------------------------------
// WasmModule — typed interface for the imported WASM package
// ---------------------------------------------------------------------------

/** The static side of the VectorStore class exported by velesdb-wasm. */
export interface WasmVectorStoreConstructor {
  /** Create a store holding its vectors in `mode` (`full`, `sq8`, `binary`, …). */
  new_with_mode(...args: AllParams<typeof BindingVectorStore.new_with_mode>): WasmVectorStore;

  /** Create a store with no vectors: the scratch store sparse vectors are validated in (`wasm-sparse.ts`). */
  new_metadata_only(
    ...args: AllParams<typeof BindingVectorStore.new_metadata_only>
  ): WasmVectorStore;

  /**
   * The binding's search-quality parser, run without a store: throws its
   * refusal of an unparseable preset. Optional because a binding released
   * before it has none (the SDK then falls back, see `wasm-search.ts`).
   */
  validate_search_quality?(quality: string): void;
}

/** What the backend reads from a `WasmDatabase.executeQuery` result. */
export interface WasmQueryResult {
  /** Statement class: `rows`, `mutation`, `deletion`, `ddl`, `train`, `admin`. */
  readonly kind: string;
  /** Rows returned (or affected). */
  readonly rowCount: number;
  /** The rows as a JSON array. */
  readonly rowsJson: string;
  /** Release the result's WASM memory. */
  free(): void;
}

/**
 * The VelesQL executor of a velesdb-wasm `WasmDatabase`, over the stores the
 * backend attaches to it (`wasm-velesql.ts`).
 */
export interface WasmVelesQLDatabase {
  /**
   * Register an existing store as collection `name`, sharing it rather than
   * copying it. Declared by hand, like {@link WasmVectorStore.sparse_remove}:
   * velesdb-wasm 8.2.0, the floor of the SDK's dependency range, has no such
   * method, and the backend then keeps its pre-executor behaviour.
   */
  attachStore?(name: string, store: WasmVectorStore): void;
  /** Drop collection `name` from the database (the attached store lives on in its handle). */
  delete_collection(...args: AllParams<BindingWasmDatabase['delete_collection']>): void;
  /** Execute one VelesQL statement; `params_json` is a JSON object or `null`. */
  executeQuery(sql: string, params_json: string | null): WasmQueryResult;
  /**
   * Run a `MATCH`; returns a JSON array of core-shaped rows
   * (`{ bindings, depth, projected, score? }`). `vector` and `threshold` are
   * REST `/match`'s. Declared by hand and optional, like
   * the edge methods below: bindings before it do not have it.
   */
  executeMatch?(sql: string, params_json: string | null, vector?: Float32Array, threshold?: number): string;
  /**
   * Add an edge given as JSON `{ id?, source, target, label, properties? }`
   * (ids as decimal strings or numbers); returns its id as a decimal string.
   */
  addEdge?(name: string, edge_json: string): string;
  /**
   * The edges of a collection, as a JSON array, optionally only one label.
   * Each edge's `id`, `source` and `target` is a decimal string.
   */
  getEdges?(name: string, label?: string): string;
  /**
   * Remove points with the edges they are an end of, as core's delete does;
   * returns how many points were there. Optional: bindings before it lack it.
   */
  removePoints?(name: string, ids: BigUint64Array): number;
  /** Remove an edge by its decimal-string id; `false` when it was absent. */
  removeEdge?(name: string, id: string): boolean;
  /** The number of edges in a collection. */
  edgeCount?(name: string): number;
  /** Release the database's WASM memory. */
  free(): void;
}

/**
 * What the backend reads from the binding's `VelesQL.parse()`: the statement
 * class, from the AST, so nothing is guessed from the statement's text.
 */
export interface WasmParsedQuery {
  /** A `SELECT` (not `MATCH`, DML, DDL, introspection or admin). */
  readonly isSelect: boolean;
  /**
   * Core's `is_aggregation_query`. Declared by hand: absent on bindings that
   * predate it, and the backend then leaves the statement to its old path.
   */
  readonly isAggregation?: boolean;
  /** The collection the statement reads, when it names one. */
  readonly collectionName?: string;
  /** Release the parse result's WASM memory. */
  free(): void;
}

/** The binding's VelesQL parser. */
export interface WasmVelesQLParser {
  /** Parse one statement; throws the parser's message on invalid VelesQL. */
  parse(query: string): WasmParsedQuery;
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

  /** WasmDatabase class constructor: the VelesQL executor (`wasm-velesql.ts`). */
  WasmDatabase?: new () => WasmVelesQLDatabase;

  /** The VelesQL parser the backend classifies statements with (`wasm-velesql.ts`). */
  VelesQL?: WasmVelesQLParser;

  /** The VelesQL contract version the binding implements (core's constant). */
  velesqlContractVersion?(): string;

  /** Fuse dense + sparse search results via Reciprocal Rank Fusion. */
  hybrid_search_fuse(...args: AllParams<typeof bindingHybridSearchFuse>): WasmSparseResult[];
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
  toPointId(id: string | number): bigint;
}
