/**
 * Search Backend operations for VelesDB REST API.
 *
 * Extracted from rest.ts to keep file size manageable.
 * Implements: search, searchBatch, textSearch, hybridSearch,
 * multiQuerySearch, multiQuerySearchIds, and searchIds.
 */

import type {
  SearchOptions,
  SearchResult,
  MultiQuerySearchOptions,
  SparseVector,
  SparseSearchNamedOptions,
  SearchQuality,
} from '../types';
import type { FilterInput } from '../filter';
import type { BaseTransport } from './shared';
import { throwOnError, collectionPath, toNumberArray } from './shared';
import { searchQualityToMode } from '../search-quality';

/** Batch search response structure (mirrors rest.ts private type). */
interface BatchSearchResponse {
  results: Array<{ results: SearchResult[] }>;
}

/** Minimal transport interface for search operations. */
export interface SearchTransport extends BaseTransport {
  sparseToRest(sv: SparseVector): Record<string, number>;
}

export async function search(
  transport: SearchTransport,
  collection: string,
  query: number[] | Float32Array,
  options?: SearchOptions
): Promise<SearchResult[]> {
  const queryVector = toNumberArray(query);

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const body: Record<string, any> = {
    vector: queryVector,
    top_k: options?.k ?? 10,
    filter: options?.filter,
    include_vectors: options?.includeVectors ?? false,
    ...searchQualityToMode(options?.quality),
  };

  if (options?.sparseVector) {
    body.sparse_vector = transport.sparseToRest(options.sparseVector);
  }
  if (options?.sparseIndexName) {
    body.sparse_index = options.sparseIndexName;
  }

  const response = await transport.requestJson<{ results: SearchResult[] }>(
    'POST',
    `${collectionPath(collection)}/search`,
    body
  );

  throwOnError(response, `Collection '${collection}'`);

  return response.data?.results ?? [];
}

export async function searchBatch(
  transport: SearchTransport,
  collection: string,
  searches: Array<{
    vector: number[] | Float32Array;
    k?: number;
    filter?: FilterInput;
    quality?: SearchQuality;
  }>
): Promise<SearchResult[][]> {
  const formattedSearches = searches.map((s) => ({
    vector: toNumberArray(s.vector),
    top_k: s.k ?? 10,
    filter: s.filter,
    ...searchQualityToMode(s.quality),
  }));

  const response = await transport.requestJson<BatchSearchResponse>(
    'POST',
    `${collectionPath(collection)}/search/batch`,
    { searches: formattedSearches }
  );

  throwOnError(response, `Collection '${collection}'`);

  return response.data?.results.map((r) => r.results) ?? [];
}

export async function textSearch(
  transport: SearchTransport,
  collection: string,
  query: string,
  options?: { k?: number; filter?: FilterInput }
): Promise<SearchResult[]> {
  const response = await transport.requestJson<{ results: SearchResult[] }>(
    'POST',
    `${collectionPath(collection)}/search/text`,
    {
      query,
      top_k: options?.k ?? 10,
      filter: options?.filter,
    }
  );

  throwOnError(response, `Collection '${collection}'`);

  return response.data?.results ?? [];
}

export async function hybridSearch(
  transport: SearchTransport,
  collection: string,
  vector: number[] | Float32Array,
  textQuery: string,
  options?: { k?: number; vectorWeight?: number; filter?: FilterInput }
): Promise<SearchResult[]> {
  const queryVector = toNumberArray(vector);

  const response = await transport.requestJson<{ results: SearchResult[] }>(
    'POST',
    `${collectionPath(collection)}/search/hybrid`,
    {
      vector: queryVector,
      query: textQuery,
      top_k: options?.k ?? 10,
      vector_weight: options?.vectorWeight ?? 0.5,
      filter: options?.filter,
    }
  );

  throwOnError(response, `Collection '${collection}'`);

  return response.data?.results ?? [];
}

export async function multiQuerySearch(
  transport: SearchTransport,
  collection: string,
  vectors: Array<number[] | Float32Array>,
  options?: MultiQuerySearchOptions
): Promise<SearchResult[]> {
  const formattedVectors = vectors.map(toNumberArray);

  const response = await transport.requestJson<{ results: SearchResult[] }>(
    'POST',
    `${collectionPath(collection)}/search/multi`,
    {
      vectors: formattedVectors,
      top_k: options?.k ?? 10,
      strategy: options?.fusion ?? 'rrf',
      rrf_k: options?.fusionParams?.k ?? 60,
      avg_weight: options?.fusionParams?.avgWeight,
      max_weight: options?.fusionParams?.maxWeight,
      hit_weight: options?.fusionParams?.hitWeight,
      dense_weight: options?.fusionParams?.denseWeight,
      sparse_weight: options?.fusionParams?.sparseWeight,
      filter: options?.filter,
    }
  );

  throwOnError(response, `Collection '${collection}'`);

  return response.data?.results ?? [];
}

/**
 * Multi-query fusion search returning only IDs and scores (no payloads).
 *
 * Lighter than {@link multiQuerySearch} when payloads are not needed — the
 * server skips payload hydration. Metadata filters are not supported on this
 * endpoint; use {@link multiQuerySearch} for filtered fusion.
 */
export async function multiQuerySearchIds(
  transport: SearchTransport,
  collection: string,
  vectors: Array<number[] | Float32Array>,
  options?: MultiQuerySearchOptions
): Promise<Array<{ id: number; score: number }>> {
  const formattedVectors = vectors.map(toNumberArray);

  const response = await transport.requestJson<{
    results: Array<{ id: number; score: number }>;
  }>('POST', `${collectionPath(collection)}/search/multi/ids`, {
    vectors: formattedVectors,
    top_k: options?.k ?? 10,
    strategy: options?.fusion ?? 'rrf',
    rrf_k: options?.fusionParams?.k ?? 60,
    avg_weight: options?.fusionParams?.avgWeight,
    max_weight: options?.fusionParams?.maxWeight,
    hit_weight: options?.fusionParams?.hitWeight,
    dense_weight: options?.fusionParams?.denseWeight,
    sparse_weight: options?.fusionParams?.sparseWeight,
  });

  throwOnError(response, `Collection '${collection}'`);

  return response.data?.results ?? [];
}

/** Search a named sparse index, optionally combined with a dense vector. */
export async function sparseSearchNamed(
  transport: SearchTransport,
  collection: string,
  query: SparseVector,
  indexName: string,
  options?: SparseSearchNamedOptions
): Promise<SearchResult[]> {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const body: Record<string, any> = {
    sparse_vectors: { [indexName]: transport.sparseToRest(query) },
    sparse_index: indexName,
    top_k: options?.k ?? 10,
    filter: options?.filter,
    ...searchQualityToMode(options?.quality),
  };

  if (options?.vector) {
    body.vector = Array.from(options.vector);
  }

  const response = await transport.requestJson<{ results: SearchResult[] }>(
    'POST',
    `${collectionPath(collection)}/search`,
    body
  );

  throwOnError(response, `Collection '${collection}'`);

  return response.data?.results ?? [];
}

export async function searchIds(
  transport: SearchTransport,
  collection: string,
  query: number[] | Float32Array,
  options?: SearchOptions
): Promise<Array<{ id: number; score: number }>> {
  const queryVector = toNumberArray(query);

  const response = await transport.requestJson<{
    results: Array<{ id: number; score: number }>;
  }>(
    'POST',
    `${collectionPath(collection)}/search/ids`,
    {
      vector: queryVector,
      top_k: options?.k ?? 10,
      filter: options?.filter,
      ...searchQualityToMode(options?.quality),
    }
  );

  throwOnError(response, `Collection '${collection}'`);

  return response.data?.results ?? [];
}
