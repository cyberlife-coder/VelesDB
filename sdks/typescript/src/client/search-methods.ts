/**
 * VelesDB Client - Search operation methods
 *
 * Standalone functions implementing search operations (dense, text,
 * hybrid, multi-query, batch, scroll, PQ training, stream insert)
 * for the VelesDB client class.
 * @packageDocumentation
 */

import type {
  IVelesDBBackend,
  VelesDBConfig,
  VectorDocument,
  SearchOptions,
  SearchQuality,
  SearchResult,
  MultiQuerySearchOptions,
  PqTrainOptions,
  ScrollRequest,
  ScrollResponse,
  CollectionStatsResponse,
  CollectionConfigResponse,
  RebuildIndexResponse,
  GuardRailsUpdateRequest,
  GuardRailsConfigResponse,
  AggregateQueryOptions,
  AggregateResponse,
  QueryOptions,
  QueryApiResponse,
  ExplainResponse,
  CollectionSanityResponse,
} from '../types';
import type { FilterInput } from '../filter';
import { ValidationError } from '../types';
import {
  requireNonEmptyString,
  requireVector,
  validateDocsBatch,
  validateDocument,
} from './validation';

/** Search for similar vectors. */
export function search(
  backend: IVelesDBBackend,
  collection: string,
  query: number[] | Float32Array,
  options?: SearchOptions
): Promise<SearchResult[]> {
  requireVector(query, 'Query');
  return backend.search(collection, query, options);
}

/** Search for multiple vectors in parallel. */
export function searchBatch(
  backend: IVelesDBBackend,
  collection: string,
  searches: Array<{
    vector: number[] | Float32Array;
    k?: number;
    filter?: FilterInput;
    quality?: SearchQuality;
  }>
): Promise<SearchResult[][]> {
  if (!Array.isArray(searches)) {
    throw new ValidationError('Searches must be an array');
  }

  for (const s of searches) {
    requireVector(s.vector, 'Each search vector');
  }

  return backend.searchBatch(collection, searches);
}

/** Perform full-text search using BM25. */
export function textSearch(
  backend: IVelesDBBackend,
  collection: string,
  query: string,
  options?: { k?: number; filter?: FilterInput }
): Promise<SearchResult[]> {
  requireNonEmptyString(query, 'Query');
  return backend.textSearch(collection, query, options);
}

/** Perform hybrid search combining vector similarity and BM25 text search. */
export function hybridSearch(
  backend: IVelesDBBackend,
  collection: string,
  vector: number[] | Float32Array,
  textQuery: string,
  options?: { k?: number; vectorWeight?: number; filter?: FilterInput }
): Promise<SearchResult[]> {
  requireVector(vector, 'Vector');
  requireNonEmptyString(textQuery, 'Text query');
  return backend.hybridSearch(collection, vector, textQuery, options);
}

/** Multi-query fusion search combining results from multiple query vectors. */
export function multiQuerySearch(
  backend: IVelesDBBackend,
  collection: string,
  vectors: Array<number[] | Float32Array>,
  options?: MultiQuerySearchOptions
): Promise<SearchResult[]> {
  if (!Array.isArray(vectors) || vectors.length === 0) {
    throw new ValidationError('Vectors must be a non-empty array');
  }

  for (const v of vectors) {
    requireVector(v, 'Each vector');
  }

  return backend.multiQuerySearch(collection, vectors, options);
}

/** Train Product Quantization on a collection. */
export function trainPq(
  backend: IVelesDBBackend,
  collection: string,
  options?: PqTrainOptions
): Promise<string> {
  return backend.trainPq(collection, options);
}

/** Stream-insert documents with backpressure support. */
export function streamInsert(
  backend: IVelesDBBackend,
  config: VelesDBConfig,
  collection: string,
  docs: VectorDocument[]
): Promise<void> {
  validateDocsBatch(docs, doc => validateDocument(doc, config));
  return backend.streamInsert(collection, docs);
}

/** Scroll through collection points with cursor-based pagination. */
export function scroll(
  backend: IVelesDBBackend,
  collection: string,
  request?: ScrollRequest
): Promise<ScrollResponse> {
  requireNonEmptyString(collection, 'Collection name');

  if (request?.batchSize !== undefined) {
    if (request.batchSize < 1 || request.batchSize > 10000) {
      throw new ValidationError('batchSize must be between 1 and 10000');
    }
  }

  return backend.scroll(collection, request);
}

/** Search returning only IDs and scores (lightweight). */
export function searchIds(
  backend: IVelesDBBackend,
  collection: string,
  query: number[] | Float32Array,
  options?: SearchOptions
): Promise<Array<{ id: number; score: number }>> {
  return backend.searchIds(collection, query, options);
}

/** Execute a VelesQL multi-model query. */
export function query(
  backend: IVelesDBBackend,
  collection: string,
  queryString: string,
  params?: Record<string, unknown>,
  options?: QueryOptions
): Promise<QueryApiResponse> {
  requireNonEmptyString(collection, 'Collection name');
  requireNonEmptyString(queryString, 'Query string');
  return backend.query(collection, queryString, params, options);
}

/** Explain the execution plan for a VelesQL query. */
export function queryExplain(
  backend: IVelesDBBackend,
  queryString: string,
  params?: Record<string, unknown>,
  options?: { analyze?: boolean }
): Promise<ExplainResponse> {
  requireNonEmptyString(queryString, 'Query string');
  return backend.queryExplain(queryString, params, options);
}

/** Run collection sanity checks. */
export function collectionSanity(
  backend: IVelesDBBackend,
  collection: string
): Promise<CollectionSanityResponse> {
  requireNonEmptyString(collection, 'Collection name');
  return backend.collectionSanity(collection);
}

/** Get collection statistics (requires prior analyze). */
export function getCollectionStats(
  backend: IVelesDBBackend,
  collection: string
): Promise<CollectionStatsResponse | null> {
  return backend.getCollectionStats(collection);
}

/** Analyze a collection to compute statistics. */
export function analyzeCollection(
  backend: IVelesDBBackend,
  collection: string
): Promise<CollectionStatsResponse> {
  return backend.analyzeCollection(collection);
}

/** Get collection configuration. */
export function getCollectionConfig(
  backend: IVelesDBBackend,
  collection: string
): Promise<CollectionConfigResponse> {
  return backend.getCollectionConfig(collection);
}

/** Rebuild the HNSW index of a collection (compacts tombstones). */
export function rebuildIndex(
  backend: IVelesDBBackend,
  collection: string
): Promise<RebuildIndexResponse> {
  requireNonEmptyString(collection, 'Collection');
  return backend.rebuildIndex(collection);
}

/** Read the current process-wide guard-rails configuration. */
export function getGuardrails(
  backend: IVelesDBBackend
): Promise<GuardRailsConfigResponse> {
  return backend.getGuardrails();
}

/** Partial-update the process-wide guard-rails configuration. */
export function updateGuardrails(
  backend: IVelesDBBackend,
  req: GuardRailsUpdateRequest
): Promise<GuardRailsConfigResponse> {
  return backend.updateGuardrails(req);
}

/** Execute a VelesQL aggregate query (COUNT/AVG/GROUP BY/...). */
export function aggregate(
  backend: IVelesDBBackend,
  queryString: string,
  params?: Record<string, unknown>,
  options?: AggregateQueryOptions
): Promise<AggregateResponse> {
  requireNonEmptyString(queryString, 'Query string');
  return backend.aggregate(queryString, params, options);
}
