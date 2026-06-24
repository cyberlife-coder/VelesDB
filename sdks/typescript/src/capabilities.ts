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

/**
 * Capability map surfaced by `VelesDB.capabilities()`.
 *
 * Every field is a `boolean` so that callers can write
 * `if (caps.feature) { ... }` without `?.` chaining. A missing
 * backend must still expose the full set of keys with `false`
 * values — we prefer explicit "unsupported" over "unknown".
 */
export interface CapabilityMap {
  /** Dense vector similarity search (`search`, `searchIds`, `searchBatch`). */
  vectorSearch: boolean;
  /** BM25 full-text search (`textSearch`). */
  textSearch: boolean;
  /** Combined dense + BM25 search (`hybridSearch`). */
  hybridSearch: boolean;
  /** Multi-query fusion search (`multiQuerySearch`). */
  multiQuerySearch: boolean;
  /** Sparse vector search (`sparse_vector` on the search body + hybrid sparse+dense). */
  sparseSearch: boolean;
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
  scroll: true,
  graphTraversal: true,
  secondaryIndexes: true,
  agentMemory: true,
  enableStreaming: true,
  streamInsert: true,
  pqTraining: true,
  velesqlQuery: true,
  collectionIntrospection: true,
  velesqlFusionStrategies: Object.freeze(['rrf', 'weighted', 'maximum', 'rsf', 'average']),
  velesqlMatchOrderBy: true,
  velesqlAlterCollection: true,
});

/**
 * Capability map for the WASM backend.
 *
 * The WASM build ships a focused subset: the dense / text / hybrid /
 * multi-query search paths. Everything that relies on persistent
 * on-disk structures (secondary indexes, graph, streaming, PQ
 * training, agent memory, introspection, sparse inverted index) is
 * explicitly `false`. See `backends/wasm-stubs.ts` for the exact set
 * of `wasmNotSupported()` throw sites.
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
  sparseSearch: false,
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
