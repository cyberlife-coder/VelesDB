/**
 * VelesDB Backend Capability Map
 *
 * Static, per-backend description of which features the currently
 * connected backend supports. Callers use this to gracefully degrade
 * their UI / plan / workflow when a feature is not available instead
 * of catching a runtime `NOT_SUPPORTED` error after the fact.
 *
 * The map is **frozen at backend construction** — it does not round-
 * trip to the server. The REST map assumes a `velesdb-server` of the
 * same minor version; if the server does not ship a given feature,
 * the individual call will still surface a typed `VelesError` at
 * runtime.
 *
 * @example
 * ```typescript
 * import { VelesDB } from '@wiscale/velesdb-sdk';
 *
 * const db = new VelesDB({ backend: 'wasm' });
 * await db.init();
 *
 * if (db.capabilities().graphTraversal) {
 *   await db.traverseGraph('kg', { source: 1, direction: 'out' });
 * } else {
 *   // fall back to REST or a pure in-memory traversal
 * }
 * ```
 *
 * @packageDocumentation
 */

import type {
  CollectionConfig,
  CollectionType,
  FusionParamName,
  IVelesDBBackend,
  QueryOptions,
  StorageMode,
} from './types';

// ---------------------------------------------------------------------------
// Value universes, derived from the SDK's own types
// ---------------------------------------------------------------------------

/** Keys `T` declares by name, index signatures left out. */
type DeclaredKeys<T> = keyof {
  [K in keyof T as string extends K ? never : number extends K ? never : K]: T[K];
};

/** Whether argument type `P` is an options object, or an array of them, that declares a `filter`. */
type DeclaresFilter<P> = P extends readonly (infer Item)[]
  ? DeclaresFilter<Item>
  : P extends ArrayBufferView
    ? false
    : P extends object
      ? 'filter' extends DeclaredKeys<P>
        ? true
        : false
      : false;

/** Whether any of the argument types `A` declares a `filter`. */
type AnyDeclaresFilter<A> = A extends readonly unknown[]
  ? true extends { [I in keyof A]: DeclaresFilter<NonNullable<A[I]>> }[number]
    ? true
    : false
  : false;

/** The {@link IVelesDBBackend} methods with an argument that declares a `filter`. */
type FilterTakingMethod = {
  [M in keyof IVelesDBBackend]-?: AnyDeclaresFilter<Parameters<IVelesDBBackend[M]>> extends true
    ? M
    : never;
}[keyof IVelesDBBackend];

/**
 * The entry points that accept a `filter`: every backend method with an
 * argument declaring one, derived from {@link IVelesDBBackend} so a new one
 * cannot be missed, plus `'sparseSearch'`, which is `search` called with a
 * `sparseVector`.
 */
export type FilteredSearchOperation = FilterTakingMethod | 'sparseSearch';

/**
 * `values`, frozen, and rejected by the compiler unless they are every
 * member of `U`: a runtime list cannot fall behind the type it spells out.
 */
function everyMemberOf<U extends string>() {
  return <const V extends readonly U[]>(
    values: V & ([U] extends [V[number]] ? unknown : never)
  ): Readonly<V> => Object.freeze(values);
}

/** Every {@link FilteredSearchOperation}. */
export const FILTERED_SEARCH_OPERATIONS = everyMemberOf<FilteredSearchOperation>()([
  'search',
  'sparseSearch',
  'searchBatch',
  'searchIds',
  'textSearch',
  'hybridSearch',
  'multiQuerySearch',
  'multiQuerySearchIds',
  'sparseSearchNamed',
  'scroll',
]);

/** Every `fusionParams` field. */
export const FUSION_PARAM_NAMES = everyMemberOf<FusionParamName>()([
  'k',
  'avgWeight',
  'maxWeight',
  'hitWeight',
  'denseWeight',
  'sparseWeight',
]);

/** Every storage mode. */
export const STORAGE_MODES = everyMemberOf<StorageMode>()(['full', 'sq8', 'binary', 'pq', 'rabitq']);

/** Every collection type. */
export const COLLECTION_TYPES = everyMemberOf<CollectionType>()(['vector', 'metadata_only', 'graph']);

/** Every `CollectionConfig` field. */
export const COLLECTION_CONFIG_FIELDS = everyMemberOf<keyof CollectionConfig>()([
  'dimension',
  'metric',
  'storageMode',
  'collectionType',
  'description',
  'hnsw',
  'pqRescoreOversampling',
  'deferredIndexing',
  'asyncIndexBuilder',
]);

/** Every `QueryOptions` field. */
export const QUERY_OPTION_NAMES = everyMemberOf<keyof QueryOptions>()(['timeoutMs', 'stream']);

/** `USING FUSION(strategy='...')` names the core SQL parser accepts. */
const VELESQL_FUSION_STRATEGIES: readonly string[] = Object.freeze([
  'rrf',
  'weighted',
  'maximum',
  'rsf',
  'average',
]);

// ---------------------------------------------------------------------------
// The map
// ---------------------------------------------------------------------------

/**
 * Capability map surfaced by `VelesDB.capabilities()`.
 *
 * Feature fields are `boolean` so that callers can write
 * `if (caps.feature) { ... }` without `?.` chaining; list fields name
 * the values the backend honours. A missing backend must still expose
 * the full set of keys with `false` or empty values — we prefer
 * explicit "unsupported" over "unknown".
 */
export interface CapabilityMap {
  /** Dense vector similarity search (`search`, `searchBatch`). */
  vectorSearch: boolean;
  /** BM25 full-text search (`textSearch`). */
  textSearch: boolean;
  /** Combined dense + BM25 search (`hybridSearch`). */
  hybridSearch: boolean;
  /** Multi-query fusion search (`multiQuerySearch`). */
  multiQuerySearch: boolean;
  /** Sparse vector search: `search` with a `sparseVector`, alone or fused with the dense query. */
  sparseSearch: boolean;
  /**
   * Entry points whose `filter` the backend applies. A backend that
   * cannot apply a filter to one of them refuses it with `NOT_SUPPORTED`
   * rather than returning rows the filter excludes.
   */
  filteredSearch: readonly FilteredSearchOperation[];
  /**
   * `multiQuerySearch` `fusionParams` fields the backend applies. A field the
   * chosen strategy reads but this list leaves out is refused with
   * `NOT_SUPPORTED`; a field the strategy never reads is ignored, as core
   * ignores it.
   */
  multiQueryFusionParams: readonly FusionParamName[];
  /** Named sparse indexes: `search({ sparseIndexName })` and `sparseSearchNamed`. */
  namedSparseIndexes: boolean;
  /** `search({ includeVectors: true })` returns each hit's vector. */
  includeVectors: boolean;
  /** ID-and-score searches that skip payloads (`searchIds`, `multiQuerySearchIds`). */
  idOnlySearch: boolean;
  /** `storageMode` values `createCollection` stores vectors in; another is refused with `NOT_SUPPORTED`. */
  storageModes: readonly StorageMode[];
  /** `collectionType` values `createCollection` creates; another is refused with `NOT_SUPPORTED`. */
  collectionTypes: readonly CollectionType[];
  /** `CollectionConfig` fields `createCollection` applies; another one, set, is refused with `NOT_SUPPORTED`. */
  collectionConfig: readonly (keyof CollectionConfig)[];
  /** `QueryOptions` fields `query` applies; another one, set, is refused with `NOT_SUPPORTED`. */
  queryOptions: readonly (keyof QueryOptions)[];
  /** Cursor-based scroll pagination over a collection (`scroll`). */
  scroll: boolean;
  /** Knowledge graph edge CRUD + traversal (`addEdge`, `traverseGraph`, `traverseParallel`, `getNodeDegree`). */
  graphTraversal: boolean;
  /** Secondary property indexes (`createIndex`, `listIndexes`, `hasIndex`, `dropIndex`). */
  secondaryIndexes: boolean;
  /** Agent Memory SDK (semantic, episodic, procedural). */
  agentMemory: boolean;
  /** Enable the bounded streaming-ingestion channel (`enableStreaming`). */
  enableStreaming: boolean;
  /** Streaming insert with backpressure (`streamInsert`). */
  streamInsert: boolean;
  /** Product quantization training (`trainPq`). */
  pqTraining: boolean;
  /** VelesQL multi-model query + EXPLAIN (`query`, `queryExplain`). */
  velesqlQuery: boolean;
  /** Collection introspection endpoints (`collectionSanity`, `getCollectionStats`, `analyzeCollection`, `getCollectionConfig`). */
  collectionIntrospection: boolean;
  /**
   * `USING FUSION(strategy='...')` strategies the backend's query path
   * accepts. Empty when `velesqlQuery` is `false`. The core SQL parser
   * accepts `rrf`, `weighted`, `maximum`, `rsf`, `average`.
   */
  velesqlFusionStrategies: readonly string[];
  /**
   * `MATCH (...) RETURN ... ORDER BY ... [LIMIT n]` is honored end-to-end
   * (sorted, then limited) by the backend's query path.
   */
  velesqlMatchOrderBy: boolean;
  /**
   * `ALTER COLLECTION <name> SET(...)` is supported via the typed
   * {@link VelesDB.alterCollection} / {@link VelesDB.setAutoReindex} helpers.
   */
  velesqlAlterCollection: boolean;
}

/** `CapabilityMap` keys whose value is a list. */
export type ListCapability = {
  [K in keyof CapabilityMap]: CapabilityMap[K] extends readonly unknown[] ? K : never;
}[keyof CapabilityMap];

/**
 * For each list capability, every value it could hold. A backend's list is
 * a subset; `tests/wasm-capabilities-conformance.test.ts` probes each value.
 */
export const CAPABILITY_LIST_UNIVERSES: Readonly<Record<ListCapability, readonly string[]>> =
  Object.freeze({
    filteredSearch: FILTERED_SEARCH_OPERATIONS,
    multiQueryFusionParams: FUSION_PARAM_NAMES,
    storageModes: STORAGE_MODES,
    collectionTypes: COLLECTION_TYPES,
    collectionConfig: COLLECTION_CONFIG_FIELDS,
    queryOptions: QUERY_OPTION_NAMES,
    velesqlFusionStrategies: VELESQL_FUSION_STRATEGIES,
  });

/**
 * Capability map for the REST backend — assumes a server of the
 * same minor version as the SDK. Every feature the SDK wraps is
 * advertised; individual endpoints may still surface a typed
 * `VelesError` at runtime if the server was built with a feature
 * flag disabled.
 */
export const REST_CAPABILITIES: Readonly<CapabilityMap> = Object.freeze({
  vectorSearch: true,
  textSearch: true,
  hybridSearch: true,
  multiQuerySearch: true,
  sparseSearch: true,
  // velesdb-server refuses a filter on `/search/multi/ids` with a 400. The
  // SDK sends it on so that refusal reaches the caller, but it is not applied.
  filteredSearch: Object.freeze<FilteredSearchOperation[]>(
    FILTERED_SEARCH_OPERATIONS.filter((operation) => operation !== 'multiQuerySearchIds')
  ),
  multiQueryFusionParams: FUSION_PARAM_NAMES,
  namedSparseIndexes: true,
  includeVectors: true,
  idOnlySearch: true,
  storageModes: STORAGE_MODES,
  collectionTypes: COLLECTION_TYPES,
  collectionConfig: COLLECTION_CONFIG_FIELDS,
  queryOptions: QUERY_OPTION_NAMES,
  scroll: true,
  graphTraversal: true,
  secondaryIndexes: true,
  agentMemory: true,
  enableStreaming: true,
  streamInsert: true,
  pqTraining: true,
  velesqlQuery: true,
  collectionIntrospection: true,
  velesqlFusionStrategies: VELESQL_FUSION_STRATEGIES,
  velesqlMatchOrderBy: true,
  velesqlAlterCollection: true,
});

/**
 * Capability map for the WASM backend.
 *
 * The WASM build ships a focused subset: dense, sparse, text, hybrid and
 * multi-query search over an in-memory store. Everything that relies on
 * persistent on-disk structures (secondary indexes, graph, streaming,
 * PQ training, agent memory, introspection) is explicitly `false`;
 * `backends/wasm-stubs.ts` holds those throw sites.
 *
 * This table is the one place WASM support is stated. The backend reads it
 * before it uses an option (`backends/wasm-capability-guards.ts`) and
 * refuses with `NOT_SUPPORTED` what it withholds, rather than dropping it:
 * a `filter` on an operation `filteredSearch` does not list; a
 * `fusionParams`, `CollectionConfig` or `QueryOptions` field its list leaves
 * out; a storage mode or collection type it cannot create. Search `quality`
 * is accepted: WASM scans every stored vector, which meets any preset's
 * recall. `tests/wasm-capabilities-conformance.test.ts` probes every key
 * against the backend, so the two cannot drift apart unnoticed; the map
 * once said `sparseSearch: false` while sparse search ran (#2095).
 *
 * `velesqlQuery` is `false`: `query()` only executes pure top-k NEAR
 * statements (`SELECT * FROM <collection> WHERE vector NEAR $param
 * [LIMIT n]`) and throws `NOT_SUPPORTED` for any other VelesQL clause
 * (WHERE predicates, JOIN, GROUP BY, MATCH, set operations, FUSION),
 * so full VelesQL is not advertised.
 */
export const WASM_CAPABILITIES: Readonly<CapabilityMap> = Object.freeze({
  vectorSearch: true,
  textSearch: true,
  hybridSearch: true,
  multiQuerySearch: true,
  sparseSearch: true,
  // Dense search filters through the binding's `search_with_filter`, and
  // `searchBatch` runs dense searches. The binding's `sparse_search`,
  // `text_search`, `hybrid_search` and `multi_query_search` take no filter,
  // and the id-only, named-sparse and scroll entry points are not implemented.
  filteredSearch: Object.freeze<FilteredSearchOperation[]>(['search', 'searchBatch']),
  // velesdb-wasm's `multi_query_search` takes `rrf_k` and the weighted
  // `[avg, max, hit]` triple. Its `relative_score` averages the query
  // branches with equal weight, so it has no use for dense/sparse weights.
  multiQueryFusionParams: Object.freeze<FusionParamName[]>([
    'k',
    'avgWeight',
    'maxWeight',
    'hitWeight',
  ]),
  namedSparseIndexes: false,
  includeVectors: false,
  idOnlySearch: false,
  // velesdb-wasm stores `pq` and `rabitq` as SQ8 (the browser has no
  // codebook training), so only the modes it implements are listed.
  storageModes: Object.freeze<StorageMode[]>(['full', 'sq8', 'binary']),
  collectionTypes: Object.freeze<CollectionType[]>(['vector']),
  // No HNSW graph, no PQ rescoring, no deferred or async indexing: WASM
  // scans every stored vector, so their settings have nowhere to go.
  collectionConfig: Object.freeze<(keyof CollectionConfig)[]>([
    'dimension',
    'metric',
    'storageMode',
    'collectionType',
    'description',
  ]),
  // `query()` runs in process and answers at once: no timeout, no stream.
  queryOptions: Object.freeze<(keyof QueryOptions)[]>([]),
  scroll: false,
  graphTraversal: false,
  secondaryIndexes: false,
  agentMemory: false,
  enableStreaming: false,
  streamInsert: false,
  pqTraining: false,
  velesqlQuery: false,
  collectionIntrospection: false,
  // `velesqlQuery` is false on this backend, so the VelesQL sub-capabilities
  // are all unavailable.
  velesqlFusionStrategies: Object.freeze([]),
  velesqlMatchOrderBy: false,
  velesqlAlterCollection: false,
});
