/**
 * Search operations for REST backend
 */

import type {
  SearchOptions,
  SearchResult,
  MultiQuerySearchOptions,
} from '../../types';
import { NotFoundError, VelesDBError } from '../../types';
import type { HttpClient } from './http-client';
import type { BatchSearchResponse } from './server-types';

export async function search(
  client: HttpClient,
  collection: string,
  query: number[] | Float32Array,
  options?: SearchOptions
): Promise<SearchResult[]> {
  client.ensureInitialized();

  const queryVector = query instanceof Float32Array ? Array.from(query) : query;

  const body: Record<string, unknown> = {
    vector: queryVector,
    top_k: options?.k ?? 10,
    filter: options?.filter,
    include_vectors: options?.includeVectors ?? false,
  };

  if (options?.efSearch !== undefined) {
    body.ef_search = options.efSearch;
  }
  if (options?.mode !== undefined) {
    body.mode = options.mode;
  }
  if (options?.timeoutMs !== undefined) {
    body.timeout_ms = options.timeoutMs;
  }

  const response = await client.request<{ results: SearchResult[] }>(
    'POST',
    `/collections/${encodeURIComponent(collection)}/search`,
    body
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return response.data?.results ?? [];
}

export async function searchBatch(
  client: HttpClient,
  collection: string,
  searches: Array<{
    vector: number[] | Float32Array;
    k?: number;
    filter?: Record<string, unknown>;
    includeVectors?: boolean;
  }>
): Promise<SearchResult[][]> {
  client.ensureInitialized();

  const formattedSearches = searches.map(s => ({
    vector: s.vector instanceof Float32Array ? Array.from(s.vector) : s.vector,
    top_k: s.k ?? 10,
    filter: s.filter,
    include_vectors: s.includeVectors ?? false,
  }));

  const response = await client.request<BatchSearchResponse>(
    'POST',
    `/collections/${encodeURIComponent(collection)}/search/batch`,
    { searches: formattedSearches }
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return response.data?.results.map(r => r.results) ?? [];
}

export async function textSearch(
  client: HttpClient,
  collection: string,
  query: string,
  options?: { k?: number; filter?: Record<string, unknown> }
): Promise<SearchResult[]> {
  client.ensureInitialized();

  const response = await client.request<{ results: SearchResult[] }>(
    'POST',
    `/collections/${encodeURIComponent(collection)}/search/text`,
    {
      query,
      top_k: options?.k ?? 10,
      filter: options?.filter,
    }
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return response.data?.results ?? [];
}

export async function hybridSearch(
  client: HttpClient,
  collection: string,
  vector: number[] | Float32Array,
  textQuery: string,
  options?: { k?: number; vectorWeight?: number; filter?: Record<string, unknown> }
): Promise<SearchResult[]> {
  client.ensureInitialized();

  const queryVector = vector instanceof Float32Array ? Array.from(vector) : vector;

  const response = await client.request<{ results: SearchResult[] }>(
    'POST',
    `/collections/${encodeURIComponent(collection)}/search/hybrid`,
    {
      vector: queryVector,
      query: textQuery,
      top_k: options?.k ?? 10,
      vector_weight: options?.vectorWeight ?? 0.5,
      filter: options?.filter,
    }
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return response.data?.results ?? [];
}

export async function multiQuerySearch(
  client: HttpClient,
  collection: string,
  vectors: Array<number[] | Float32Array>,
  options?: MultiQuerySearchOptions
): Promise<SearchResult[]> {
  client.ensureInitialized();

  const formattedVectors = vectors.map(v =>
    v instanceof Float32Array ? Array.from(v) : v
  );

  const response = await client.request<{ results: SearchResult[] }>(
    'POST',
    `/collections/${encodeURIComponent(collection)}/search/multi`,
    {
      vectors: formattedVectors,
      top_k: options?.k ?? 10,
      strategy: options?.fusion ?? 'rrf',
      rrf_k: options?.fusionParams?.k ?? 60,
      avg_weight: options?.fusionParams?.avgWeight,
      max_weight: options?.fusionParams?.maxWeight,
      hit_weight: options?.fusionParams?.hitWeight,
      filter: options?.filter,
    }
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return response.data?.results ?? [];
}
