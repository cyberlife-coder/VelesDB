/**
 * @module tauri-plugin-velesdb
 * 
 * TypeScript bindings for the VelesDB Tauri plugin.
 * Provides type-safe access to vector database operations in desktop apps.
 * 
 * @example
 * ```typescript
 * import { createCollection, search, upsert } from 'tauri-plugin-velesdb';
 * 
 * // Create a collection
 * await createCollection({ name: 'docs', dimension: 768, metric: 'cosine' });
 * 
 * // Insert vectors
 * await upsert({
 *   collection: 'docs',
 *   points: [{ id: 1, vector: [...], payload: { title: 'Doc' } }]
 * });
 * 
 * // Search
 * const results = await search({ collection: 'docs', vector: [...], topK: 10 });
 * ```
 */

import { invoke } from '@tauri-apps/api/core';

// ============================================================================
// Types
// ============================================================================

/** Distance metric for vector similarity. */
export type DistanceMetric = 'cosine' | 'euclidean' | 'dot' | 'hamming' | 'jaccard';

/** Storage mode for vector compression. */
export type StorageMode = 'full' | 'sq8' | 'binary' | 'pq' | 'rabitq';


/** Request to create a new vector collection. */
export interface CreateCollectionRequest {
  /** Collection name (unique identifier). */
  name: string;
  /** Vector dimension (e.g., 768 for BERT, 1536 for GPT). */
  dimension: number;
  /** Distance metric for similarity calculations. Default: 'cosine'. */
  metric?: DistanceMetric;
  /** Storage mode for vector compression. Default: 'full'. */
  storageMode?: StorageMode;
  /** HNSW M parameter (max connections per node). Auto-tuned if omitted. */
  hnswM?: number;
  /** HNSW ef_construction parameter. Auto-tuned if omitted. */
  hnswEfConstruction?: number;
  /** HNSW alpha for neighbor diversification. Default: 1.2. */
  hnswAlpha?: number;
  /** HNSW initial max elements capacity. Auto-tuned if omitted. */
  hnswMaxElements?: number;
  /** PQ rescore oversampling factor. Default: 4. */
  pqRescoreOversampling?: number;
}

/** Request to create a graph collection with optional schema. */
export interface CreateGraphCollectionRequest {
  /** Collection name (unique identifier). */
  name: string;
  /** Optional vector dimension for node embeddings. */
  dimension?: number;
  /** Distance metric (when dimension is set). Default: 'cosine'. */
  metric?: DistanceMetric;
  /** Graph schema definition. Pass { schemaless: true } for schemaless mode. */
  graphSchema?: Record<string, unknown>;
}

/** Request to create a metadata-only collection. */
export interface CreateMetadataCollectionRequest {
  /** Collection name (unique identifier). */
  name: string;
}

/** A metadata-only point to insert (no vector). */
export interface MetadataPointInput {
  /** Unique point identifier. */
  id: string | number;
  /** Payload with metadata. */
  payload: Record<string, unknown>;
}

/** Request to upsert metadata-only points. */
export interface UpsertMetadataRequest {
  /** Target collection name. */
  collection: string;
  /** Metadata points to upsert. */
  points: MetadataPointInput[];
}

/** Collection information. */
export interface CollectionInfo {
  /** Collection name. */
  name: string;
  /** Vector dimension. */
  dimension: number;
  /** Distance metric. */
  metric: string;
  /** Number of vectors in the collection. */
  count: number;
  /** Storage mode (full, sq8, binary, pq, rabitq, graph, metadata_only). */
  storageMode: string;
}

/** A point (vector with metadata) to insert. */
export interface PointInput {
  /** Unique point identifier. */
  id: string | number;
  /** Vector data (must match collection dimension). */
  vector: number[];
  /** Optional JSON payload with metadata. */
  payload?: Record<string, unknown>;
}

/** Request to upsert points. */
export interface UpsertRequest {
  /** Target collection name. */
  collection: string;
  /** Points to insert or update. */
  points: PointInput[];
}

/** Request for vector similarity search. */
export interface SearchRequest {
  /** Target collection name. */
  collection: string;
  /** Query vector. */
  vector: number[];
  /** Number of results to return. Default: 10. */
  topK?: number;
  /** Optional metadata filter. */
  filter?: Record<string, unknown>;
  /** Search quality: 'fast', 'balanced', 'accurate', 'perfect', 'auto', 'custom:<ef>' or 'adaptive:<min_ef>:<max_ef>', each ef in 16-4096; any other value is refused. */
  quality?: string;
}

/** Request for BM25 text search. */
export interface TextSearchRequest {
  /** Target collection name. */
  collection: string;
  /** Text query for BM25 search. */
  query: string;
  /** Number of results to return. Default: 10. */
  topK?: number;
  /** Optional metadata filter. */
  filter?: Record<string, unknown>;
}

/** Request for hybrid (vector + text) search. */
export interface HybridSearchRequest {
  /** Target collection name. */
  collection: string;
  /** Query vector for similarity search. */
  vector: number[];
  /** Text query for BM25 search. */
  query: string;
  /** Number of results to return. Default: 10. */
  topK?: number;
  /** Weight for vector results (0.0-1.0). Default: 0.5. */
  vectorWeight?: number;
  /** Optional metadata filter. */
  filter?: Record<string, unknown>;
}

/** Request for VelesQL query. */
export interface QueryRequest {
  /** VelesQL query string. */
  query: string;
  /** Query parameters (for parameterized queries). */
  params?: Record<string, unknown>;
  /**
   * Collection a top-level `MATCH` runs on — required for it, since its
   * pattern names none. Sent to the engine as `params._collection` unless
   * that key is set, which then takes precedence (the REST server's `/query`
   * rule); every other statement ignores it. A `_collection` that is not a
   * non-empty string is refused with `VELES-038`.
   */
  collection?: string;
}

/** Request to get points by IDs. */
export interface GetPointsRequest {
  /** Target collection name. */
  collection: string;
  /** Point IDs to retrieve. */
  ids: Array<string | number>;
}

/** Request to delete points by IDs. */
export interface DeletePointsRequest {
  /** Target collection name. */
  collection: string;
  /** Point IDs to delete. */
  ids: Array<string | number>;
}

/** Individual search request within a batch. */
export interface IndividualSearchRequest {
  /** Query vector. */
  vector: number[];
  /** Number of results to return. Default: 10. */
  topK?: number;
  /** Optional metadata filter. */
  filter?: Record<string, unknown>;
  /** Search quality: 'fast', 'balanced', 'accurate', 'perfect', 'auto', 'custom:<ef>' or 'adaptive:<min_ef>:<max_ef>', each ef in 16-4096; any other value is refused. */
  quality?: string;
}

/** Request for batch search. */
export interface BatchSearchRequest {
  /** Target collection name. */
  collection: string;
  /** List of search queries. */
  searches: IndividualSearchRequest[];
}

/** Fusion strategy for multi-query search. */
export type FusionStrategy =
  | 'rrf'
  | 'weighted_rrf'
  | 'average'
  | 'maximum'
  | 'weighted'
  | 'relative_score';

/** Fusion parameters for multi-query search. */
export interface FusionParams {
  /** RRF k parameter (default: 60). */
  k?: number;
  /** Weighted fusion: average weight (default: 0.6). */
  avgWeight?: number;
  /** Weighted fusion: max weight (default: 0.3). */
  maxWeight?: number;
  /** Weighted fusion: hit weight (default: 0.1). */
  hitWeight?: number;
  /** Relative score fusion: dense branch weight (default: 0.5). */
  denseWeight?: number;
  /** Relative score fusion: sparse branch weight (default: 0.5). */
  sparseWeight?: number;
  /** Weighted RRF: one weight per query vector, in vector order. Required under `weighted_rrf`. */
  branchWeights?: number[];
}

/** Request for multi-query fusion search. */
export interface MultiQuerySearchRequest {
  /** Target collection name. */
  collection: string;
  /** List of query vectors. */
  vectors: number[][];
  /** Number of results to return. Default: 10. */
  topK?: number;
  /** Fusion strategy: 'rrf', 'weighted_rrf', 'average', 'maximum', 'weighted', 'relative_score'. Default: 'rrf'. */
  fusion?: FusionStrategy;
  /** Fusion parameters. */
  fusionParams?: FusionParams;
  /** Optional metadata filter. */
  filter?: Record<string, unknown>;
}

/** Point output for get operations. */
export interface PointOutput {
  /** Point ID, as a decimal string (exact past 2^53, as REST sends it). */
  id: string;
  /** Vector data. */
  vector: number[];
  /** Point payload, `null` for a point stored without one. */
  payload: Record<string, unknown> | null;
}

/** Search result item. */
export interface SearchResult {
  /** Point ID, as a decimal string (exact past 2^53, as REST sends it). */
  id: string;
  /** Similarity/distance score. */
  score: number;
  /** Point payload, `null` for a point stored without one. */
  payload: Record<string, unknown> | null;
}

/** Response from search operations. */
export interface SearchResponse {
  /** Search results ordered by relevance. */
  results: SearchResult[];
  /** Query execution time in milliseconds. */
  timingMs: number;
}

/** A single ID + score pair returned by {@link searchIds}. */
export interface IdScoreOutput {
  /** Matched point ID, as a decimal string. */
  id: string;
  /** Relevance score. */
  score: number;
}

/**
 * Runtime query guardrail limits, read by {@link getGuardrails} and replaced
 * as a whole by {@link updateGuardrails}.
 */
export interface GuardrailLimits {
  /** Maximum graph traversal depth. */
  maxDepth: number;
  /** Maximum intermediate cardinality. */
  maxCardinality: number;
  /** Memory limit per query in bytes. */
  memoryLimitBytes: number;
  /** Query timeout in milliseconds (0 disables the timeout). */
  timeoutMs: number;
  /** Rate limit: max queries per second per client. `0` is refused. */
  rateLimitQps: number;
  /** Circuit breaker: failure threshold before tripping. `0` is refused. */
  circuitFailureThreshold: number;
  /** Circuit breaker: recovery time in seconds. */
  circuitRecoverySeconds: number;
}

/**
 * Hybrid result from VelesQL queries (vector + graph + column data).
 *
 * Every field is always present: one that does not apply is `null`.
 */
export interface HybridResult {
  /** Node/point ID, as a decimal string (`"0"` for an aggregate's row). */
  nodeId: string;
  /**
   * Vector similarity score: the `vector_score` component core tagged the row
   * with (every row a vector scored carries one), `null` on a row no vector
   * scored (a metadata, text-only or aggregate row).
   */
  vectorScore: number | null;
  /** Graph traversal score; the `query` command sets none, so `null`. */
  graphScore: number | null;
  /** Fused score combining vector and graph components. */
  fusedScore: number;
  /** Payload/bindings from the query, `null` when there are none. */
  bindings: Record<string, unknown> | null;
  /**
   * Result of an aggregate `SELECT ... FROM` (scalar aggregates or a
   * `GROUP BY` without `NEAR`), on the single row such a query returns: one
   * object without `GROUP BY` (`{ count: 3 }`), an array of one object per
   * group with it. `null` on every other row.
   */
  columnData: Record<string, unknown> | Array<Record<string, unknown>> | null;
}

/** Response from VelesQL query operations. */
export interface QueryResponse {
  /** Query results. */
  results: HybridResult[];
  /** Query execution time in milliseconds. */
  timingMs: number;
}

/** Error returned by plugin commands. */
export interface CommandError {
  /** Human-readable error message. */
  message: string;
  /**
   * Core taxonomy code (`VELES-###`) for programmatic handling, or
   * `TASK_JOIN` when the plugin's blocking task could not be joined.
   */
  code: string;
  /** Whether retrying can succeed, as core answers for the same condition. */
  recoverable: boolean;
}

// ============================================================================
// Collection Management
// ============================================================================

/**
 * Creates a new vector collection.
 * 
 * @param request - Collection configuration
 * @returns Collection info
 * @throws {CommandError} If collection already exists or parameters are invalid
 * 
 * @example
 * ```typescript
 * const info = await createCollection({
 *   name: 'documents',
 *   dimension: 768,
 *   metric: 'cosine'
 * });
 * console.log(`Created collection with ${info.count} vectors`);
 * ```
 */
export function createCollection(request: CreateCollectionRequest): Promise<CollectionInfo> {
  return invoke<CollectionInfo>('plugin:velesdb|create_collection', { request });
}

/**
 * Creates a metadata-only collection (no vectors, just payloads).
 * 
 * Useful for storing reference data that can be joined with vector collections.
 * 
 * @param request - Collection configuration
 * @returns Collection info
 * @throws {CommandError} If collection already exists
 * 
 * @example
 * ```typescript
 * const info = await createMetadataCollection({ name: 'products' });
 * console.log(`Created metadata collection: ${info.name}`);
 * ```
 */
export function createMetadataCollection(request: CreateMetadataCollectionRequest): Promise<CollectionInfo> {
  return invoke<CollectionInfo>('plugin:velesdb|create_metadata_collection', { request });
}

/**
 * Deletes a collection and all its data.
 * 
 * @param name - Collection name to delete
 * @throws {CommandError} If collection doesn't exist
 * 
 * @example
 * ```typescript
 * await deleteCollection('documents');
 * ```
 */
export async function deleteCollection(name: string): Promise<void> {
  await invoke('plugin:velesdb|delete_collection', { name });
}

/**
 * Lists all collections in the database.
 * 
 * @returns Array of collection info objects
 * 
 * @example
 * ```typescript
 * const collections = await listCollections();
 * collections.forEach(c => console.log(`${c.name}: ${c.count} vectors`));
 * ```
 */
export function listCollections(): Promise<CollectionInfo[]> {
  return invoke<CollectionInfo[]>('plugin:velesdb|list_collections');
}

/**
 * Gets information about a specific collection.
 * 
 * @param name - Collection name
 * @returns Collection info
 * @throws {CommandError} If collection doesn't exist
 * 
 * @example
 * ```typescript
 * const info = await getCollection('documents');
 * console.log(`Dimension: ${info.dimension}, Count: ${info.count}`);
 * ```
 */
export function getCollection(name: string): Promise<CollectionInfo> {
  return invoke<CollectionInfo>('plugin:velesdb|get_collection', { name });
}

// ============================================================================
// Vector Operations
// ============================================================================

/**
 * Inserts or updates vectors in a collection.
 * 
 * @param request - Upsert request with collection name and points
 * @returns Number of points upserted
 * @throws {CommandError} If collection doesn't exist or vectors are invalid
 * 
 * @example
 * ```typescript
 * const count = await upsert({
 *   collection: 'documents',
 *   points: [
 *     { id: 1, vector: [0.1, 0.2, ...], payload: { title: 'Doc 1' } },
 *     { id: 2, vector: [0.3, 0.4, ...], payload: { title: 'Doc 2' } }
 *   ]
 * });
 * console.log(`Upserted ${count} points`);
 * ```
 */
export function upsert(request: UpsertRequest): Promise<number> {
  return invoke<number>('plugin:velesdb|upsert', { request });
}

/**
 * Inserts or updates metadata-only points in a collection.
 * 
 * Use this for collections created with createMetadataCollection().
 * 
 * @param request - Upsert request with collection name and metadata points
 * @returns Number of points upserted
 * @throws {CommandError} If collection doesn't exist or is not metadata-only
 * 
 * @example
 * ```typescript
 * const count = await upsertMetadata({
 *   collection: 'products',
 *   points: [
 *     { id: 1, payload: { name: 'Widget', price: 99 } },
 *     { id: 2, payload: { name: 'Gadget', price: 149 } }
 *   ]
 * });
 * console.log(`Upserted ${count} metadata points`);
 * ```
 */
export function upsertMetadata(request: UpsertMetadataRequest): Promise<number> {
  return invoke<number>('plugin:velesdb|upsert_metadata', { request });
}

// ============================================================================
// Search Operations
// ============================================================================

/**
 * Performs vector similarity search.
 * 
 * @param request - Search request with query vector
 * @returns Search response with results and timing
 * @throws {CommandError} If collection doesn't exist or vector dimension mismatches
 * 
 * @example
 * ```typescript
 * const response = await search({
 *   collection: 'documents',
 *   vector: queryEmbedding,
 *   topK: 5
 * });
 * response.results.forEach(r => {
 *   console.log(`ID: ${r.id}, Score: ${r.score}, Title: ${r.payload?.title}`);
 * });
 * ```
 */
export function search(request: SearchRequest): Promise<SearchResponse> {
  return invoke<SearchResponse>('plugin:velesdb|search', { request });
}

/**
 * Performs vector similarity search returning only IDs and scores (no payloads).
 *
 * Takes the same request as {@link search}. Without a `filter` or `quality`
 * the engine skips payload hydration entirely; with either, the full search
 * runs and its results are projected to IDs + scores; a `filter` and a
 * `quality` apply together.
 *
 * @param request - Search request with query vector
 * @returns ID + score pairs ordered by relevance
 * @throws {CommandError} If collection doesn't exist or vector dimension mismatches
 *
 * @example
 * ```typescript
 * const hits = await searchIds({ collection: 'documents', vector: queryEmbedding, topK: 100 });
 * const ids = hits.map(h => h.id);
 * ```
 */
export function searchIds(request: SearchRequest): Promise<IdScoreOutput[]> {
  return invoke<IdScoreOutput[]>('plugin:velesdb|search_ids', { request });
}

/**
 * Performs BM25 full-text search across payloads.
 * 
 * @param request - Text search request
 * @returns Search response with results and timing
 * @throws {CommandError} If collection doesn't exist
 * 
 * @example
 * ```typescript
 * const response = await textSearch({
 *   collection: 'documents',
 *   query: 'machine learning tutorial',
 *   topK: 10
 * });
 * ```
 */
export function textSearch(request: TextSearchRequest): Promise<SearchResponse> {
  return invoke<SearchResponse>('plugin:velesdb|text_search', { request });
}

/**
 * Performs hybrid search combining vector similarity and BM25 text relevance.
 * Uses Reciprocal Rank Fusion (RRF) to merge results.
 * 
 * @param request - Hybrid search request
 * @returns Search response with fused results and timing
 * @throws {CommandError} If collection doesn't exist or parameters are invalid
 * 
 * @example
 * ```typescript
 * const response = await hybridSearch({
 *   collection: 'documents',
 *   vector: queryEmbedding,
 *   query: 'neural networks',
 *   topK: 10,
 *   vectorWeight: 0.7  // 70% vector, 30% text
 * });
 * ```
 */
export function hybridSearch(request: HybridSearchRequest): Promise<SearchResponse> {
  return invoke<SearchResponse>('plugin:velesdb|hybrid_search', { request });
}

/**
 * Executes a VelesQL query (SELECT, MATCH, DDL, DML).
 *
 * @param request - Query request with VelesQL string
 * @returns Query response with hybrid results and timing
 * @throws {CommandError} If query syntax is invalid or collection doesn't exist
 *
 * @example
 * ```typescript
 * // SELECT query
 * const response = await query({
 *   query: "SELECT * FROM documents WHERE vector NEAR $v LIMIT 10",
 *   params: { v: queryEmbedding }
 * });
 *
 * // MATCH (graph) query: the pattern names no collection, so pass it
 * const response = await query({
 *   query: "MATCH (d:Doc)-[:AUTHORED_BY]->(a:Person) RETURN a.name",
 *   collection: 'knowledge'
 * });
 * ```
 */
export function query(request: QueryRequest): Promise<QueryResponse> {
  return invoke<QueryResponse>('plugin:velesdb|query', { request });
}

/**
 * Executes a VelesQL statement that changes nothing — SELECT, MATCH,
 * SELECT EDGES, SHOW, DESCRIBE or EXPLAIN — and refuses any other one
 * (VELES-010) before it runs.
 *
 * The VelesQL entry point the `velesdb:read-only` permission set grants;
 * `query` also runs DDL and DML.
 *
 * @param request - Query request with VelesQL string
 * @returns Query response with hybrid results and timing
 * @throws {CommandError} If the statement writes, its syntax is invalid or
 *   the collection doesn't exist
 */
export function readQuery(request: QueryRequest): Promise<QueryResponse> {
  return invoke<QueryResponse>('plugin:velesdb|read_query', { request });
}

/**
 * Retrieves points by their IDs.
 * 
 * @param request - Get points request with collection name and IDs
 * @returns Array of points (null for IDs not found)
 * @throws {CommandError} If collection doesn't exist
 * 
 * @example
 * ```typescript
 * const points = await getPoints({
 *   collection: 'documents',
 *   ids: [1, 2, 3]
 * });
 * points.forEach((p, i) => {
 *   if (p) console.log(`Point ${p.id}: ${p.payload?.title}`);
 *   else console.log(`Point at index ${i} not found`);
 * });
 * ```
 */
export function getPoints(request: GetPointsRequest): Promise<Array<PointOutput | null>> {
  return invoke<Array<PointOutput | null>>('plugin:velesdb|get_points', { request });
}

/**
 * Deletes points by their IDs.
 * 
 * @param request - Delete points request with collection name and IDs
 * @throws {CommandError} If collection doesn't exist
 * 
 * @example
 * ```typescript
 * await deletePoints({
 *   collection: 'documents',
 *   ids: [1, 2, 3]
 * });
 * ```
 */
export async function deletePoints(request: DeletePointsRequest): Promise<void> {
  await invoke('plugin:velesdb|delete_points', { request });
}

/**
 * Performs batch vector similarity search.
 * 
 * @param request - Batch search request with multiple queries
 * @returns Array of search responses, one per query
 * @throws {CommandError} If collection doesn't exist
 * 
 * @example
 * ```typescript
 * const responses = await batchSearch({
 *   collection: 'documents',
 *   searches: [
 *     { vector: embedding1, topK: 5 },
 *     { vector: embedding2, topK: 10, filter: { condition: { type: 'eq', field: 'category', value: 'tech' } } }
 *   ]
 * });
 * responses.forEach((resp, i) => {
 *   console.log(`Query ${i}: ${resp.results.length} results in ${resp.timingMs}ms`);
 * });
 * ```
 */
export function batchSearch(request: BatchSearchRequest): Promise<SearchResponse[]> {
  return invoke<SearchResponse[]>('plugin:velesdb|batch_search', { request });
}

/**
 * Performs multi-query fusion search combining results from multiple query vectors.
 * 
 * Ideal for RAG pipelines using Multiple Query Generation (MQG).
 * 
 * @param request - Multi-query search request
 * @returns Fused search response
 * @throws {CommandError} If collection doesn't exist or parameters are invalid
 * 
 * @example
 * ```typescript
 * // RRF fusion (default)
 * const response = await multiQuerySearch({
 *   collection: 'documents',
 *   vectors: [embedding1, embedding2, embedding3],
 *   topK: 10,
 *   fusion: 'rrf',
 *   fusionParams: { k: 60 }
 * });
 * 
 * // Weighted fusion
 * const response = await multiQuerySearch({
 *   collection: 'documents',
 *   vectors: [embedding1, embedding2],
 *   topK: 10,
 *   fusion: 'weighted',
 *   fusionParams: { avgWeight: 0.6, maxWeight: 0.3, hitWeight: 0.1 }
 * });
 * ```
 */
export function multiQuerySearch(request: MultiQuerySearchRequest): Promise<SearchResponse> {
  return invoke<SearchResponse>('plugin:velesdb|multi_query_search', { request });
}

/**
 * Checks if a collection is empty.
 * 
 * @param name - Collection name
 * @returns true if collection has no points, false otherwise
 * @throws {CommandError} If collection doesn't exist
 * 
 * @example
 * ```typescript
 * const empty = await isEmpty('documents');
 * if (empty) console.log('Collection is empty');
 * ```
 */
export function isEmpty(name: string): Promise<boolean> {
  return invoke<boolean>('plugin:velesdb|is_empty', { name });
}

/**
 * Flushes pending changes to disk.
 * 
 * @param name - Collection name
 * @throws {CommandError} If collection doesn't exist
 * 
 * @example
 * ```typescript
 * await flush('documents');
 * console.log('Changes persisted to disk');
 * ```
 */
export async function flush(name: string): Promise<void> {
  await invoke('plugin:velesdb|flush', { name });
}

/**
 * Compacts on-disk storage for a collection, reclaiming space from deleted vectors.
 *
 * @param name - Collection name
 * @returns Number of bytes reclaimed
 * @throws {CommandError} If collection doesn't exist
 *
 * @example
 * ```typescript
 * const freed = await compactStorage('documents');
 * console.log(`Reclaimed ${freed} bytes`);
 * ```
 */
export function compactStorage(name: string): Promise<number> {
  return invoke<number>('plugin:velesdb|compact_storage', { name });
}

/**
 * Replaces the query guardrail limits of every collection in the database.
 *
 * This is a full replacement: every field of `limits` is applied once it
 * passes validation. A `rateLimitQps` or `circuitFailureThreshold` of `0` is
 * refused with `VELES-009` and nothing changes.
 *
 * @param limits - The complete set of limits to apply
 * @throws {CommandError} If the limits fail validation
 *
 * @example
 * ```typescript
 * const limits = await getGuardrails('documents');
 * await updateGuardrails({ ...limits, timeoutMs: 5000 });
 * ```
 */
export async function updateGuardrails(limits: GuardrailLimits): Promise<void> {
  await invoke('plugin:velesdb|update_guardrails', { limits });
}

/**
 * Returns the current query guardrail limits for a collection.
 *
 * @param name - Collection name
 * @returns The collection's guardrail limits
 * @throws {CommandError} If collection doesn't exist
 */
export function getGuardrails(name: string): Promise<GuardrailLimits> {
  return invoke<GuardrailLimits>('plugin:velesdb|get_guardrails', { name });
}

// ============================================================================
// Knowledge Graph API
// ============================================================================

/** Request to add an edge to a graph collection. */
export interface AddEdgeRequest {
  collection: string;
  /**
   * Edge id. Send the decimal string an edge read answers (`EdgeOutput.id`);
   * a number is accepted up to `Number.MAX_SAFE_INTEGER`.
   */
  id: string | number;
  /** Node id: a decimal string or a number (a number is exact up to `Number.MAX_SAFE_INTEGER`). */
  source: string | number;
  /** Node id: a decimal string or a number (a number is exact up to `Number.MAX_SAFE_INTEGER`). */
  target: string | number;
  label: string;
  properties?: Record<string, unknown>;
}

/** A single edge inside an {@link AddEdgesBatchRequest}. */
export interface BatchEdgeInput {
  /**
   * Edge ID. Send the decimal string an edge read answers (`EdgeOutput.id`);
   * a number is accepted up to `Number.MAX_SAFE_INTEGER`.
   */
  id: string | number;
  /** Source node ID: a decimal string or a number. */
  source: string | number;
  /** Target node ID: a decimal string or a number. */
  target: string | number;
  /** Edge label (relationship type). */
  label: string;
  /** Optional edge properties. */
  properties?: Record<string, unknown>;
}

/** Request to add several edges to a graph collection in one operation. */
export interface AddEdgesBatchRequest {
  /** Collection name. */
  collection: string;
  /** Edges to insert. */
  edges: BatchEdgeInput[];
}

/** Request to retrieve edges from a graph collection. */
export interface GetEdgesRequest {
  collection: string;
  label?: string;
  source?: string | number;
  target?: string | number;
}

/** Request to traverse the knowledge graph. */
export interface TraverseGraphRequest {
  collection: string;
  /** Node id: a decimal string or a number (a number is exact up to `Number.MAX_SAFE_INTEGER`). */
  source: string | number;
  /** Maximum traversal depth. Default: 3. */
  maxDepth?: number;
  relTypes?: string[];
  /** Maximum results to return. Default: 100. */
  limit?: number;
  /** Traversal algorithm. Default: 'bfs'. */
  algorithm?: 'bfs' | 'dfs';
}

/** Request to get the degree of a node. */
export interface GetNodeDegreeRequest {
  collection: string;
  /** Node id: a decimal string or a number (a number is exact up to `Number.MAX_SAFE_INTEGER`). */
  nodeId: string | number;
}

/** A single edge in the knowledge graph. */
export interface EdgeOutput {
  /**
   * Edge id as a decimal string: a derived edge id is hashed and routinely
   * above 2^53, which a JavaScript number rounds.
   */
  id: string;
  /** Source node id, as a decimal string (as REST's `EdgeResponse`). */
  source: string;
  /** Target node id, as a decimal string. */
  target: string;
  label: string;
  properties: Record<string, unknown>;
}

/** A single traversal result node. */
export interface TraversalOutput {
  /** Node reached, as a decimal string (as REST's `TraversalResultItem`). */
  targetId: string;
  depth: number;
  /** Ids of the edges walked, in order, as decimal strings (see `EdgeOutput.id`). */
  path: string[];
}

/** In/out degree of a graph node. */
export interface NodeDegreeOutput {
  /** Node id, as a decimal string. */
  nodeId: string;
  inDegree: number;
  outDegree: number;
}

/**
 * Adds an edge to a knowledge graph collection.
 *
 * @param request - Edge to add (collection, id, source, target, label, optional properties)
 * @throws {CommandError} If the collection doesn't exist
 *
 * @example
 * ```typescript
 * await addEdge({
 *   collection: 'social', id: 1, source: 100, target: 200,
 *   label: 'FOLLOWS', properties: { since: '2024-01-01' }
 * });
 * ```
 */
export async function addEdge(request: AddEdgeRequest): Promise<void> {
  await invoke('plugin:velesdb|add_edge', { request });
}

/**
 * Adds several edges to a knowledge graph collection in one batched operation.
 *
 * @param request - Collection name and the edges to insert
 * @returns Number of edges inserted
 * @throws {CommandError} If the collection is not a graph collection or an edge is invalid
 *
 * @example
 * ```typescript
 * const added = await addEdgesBatch({
 *   collection: 'social',
 *   edges: [
 *     { id: 1, source: 100, target: 200, label: 'FOLLOWS' },
 *     { id: 2, source: 200, target: 300, label: 'FOLLOWS', properties: { since: '2024-01-01' } },
 *   ],
 * });
 * ```
 */
export function addEdgesBatch(request: AddEdgesBatchRequest): Promise<number> {
  return invoke<number>('plugin:velesdb|add_edges_batch', { request });
}

/**
 * Retrieves edges from a knowledge graph collection.
 *
 * @param request - Filter (by label, source, or target node)
 * @returns Array of matching edges
 * @throws {CommandError} If the collection doesn't exist
 *
 * @example
 * ```typescript
 * const edges = await getEdges({ collection: 'social', label: 'FOLLOWS' });
 * ```
 */
export function getEdges(request: GetEdgesRequest): Promise<EdgeOutput[]> {
  return invoke<EdgeOutput[]>('plugin:velesdb|get_edges', { request });
}

/**
 * Traverses the knowledge graph from a source node.
 *
 * @param request - Traversal parameters (source, maxDepth, algorithm, optional relTypes filter)
 * @returns Array of reachable nodes with their depth and path
 * @throws {CommandError} If the collection doesn't exist
 *
 * @example
 * ```typescript
 * const result = await traverseGraph({
 *   collection: 'social', source: 100, algorithm: 'bfs', maxDepth: 3, limit: 50
 * });
 * ```
 */
export function traverseGraph(request: TraverseGraphRequest): Promise<TraversalOutput[]> {
  return invoke<TraversalOutput[]>('plugin:velesdb|traverse_graph', { request });
}

/**
 * Gets the in-degree and out-degree of a graph node.
 *
 * @param request - Collection name and node ID
 * @returns Node degree information (inDegree, outDegree)
 * @throws {CommandError} If the collection doesn't exist
 *
 * @example
 * ```typescript
 * const degree = await getNodeDegree({ collection: 'social', nodeId: 100 });
 * console.log(`In: ${degree.inDegree}, Out: ${degree.outDegree}`);
 * ```
 */
export function getNodeDegree(request: GetNodeDegreeRequest): Promise<NodeDegreeOutput> {
  return invoke<NodeDegreeOutput>('plugin:velesdb|get_node_degree', { request });
}

/**
 * Creates a graph collection with optional schema.
 *
 * @param request - Graph collection configuration
 * @returns Collection info
 * @throws {CommandError} If collection already exists or parameters are invalid
 *
 * @example
 * ```typescript
 * // Schemaless graph (default)
 * const info = await createGraphCollection({ name: 'knowledge' });
 *
 * // Graph with embeddings
 * const info = await createGraphCollection({
 *   name: 'knowledge', dimension: 768, metric: 'cosine',
 *   graphSchema: { schemaless: true }
 * });
 * ```
 */
export function createGraphCollection(request: CreateGraphCollectionRequest): Promise<CollectionInfo> {
  return invoke<CollectionInfo>('plugin:velesdb|create_graph_collection', { request });
}

// ============================================================================
// Scroll / Pagination
// ============================================================================

/** Request to scroll through collection points. */
export interface ScrollRequest {
  /** Target collection name. */
  collection: string;
  /** Cursor from a previous scroll (omit for the first batch): the
   * `nextCursor` string as received, or a number. */
  cursor?: string | number;
  /** Number of points per batch. Default: 100. */
  batchSize?: number;
  /** Optional metadata filter. */
  filter?: Record<string, unknown>;
}

/** Response from a scroll operation. */
export interface ScrollResponse {
  /** Points in this batch. */
  points: PointOutput[];
  /** Cursor for the next batch, as a decimal string (absent when no more
   * points). */
  nextCursor?: string;
}

/**
 * Scrolls through collection points with cursor-based pagination.
 *
 * @param request - Scroll parameters
 * @returns Batch of points and optional next cursor
 * @throws {CommandError} If collection doesn't exist
 *
 * @example
 * ```typescript
 * let cursor: string | undefined;
 * do {
 *   const batch = await scrollCollection({ collection: 'docs', cursor, batchSize: 50 });
 *   batch.points.forEach(p => console.log(p.id));
 *   cursor = batch.nextCursor;
 * } while (cursor !== undefined);
 * ```
 */
export function scrollCollection(request: ScrollRequest): Promise<ScrollResponse> {
  return invoke<ScrollResponse>('plugin:velesdb|scroll_collection', { request });
}

// ============================================================================
// Agent Memory — Semantic
// ============================================================================

/** Request to store knowledge in semantic memory. */
export interface SemanticStoreRequest {
  /** Unique ID for this knowledge fact. */
  id: string | number;
  /** Text content of the knowledge. */
  content: string;
  /** Embedding vector for the content. */
  embedding: number[];
}

/** Request to query semantic memory. */
export interface SemanticQueryRequest {
  /** Query embedding vector. */
  embedding: number[];
  /** Number of results to return. Default: 10. */
  topK?: number;
  /** Search quality: 'fast', 'balanced', 'accurate', 'perfect', 'auto', 'custom:<ef>' or 'adaptive:<min_ef>:<max_ef>', each ef in 16-4096; any other value is refused. Omitted keeps the memory collection's own quality. */
  quality?: string;
}

/** Result from semantic memory query. */
export interface SemanticQueryResult {
  /** Knowledge fact ID. */
  id: string;
  /** Similarity score. */
  score: number;
  /** Knowledge content text. */
  content: string;
}

/**
 * Stores a knowledge fact in semantic memory.
 *
 * @param request - Semantic store request
 * @throws {CommandError} On storage failure
 */
export async function semanticStore(request: SemanticStoreRequest): Promise<void> {
  await invoke('plugin:velesdb|semantic_store', { request });
}

/**
 * Queries semantic memory by similarity search.
 *
 * @param request - Semantic query request
 * @returns Array of matching knowledge facts
 */
export function semanticQuery(request: SemanticQueryRequest): Promise<SemanticQueryResult[]> {
  return invoke<SemanticQueryResult[]>('plugin:velesdb|semantic_query', { request });
}

/** Request to store knowledge in semantic memory with a TTL. */
export interface SemanticStoreWithTtlRequest {
  /** Unique ID for this knowledge fact. */
  id: string | number;
  /** Text content of the knowledge. */
  content: string;
  /** Embedding vector for the content. */
  embedding: number[];
  /** Time-to-live in seconds before the entry expires. */
  ttlSeconds: number;
}

/** Request to delete a semantic memory entry by ID. */
export interface SemanticDeleteRequest {
  /** Knowledge fact ID to delete. */
  id: string | number;
}

/**
 * Stores a knowledge fact in semantic memory with a time-to-live.
 *
 * Once the TTL elapses the entry is hidden from reads, and the next
 * {@link memoryAutoExpire} pass removes it.
 *
 * @param request - Semantic store request with TTL
 * @throws {CommandError} On dimension mismatch or storage failure
 */
export async function semanticStoreWithTtl(request: SemanticStoreWithTtlRequest): Promise<void> {
  await invoke('plugin:velesdb|semantic_store_with_ttl', { request });
}

/**
 * Deletes a knowledge fact from semantic memory.
 *
 * @param request - ID of the fact to delete
 * @throws {CommandError} On storage failure
 */
export async function semanticDelete(request: SemanticDeleteRequest): Promise<void> {
  await invoke('plugin:velesdb|semantic_delete', { request });
}

/**
 * Returns the embedding dimension of semantic memory: 384 unless the host app
 * set another with `Builder::with_memory_dimension` on the Rust side. Every
 * agent-memory embedding must have this length.
 *
 * @returns Embedding dimension
 */
export function semanticDimension(): Promise<number> {
  return invoke<number>('plugin:velesdb|semantic_dimension');
}

/**
 * Serializes semantic memory to snapshot bytes.
 *
 * @returns Snapshot bytes, one array element (0-255) per byte
 * @throws {CommandError} On serialization failure
 */
export function semanticSerialize(): Promise<number[]> {
  return invoke<number[]>('plugin:velesdb|semantic_serialize');
}

/**
 * Restores semantic memory from snapshot bytes produced by {@link semanticSerialize}.
 *
 * @param data - Snapshot bytes as a plain array (convert a `Uint8Array` with `Array.from`)
 * @throws {CommandError} If the snapshot is invalid
 */
export async function semanticDeserialize(data: number[]): Promise<void> {
  await invoke('plugin:velesdb|semantic_deserialize', { data });
}

// ============================================================================
// Agent Memory — Episodic
// ============================================================================

/** Request to record an episode. */
export interface EpisodicRecordRequest {
  /** Episode event ID. */
  eventId: string | number;
  /** Episode description/content. */
  content: string;
  /** Timestamp (epoch seconds). */
  timestamp: number;
  /** Embedding vector for the episode. */
  embedding: number[];
}

/** Request to query recent episodes. */
export interface EpisodicRecentRequest {
  /** Number of recent episodes to return. Default: 10. */
  limit?: number;
  /** Only return episodes since this timestamp (epoch seconds). */
  sinceTimestamp?: number;
}

/** Result from episodic memory query. */
export interface EpisodicResult {
  /** Episode ID. */
  id: string;
  /** Episode content. */
  content: string;
  /** Timestamp (epoch seconds). */
  timestamp: number;
}

/**
 * Records an episode in episodic memory.
 *
 * @param request - Episode to record
 * @throws {CommandError} On storage failure
 */
export async function episodicRecord(request: EpisodicRecordRequest): Promise<void> {
  await invoke('plugin:velesdb|episodic_record', { request });
}

/**
 * Queries recent episodes from episodic memory.
 *
 * @param request - Query parameters (limit, since_timestamp)
 * @returns Array of recent episodes
 */
export function episodicRecent(request: EpisodicRecentRequest): Promise<EpisodicResult[]> {
  return invoke<EpisodicResult[]>('plugin:velesdb|episodic_recent', { request });
}

/** Request to recall episodes similar to a query embedding. */
export interface EpisodicRecallSimilarRequest {
  /** Query embedding vector. */
  embedding: number[];
  /** Number of results to return. Default: 10. */
  topK?: number;
}

/** Request to list episodes older than a timestamp. */
export interface EpisodicOlderThanRequest {
  /** Return episodes recorded before this timestamp (epoch seconds). */
  timestamp: number;
  /** Maximum number of episodes to return. Default: 10. */
  limit?: number;
}

/** Request to delete an episode by event ID. */
export interface EpisodicDeleteRequest {
  /** Episode event ID to delete. */
  eventId: string | number;
}

/** Result from an episodic similarity recall. */
export interface EpisodicSimilarResult {
  /** Episode ID. */
  id: string;
  /** Episode content. */
  content: string;
  /** Timestamp (epoch seconds). */
  timestamp: number;
  /** Similarity score. */
  score: number;
}

/**
 * Recalls episodes similar to a query embedding.
 *
 * @param request - Query embedding and result count
 * @returns Matching episodes with their similarity score
 * @throws {CommandError} On dimension mismatch
 */
export function episodicRecallSimilar(
  request: EpisodicRecallSimilarRequest,
): Promise<EpisodicSimilarResult[]> {
  return invoke<EpisodicSimilarResult[]>('plugin:velesdb|episodic_recall_similar', { request });
}

/**
 * Returns episodes recorded before a timestamp.
 *
 * @param request - Cut-off timestamp and result limit
 * @returns Episodes older than the timestamp
 */
export function episodicOlderThan(request: EpisodicOlderThanRequest): Promise<EpisodicResult[]> {
  return invoke<EpisodicResult[]>('plugin:velesdb|episodic_older_than', { request });
}

/**
 * Deletes an episode from episodic memory.
 *
 * @param request - Event ID of the episode to delete
 * @throws {CommandError} On storage failure
 */
export async function episodicDelete(request: EpisodicDeleteRequest): Promise<void> {
  await invoke('plugin:velesdb|episodic_delete', { request });
}

/**
 * Serializes episodic memory to snapshot bytes.
 *
 * @returns Snapshot bytes, one array element (0-255) per byte
 * @throws {CommandError} On serialization failure
 */
export function episodicSerialize(): Promise<number[]> {
  return invoke<number[]>('plugin:velesdb|episodic_serialize');
}

/**
 * Restores episodic memory from snapshot bytes produced by {@link episodicSerialize}.
 *
 * @param data - Snapshot bytes as a plain array (convert a `Uint8Array` with `Array.from`)
 * @throws {CommandError} If the snapshot is invalid
 */
export async function episodicDeserialize(data: number[]): Promise<void> {
  await invoke('plugin:velesdb|episodic_deserialize', { data });
}

// ============================================================================
// Agent Memory — Procedural
// ============================================================================

/** Request to learn a procedure. */
export interface ProceduralLearnRequest {
  /** Procedure ID. */
  procedureId: string | number;
  /** Procedure name. */
  name: string;
  /** Steps to perform. */
  steps: string[];
  /** Embedding vector for the procedure. */
  embedding: number[];
  /** Confidence level (0.0-1.0). Default: 1.0. */
  confidence?: number;
}

/** Request to recall procedures by similarity. */
export interface ProceduralRecallRequest {
  /** Query embedding vector. */
  embedding: number[];
  /** Number of results. Default: 10. */
  topK?: number;
  /** Minimum confidence threshold. Default: 0.0. */
  minConfidence?: number;
}

/** Result from procedural memory recall. */
export interface ProceduralMatchResult {
  /** Procedure ID. */
  id: string;
  /** Procedure name. */
  name: string;
  /** Steps. */
  steps: string[];
  /** Confidence score. */
  confidence: number;
  /** Similarity score from vector search. */
  score: number;
}

/**
 * Learns a procedure in procedural memory.
 *
 * @param request - Procedure to learn
 * @throws {CommandError} On storage failure
 */
export async function proceduralLearn(request: ProceduralLearnRequest): Promise<void> {
  await invoke('plugin:velesdb|procedural_learn', { request });
}

/**
 * Recalls procedures by similarity from procedural memory.
 *
 * @param request - Recall parameters
 * @returns Array of matching procedures
 */
export function proceduralRecall(request: ProceduralRecallRequest): Promise<ProceduralMatchResult[]> {
  return invoke<ProceduralMatchResult[]>('plugin:velesdb|procedural_recall', { request });
}

/** Request to reinforce a stored procedure. */
export interface ProceduralReinforceRequest {
  /** Procedure ID to reinforce. */
  procedureId: string | number;
  /** Whether the procedure execution succeeded. */
  success: boolean;
}

/** Request to delete a procedure by ID. */
export interface ProceduralDeleteRequest {
  /** Procedure ID to delete. */
  procedureId: string | number;
}

/**
 * Reinforces a stored procedure with the outcome of an execution, using the
 * configured reinforcement strategy.
 *
 * @param request - Procedure ID and outcome
 * @throws {CommandError} If the procedure is unknown or expired
 */
export async function proceduralReinforce(request: ProceduralReinforceRequest): Promise<void> {
  await invoke('plugin:velesdb|procedural_reinforce', { request });
}

/**
 * Lists all stored (non-expired) procedures.
 *
 * No similarity search runs, so every result's `score` is `0`.
 *
 * @returns All stored procedures
 */
export function proceduralListAll(): Promise<ProceduralMatchResult[]> {
  return invoke<ProceduralMatchResult[]>('plugin:velesdb|procedural_list_all');
}

/**
 * Deletes a procedure from procedural memory.
 *
 * @param request - ID of the procedure to delete
 * @throws {CommandError} On storage failure
 */
export async function proceduralDelete(request: ProceduralDeleteRequest): Promise<void> {
  await invoke('plugin:velesdb|procedural_delete', { request });
}

/**
 * Serializes procedural memory to snapshot bytes.
 *
 * @returns Snapshot bytes, one array element (0-255) per byte
 * @throws {CommandError} On serialization failure
 */
export function proceduralSerialize(): Promise<number[]> {
  return invoke<number[]>('plugin:velesdb|procedural_serialize');
}

/**
 * Restores procedural memory from snapshot bytes produced by {@link proceduralSerialize}.
 *
 * @param data - Snapshot bytes as a plain array (convert a `Uint8Array` with `Array.from`)
 * @throws {CommandError} If the snapshot is invalid
 */
export async function proceduralDeserialize(data: number[]): Promise<void> {
  await invoke('plugin:velesdb|procedural_deserialize', { data });
}

// ============================================================================
// Agent Memory — TTL, eviction and snapshot versioning
// ============================================================================

/** Memory subsystem targeted by a TTL request. */
export type MemoryKind = 'semantic' | 'episodic' | 'procedural';

/** Request to set a TTL on a single memory entry. */
export interface MemoryTtlRequest {
  /** Target memory subsystem. */
  kind: MemoryKind;
  /** Entry ID to expire. */
  id: string | number;
  /** Time-to-live in seconds. */
  ttlSeconds: number;
}

/** Result of a {@link memoryAutoExpire} pass. */
export interface ExpireResult {
  /** Semantic entries expired. */
  semanticExpired: number;
  /** Episodic entries expired. */
  episodicExpired: number;
  /** Procedural entries expired. */
  proceduralExpired: number;
  /** Episodic entries consolidated into semantic memory. */
  episodicConsolidated: number;
  /** Procedural entries evicted for low confidence. */
  proceduralEvicted: number;
  /** `true` when more old episodes remained than this cycle processed. */
  consolidationTruncated: boolean;
}

/** Request to evict procedures below a confidence threshold. */
export interface EvictLowConfidenceRequest {
  /** Minimum confidence; procedures below this are evicted. */
  minConfidence: number;
}

/** Request to load a specific memory snapshot version. */
export interface LoadSnapshotVersionRequest {
  /** Snapshot version number to restore. */
  version: number;
}

/**
 * Sets a TTL on a single memory entry.
 *
 * The id is not checked: a TTL set on an unknown id is a no-op at expiry.
 *
 * @param request - Subsystem, entry ID and TTL
 *
 * @example
 * ```typescript
 * await memorySetTtl({ kind: 'episodic', id: 42, ttlSeconds: 3600 });
 * ```
 */
export async function memorySetTtl(request: MemoryTtlRequest): Promise<void> {
  await invoke('plugin:velesdb|memory_set_ttl', { request });
}

/**
 * Expires entries past their TTL and applies the eviction and consolidation policies.
 *
 * @returns Per-subsystem counts of what the pass removed or consolidated
 * @throws {CommandError} On storage failure
 */
export function memoryAutoExpire(): Promise<ExpireResult> {
  return invoke<ExpireResult>('plugin:velesdb|memory_auto_expire');
}

/**
 * Evicts procedures whose confidence is below a threshold.
 *
 * @param request - Confidence threshold
 * @returns Number of procedures evicted
 * @throws {CommandError} On storage failure
 */
export function memoryEvictLowConfidence(request: EvictLowConfidenceRequest): Promise<number> {
  return invoke<number>('plugin:velesdb|memory_evict_low_confidence', { request });
}

/**
 * Creates a versioned snapshot of the full memory state.
 *
 * @returns The version number of the created snapshot
 * @throws {CommandError} If snapshots are not configured or persistence fails
 */
export function memorySnapshot(): Promise<number> {
  return invoke<number>('plugin:velesdb|memory_snapshot');
}

/**
 * Restores the most recent versioned snapshot.
 *
 * @returns The version number of the restored snapshot
 * @throws {CommandError} If snapshots are not configured, loading fails, or restoration fails
 */
export function memoryLoadLatestSnapshot(): Promise<number> {
  return invoke<number>('plugin:velesdb|memory_load_latest_snapshot');
}

/**
 * Restores a specific versioned snapshot.
 *
 * @param request - Snapshot version to restore
 * @throws {CommandError} If snapshots are not configured, loading fails, or restoration fails
 */
export async function memoryLoadSnapshotVersion(request: LoadSnapshotVersionRequest): Promise<void> {
  await invoke('plugin:velesdb|memory_load_snapshot_version', { request });
}

/**
 * Lists all available snapshot version numbers.
 *
 * @returns Snapshot version numbers
 * @throws {CommandError} If snapshots are not configured or listing fails
 */
export function memoryListSnapshotVersions(): Promise<number[]> {
  return invoke<number[]>('plugin:velesdb|memory_list_snapshot_versions');
}

// ============================================================================
// Agent Memory — VelesQL
// ============================================================================

/** Request executing a VelesQL query against one memory subsystem. */
export interface MemoryQueryRequest {
  /** VelesQL query string. */
  sql: string;
  /** Named query parameters. */
  params?: Record<string, unknown>;
}

/**
 * Executes a VelesQL query against semantic memory.
 *
 * @param request - VelesQL query and parameters
 * @returns Matching rows
 * @throws {CommandError} If the query is invalid
 *
 * @example
 * ```typescript
 * const rows = await memoryQuerySemantic({
 *   sql: 'SELECT * FROM _semantic_memory WHERE vector NEAR $v LIMIT 5',
 *   params: { v: queryEmbedding },
 * });
 * ```
 */
export function memoryQuerySemantic(request: MemoryQueryRequest): Promise<HybridResult[]> {
  return invoke<HybridResult[]>('plugin:velesdb|memory_query_semantic', { request });
}

/**
 * Executes a VelesQL query against episodic memory.
 *
 * @param request - VelesQL query and parameters
 * @returns Matching rows
 * @throws {CommandError} If the query is invalid
 */
export function memoryQueryEpisodic(request: MemoryQueryRequest): Promise<HybridResult[]> {
  return invoke<HybridResult[]>('plugin:velesdb|memory_query_episodic', { request });
}

/**
 * Executes a VelesQL query against procedural memory.
 *
 * @param request - VelesQL query and parameters
 * @returns Matching rows
 * @throws {CommandError} If the query is invalid
 */
export function memoryQueryProcedural(request: MemoryQueryRequest): Promise<HybridResult[]> {
  return invoke<HybridResult[]>('plugin:velesdb|memory_query_procedural', { request });
}

// ============================================================================
// Sparse vectors
// ============================================================================

/** Request for sparse-only vector search. */
export interface SparseSearchRequest {
  /** Target collection name. */
  collection: string;
  /**
   * Sparse vector as `{ "<dimIndex>": weight, ... }`: at most 65,536 entries,
   * finite weights, or the call is refused (VELES-009), as REST refuses it.
   */
  sparseVector: Record<string, number>;
  /** Number of results to return. Default: 10. */
  topK?: number;
  /** Optional named sparse index to query. */
  indexName?: string;
}

/** Request for hybrid dense + sparse search. */
export interface HybridSparseSearchRequest {
  /** Target collection name. */
  collection: string;
  /** Dense query vector. */
  vector: number[];
  /**
   * Sparse vector as `{ "<dimIndex>": weight, ... }`: at most 65,536 entries,
   * finite weights, or the call is refused (VELES-009), as REST refuses it.
   */
  sparseVector: Record<string, number>;
  /** Number of results to return. Default: 10. */
  topK?: number;
}

/** A {@link PointInput} carrying an optional sparse vector. */
export interface SparsePointInput extends PointInput {
  /**
   * Optional sparse vector as `{ "<dimIndex>": weight, ... }`: at most 65,536
   * entries, finite weights, or the upsert is refused (VELES-009).
   */
  sparseVector?: Record<string, number>;
}

/** Request to upsert points with optional sparse vectors. */
export interface SparseUpsertRequest {
  /** Target collection name. */
  collection: string;
  /** Points to insert or update. */
  points: SparsePointInput[];
}

/**
 * Searches a collection using a sparse vector (inverted-index retrieval).
 *
 * @param request - Sparse query parameters
 * @returns Search results ordered by relevance
 * @throws {CommandError} If the collection does not exist
 *
 * @example
 * ```typescript
 * const res = await sparseSearch({
 *   collection: 'docs',
 *   sparseVector: { '12': 0.8, '57': 0.6 },
 *   topK: 10,
 * });
 * ```
 */
export function sparseSearch(request: SparseSearchRequest): Promise<SearchResponse> {
  return invoke<SearchResponse>('plugin:velesdb|sparse_search', { request });
}

/**
 * Searches a collection using both a dense and a sparse vector, fusing results.
 *
 * @param request - Hybrid dense + sparse query parameters
 * @returns Fused search results ordered by relevance
 * @throws {CommandError} If the collection does not exist
 */
export function hybridSparseSearch(request: HybridSparseSearchRequest): Promise<SearchResponse> {
  return invoke<SearchResponse>('plugin:velesdb|hybrid_sparse_search', { request });
}

/**
 * Upserts points with optional sparse vectors for hybrid retrieval.
 *
 * @param request - Points to insert or update
 * @returns Number of points written
 * @throws {CommandError} On dimension mismatch or write failure
 */
export function sparseUpsert(request: SparseUpsertRequest): Promise<number> {
  return invoke<number>('plugin:velesdb|sparse_upsert', { request });
}

// ============================================================================
// Quantization training
// ============================================================================

/** Request to train a Product Quantizer over a collection's vectors. */
export interface TrainPqRequest {
  /** Target collection name. */
  collection: string;
  /**
   * Number of sub-quantizers (the dimension must be divisible by it).
   * Required, as in the REST server's `TRAIN QUANTIZER ... WITH (m = ...)`:
   * the engine reads an absent `m` as `0` and refuses it (VELES-028).
   */
  m: number;
  /** Number of centroids per sub-quantizer. */
  k?: number;
  /** Whether to use Optimized Product Quantization (OPQ). */
  opq?: boolean;
}

/**
 * Trains a Product Quantizer on the collection's existing vectors.
 *
 * @param request - PQ training parameters
 * @returns A human-readable training summary
 * @throws {CommandError} If the collection has too few vectors to train
 */
export function trainPq(request: TrainPqRequest): Promise<string> {
  return invoke<string>('plugin:velesdb|train_pq', { request });
}

// ============================================================================
// Streaming insert (persistence feature only)
// ============================================================================

/** Request to stream-insert points (requires the `persistence` feature). */
export interface StreamInsertRequest {
  /** Target collection name. */
  collection: string;
  /** Points to stream-insert. */
  points: PointInput[];
}

/**
 * Streams a batch of points into a collection for high-throughput ingestion.
 *
 * Only available when the plugin is built with the `persistence` feature.
 *
 * @param request - Points to stream-insert
 * @returns Number of points written
 * @throws {CommandError} If persistence is disabled or the write fails
 */
export function streamInsert(request: StreamInsertRequest): Promise<number> {
  return invoke<number>('plugin:velesdb|stream_insert', { request });
}

/**
 * Request to enable streaming ingestion on a collection (requires the
 * `persistence` feature). Omitted fields fall back to the engine defaults
 * (`bufferSize` 10000, `batchSize` 128, `flushIntervalMs` 50).
 */
export interface EnableStreamingRequest {
  /** Collection name. */
  collection: string;
  /** Capacity of the bounded ingestion channel (backpressure threshold). */
  bufferSize?: number;
  /** Number of points that trigger an immediate micro-batch flush. */
  batchSize?: number;
  /** Maximum time (ms) before a partial batch is flushed. */
  flushIntervalMs?: number;
}

/**
 * Enables streaming ingestion on a collection.
 *
 * Starts the background drain task, so subsequent {@link streamInsert} calls
 * accept points instead of being refused as not configured. Only available
 * when the plugin is built with the `persistence` feature.
 *
 * @param request - Collection and optional streaming tuning
 * @throws {CommandError} If persistence is disabled or the collection is not a vector collection
 *
 * @example
 * ```typescript
 * await enableStreaming({ collection: 'events', batchSize: 256 });
 * await streamInsert({ collection: 'events', points });
 * ```
 */
export async function enableStreaming(request: EnableStreamingRequest): Promise<void> {
  await invoke('plugin:velesdb|enable_streaming', { request });
}

// ============================================================================
// Secondary indexes
// ============================================================================

/** Request to create a secondary index on a metadata field. */
export interface CreateIndexRequest {
  /** Target collection name. */
  collection: string;
  /** Metadata field name to index. */
  fieldName: string;
}

/** Request to drop a secondary index on a metadata field. */
export interface DropIndexRequest {
  /** Target collection name. */
  collection: string;
  /** Metadata field name whose index to drop. */
  fieldName: string;
}

/** Request to list secondary indexes on a collection. */
export interface ListIndexesRequest {
  /** Target collection name. */
  collection: string;
}

/** A single secondary index entry. */
export interface IndexInfoOutput {
  /** Node label (`secondary` for a secondary payload index). */
  label: string;
  /** Property name (the payload field for a secondary index). */
  property: string;
  /** Index type: `hash` (graph property), `range` (graph range) or `btree` (secondary payload index). */
  indexType: string;
  /** Number of distinct values indexed. */
  cardinality: number;
  /** Estimated heap size of the index, in bytes. */
  memoryBytes: number;
}

/**
 * Creates a secondary index on a metadata field for faster filtered search.
 *
 * @param request - Index target
 * @throws {CommandError} If the collection does not exist
 */
export async function createIndex(request: CreateIndexRequest): Promise<void> {
  await invoke('plugin:velesdb|create_index', { request });
}

/**
 * Drops a secondary index on a metadata field.
 *
 * @param request - Index target
 * @returns `true` if an index was removed, `false` if none existed
 * @throws {CommandError} If the collection does not exist
 */
export function dropIndex(request: DropIndexRequest): Promise<boolean> {
  return invoke<boolean>('plugin:velesdb|drop_index', { request });
}

/**
 * Lists all secondary indexes on a collection.
 *
 * @param request - Target collection
 * @returns Array of index descriptors
 * @throws {CommandError} If the collection does not exist
 */
export function listIndexes(request: ListIndexesRequest): Promise<IndexInfoOutput[]> {
  return invoke<IndexInfoOutput[]>('plugin:velesdb|list_indexes', { request });
}

// ============================================================================
// Parallel graph traversal
// ============================================================================

/** Request for multi-source parallel BFS traversal. */
export interface TraverseGraphParallelRequest {
  /** Target collection name. */
  collection: string;
  /** Source node ids to start traversal from, each a decimal string or a number. */
  sources: Array<string | number>;
  /** Maximum traversal depth. Default: 3. */
  maxDepth?: number;
  /** Maximum results to return. Default: 100. */
  limit?: number;
  /** Optional relationship types to follow. */
  relTypes?: string[];
}

/**
 * Runs a multi-source parallel BFS traversal over a graph collection.
 *
 * @param request - Parallel traversal parameters
 * @returns Array of traversal result nodes
 * @throws {CommandError} If the collection is not a graph collection
 */
export function traverseGraphParallel(
  request: TraverseGraphParallelRequest,
): Promise<TraversalOutput[]> {
  return invoke<TraversalOutput[]>('plugin:velesdb|traverse_graph_parallel', { request });
}
