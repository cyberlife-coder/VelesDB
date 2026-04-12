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
  id: number;
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
  id: number;
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
  /** Search quality: 'fast', 'balanced', 'accurate', 'perfect', 'auto', or 'custom:<ef>'. */
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
}

/** Request to get points by IDs. */
export interface GetPointsRequest {
  /** Target collection name. */
  collection: string;
  /** Point IDs to retrieve. */
  ids: number[];
}

/** Request to delete points by IDs. */
export interface DeletePointsRequest {
  /** Target collection name. */
  collection: string;
  /** Point IDs to delete. */
  ids: number[];
}

/** Individual search request within a batch. */
export interface IndividualSearchRequest {
  /** Query vector. */
  vector: number[];
  /** Number of results to return. Default: 10. */
  topK?: number;
  /** Optional metadata filter. */
  filter?: Record<string, unknown>;
  /** Search quality: 'fast', 'balanced', 'accurate', 'perfect', 'auto', or 'custom:<ef>'. */
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
export type FusionStrategy = 'rrf' | 'average' | 'maximum' | 'weighted' | 'relative_score';

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
}

/** Request for multi-query fusion search. */
export interface MultiQuerySearchRequest {
  /** Target collection name. */
  collection: string;
  /** List of query vectors. */
  vectors: number[][];
  /** Number of results to return. Default: 10. */
  topK?: number;
  /** Fusion strategy: 'rrf', 'average', 'maximum', 'weighted', 'relative_score'. Default: 'rrf'. */
  fusion?: FusionStrategy;
  /** Fusion parameters. */
  fusionParams?: FusionParams;
  /** Optional metadata filter. */
  filter?: Record<string, unknown>;
}

/** Point output for get operations. */
export interface PointOutput {
  /** Point ID. */
  id: number;
  /** Vector data. */
  vector: number[];
  /** Point payload (if any). */
  payload?: Record<string, unknown>;
}

/** Search result item. */
export interface SearchResult {
  /** Point ID. */
  id: number;
  /** Similarity/distance score. */
  score: number;
  /** Point payload (if any). */
  payload?: Record<string, unknown>;
}

/** Response from search operations. */
export interface SearchResponse {
  /** Search results ordered by relevance. */
  results: SearchResult[];
  /** Query execution time in milliseconds. */
  timingMs: number;
}

/** Hybrid result from VelesQL queries (vector + graph + column data). */
export interface HybridResult {
  /** Node/point ID. */
  nodeId: number;
  /** Vector similarity score (if applicable). */
  vectorScore?: number;
  /** Graph traversal score (if applicable). */
  graphScore?: number;
  /** Fused score combining vector and graph components. */
  fusedScore: number;
  /** Payload/bindings from the query. */
  bindings?: Record<string, unknown>;
  /** Column data from aggregation queries. */
  columnData?: Record<string, unknown>;
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
  /** Error code for programmatic handling. */
  code: string;
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
export async function createCollection(request: CreateCollectionRequest): Promise<CollectionInfo> {
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
export async function createMetadataCollection(request: CreateMetadataCollectionRequest): Promise<CollectionInfo> {
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
  return invoke<void>('plugin:velesdb|delete_collection', { name });
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
export async function listCollections(): Promise<CollectionInfo[]> {
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
export async function getCollection(name: string): Promise<CollectionInfo> {
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
export async function upsert(request: UpsertRequest): Promise<number> {
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
export async function upsertMetadata(request: UpsertMetadataRequest): Promise<number> {
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
export async function search(request: SearchRequest): Promise<SearchResponse> {
  return invoke<SearchResponse>('plugin:velesdb|search', { request });
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
export async function textSearch(request: TextSearchRequest): Promise<SearchResponse> {
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
export async function hybridSearch(request: HybridSearchRequest): Promise<SearchResponse> {
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
 * // MATCH (graph) query
 * const response = await query({
 *   query: "MATCH (d:Doc)-[:AUTHORED_BY]->(a:Person) RETURN a.name",
 *   params: {}
 * });
 * ```
 */
export async function query(request: QueryRequest): Promise<QueryResponse> {
  return invoke<QueryResponse>('plugin:velesdb|query', { request });
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
export async function getPoints(request: GetPointsRequest): Promise<Array<PointOutput | null>> {
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
  return invoke<void>('plugin:velesdb|delete_points', { request });
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
 *     { vector: embedding2, topK: 10, filter: { category: 'tech' } }
 *   ]
 * });
 * responses.forEach((resp, i) => {
 *   console.log(`Query ${i}: ${resp.results.length} results in ${resp.timingMs}ms`);
 * });
 * ```
 */
export async function batchSearch(request: BatchSearchRequest): Promise<SearchResponse[]> {
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
export async function multiQuerySearch(request: MultiQuerySearchRequest): Promise<SearchResponse> {
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
export async function isEmpty(name: string): Promise<boolean> {
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
  return invoke<void>('plugin:velesdb|flush', { name });
}

// ============================================================================
// Knowledge Graph API
// ============================================================================

/** Request to add an edge to a graph collection. */
export interface AddEdgeRequest {
  collection: string;
  id: number;
  source: number;
  target: number;
  label: string;
  properties?: Record<string, unknown>;
}

/** Request to retrieve edges from a graph collection. */
export interface GetEdgesRequest {
  collection: string;
  label?: string;
  source?: number;
  target?: number;
}

/** Request to traverse the knowledge graph. */
export interface TraverseGraphRequest {
  collection: string;
  source: number;
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
  nodeId: number;
}

/** A single edge in the knowledge graph. */
export interface EdgeOutput {
  id: number;
  source: number;
  target: number;
  label: string;
  properties: Record<string, unknown>;
}

/** A single traversal result node. */
export interface TraversalOutput {
  targetId: number;
  depth: number;
  path: number[];
}

/** In/out degree of a graph node. */
export interface NodeDegreeOutput {
  nodeId: number;
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
  return invoke<void>('plugin:velesdb|add_edge', { request });
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
export async function getEdges(request: GetEdgesRequest): Promise<EdgeOutput[]> {
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
export async function traverseGraph(request: TraverseGraphRequest): Promise<TraversalOutput[]> {
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
export async function getNodeDegree(request: GetNodeDegreeRequest): Promise<NodeDegreeOutput> {
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
export async function createGraphCollection(request: CreateGraphCollectionRequest): Promise<CollectionInfo> {
  return invoke<CollectionInfo>('plugin:velesdb|create_graph_collection', { request });
}

// ============================================================================
// Scroll / Pagination
// ============================================================================

/** Request to scroll through collection points. */
export interface ScrollRequest {
  /** Target collection name. */
  collection: string;
  /** Cursor from a previous scroll (omit for the first batch). */
  cursor?: number;
  /** Number of points per batch. Default: 100. */
  batchSize?: number;
  /** Optional metadata filter. */
  filter?: Record<string, unknown>;
}

/** Response from a scroll operation. */
export interface ScrollResponse {
  /** Points in this batch. */
  points: PointOutput[];
  /** Cursor for the next batch (absent when no more points). */
  nextCursor?: number;
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
 * let cursor: number | undefined;
 * do {
 *   const batch = await scrollCollection({ collection: 'docs', cursor, batchSize: 50 });
 *   batch.points.forEach(p => console.log(p.id));
 *   cursor = batch.nextCursor;
 * } while (cursor !== undefined);
 * ```
 */
export async function scrollCollection(request: ScrollRequest): Promise<ScrollResponse> {
  return invoke<ScrollResponse>('plugin:velesdb|scroll_collection', { request });
}

// ============================================================================
// Agent Memory — Semantic
// ============================================================================

/** Request to store knowledge in semantic memory. */
export interface SemanticStoreRequest {
  /** Unique ID for this knowledge fact. */
  id: number;
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
}

/** Result from semantic memory query. */
export interface SemanticQueryResult {
  /** Knowledge fact ID. */
  id: number;
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
  return invoke<void>('plugin:velesdb|semantic_store', { request });
}

/**
 * Queries semantic memory by similarity search.
 *
 * @param request - Semantic query request
 * @returns Array of matching knowledge facts
 */
export async function semanticQuery(request: SemanticQueryRequest): Promise<SemanticQueryResult[]> {
  return invoke<SemanticQueryResult[]>('plugin:velesdb|semantic_query', { request });
}

// ============================================================================
// Agent Memory — Episodic
// ============================================================================

/** Request to record an episode. */
export interface EpisodicRecordRequest {
  /** Episode event ID. */
  eventId: number;
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
  id: number;
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
  return invoke<void>('plugin:velesdb|episodic_record', { request });
}

/**
 * Queries recent episodes from episodic memory.
 *
 * @param request - Query parameters (limit, since_timestamp)
 * @returns Array of recent episodes
 */
export async function episodicRecent(request: EpisodicRecentRequest): Promise<EpisodicResult[]> {
  return invoke<EpisodicResult[]>('plugin:velesdb|episodic_recent', { request });
}

// ============================================================================
// Agent Memory — Procedural
// ============================================================================

/** Request to learn a procedure. */
export interface ProceduralLearnRequest {
  /** Procedure ID. */
  procedureId: number;
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
  id: number;
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
  return invoke<void>('plugin:velesdb|procedural_learn', { request });
}

/**
 * Recalls procedures by similarity from procedural memory.
 *
 * @param request - Recall parameters
 * @returns Array of matching procedures
 */
export async function proceduralRecall(request: ProceduralRecallRequest): Promise<ProceduralMatchResult[]> {
  return invoke<ProceduralMatchResult[]>('plugin:velesdb|procedural_recall', { request });
}
