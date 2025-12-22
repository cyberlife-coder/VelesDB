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

/** Request to create a new collection. */
export interface CreateCollectionRequest {
  /** Collection name (unique identifier). */
  name: string;
  /** Vector dimension (e.g., 768 for BERT, 1536 for GPT). */
  dimension: number;
  /** Distance metric for similarity calculations. Default: 'cosine'. */
  metric?: DistanceMetric;
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
}

/** Request for BM25 text search. */
export interface TextSearchRequest {
  /** Target collection name. */
  collection: string;
  /** Text query for BM25 search. */
  query: string;
  /** Number of results to return. Default: 10. */
  topK?: number;
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
}

/** Request for VelesQL query. */
export interface QueryRequest {
  /** VelesQL query string. */
  query: string;
  /** Query parameters (for parameterized queries). */
  params?: Record<string, unknown>;
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
 * Executes a VelesQL query.
 * 
 * @param request - Query request with VelesQL string
 * @returns Search response with results and timing
 * @throws {CommandError} If query syntax is invalid or collection doesn't exist
 * 
 * @example
 * ```typescript
 * const response = await query({
 *   query: "SELECT * FROM documents WHERE content MATCH 'rust programming' LIMIT 10",
 *   params: {}
 * });
 * ```
 */
export async function query(request: QueryRequest): Promise<SearchResponse> {
  return invoke<SearchResponse>('plugin:velesdb|query', { request });
}
