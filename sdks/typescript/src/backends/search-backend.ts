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
import { normalizeFilter } from '../filter';
import { VelesDBError } from '../types';
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

  const body: Record<string, unknown> = {
    vector: queryVector,
    top_k: options?.k ?? 10,
    filter: normalizeFilter(options?.filter),
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
    filter: normalizeFilter(s.filter),
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
      filter: normalizeFilter(options?.filter),
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
      filter: normalizeFilter(options?.filter),
    }
  );

  throwOnError(response, `Collection '${collection}'`);

  return response.data?.results ?? [];
}

/**
 * How many query vectors core's multi-query search takes
 * (`validate_multi_query_inputs`, `crates/velesdb-core/src/collection/search/batch.rs`).
 * The WASM backend holds the same bound in `wasm-search.ts`.
 */
const MULTI_QUERY_VECTORS = { min: 1, max: 10 } as const;

/**
 * Refuse a vector count core would refuse, before a request leaves: the
 * server answers an out-of-range count with a 400 caller error, and the WASM
 * backend with `BAD_REQUEST`, so a direct call on this backend must not send
 * it on to fail there.
 */
function requireMultiQueryVectorCount(
  vectors: Array<number[] | Float32Array>
): void {
  // `vectors` is typed, but an untyped caller can hand in anything: count a
  // non-array as none rather than read `.length` off it.
  const count = Array.isArray(vectors) ? vectors.length : 0;
  if (count < MULTI_QUERY_VECTORS.min || count > MULTI_QUERY_VECTORS.max) {
    throw new VelesDBError(
      `a multi-query search takes ${MULTI_QUERY_VECTORS.min} to ${MULTI_QUERY_VECTORS.max} ` +
        `vectors, as core's does; got ${Array.isArray(vectors) ? count : `a value of type ${typeof vectors}`}`,
      'BAD_REQUEST'
    );
  }
}

/**
 * The request body both multi-query endpoints send.
 *
 * `/search/multi` and `/search/multi/ids` read the same fusion request; only
 * the path and the result shape differ. Written out twice, the two copies
 * drifted — `filter` was added to one of them alone — so the body is built
 * here once and each endpoint posts it.
 */
function multiQueryBody(
  vectors: Array<number[] | Float32Array>,
  options: MultiQuerySearchOptions | undefined
): Record<string, unknown> {
  return {
    vectors: vectors.map(toNumberArray),
    top_k: options?.k ?? 10,
    strategy: options?.fusion ?? 'rrf',
    rrf_k: options?.fusionParams?.k ?? 60,
    avg_weight: options?.fusionParams?.avgWeight,
    max_weight: options?.fusionParams?.maxWeight,
    hit_weight: options?.fusionParams?.hitWeight,
    dense_weight: options?.fusionParams?.denseWeight,
    sparse_weight: options?.fusionParams?.sparseWeight,
    filter: normalizeFilter(options?.filter),
  };
}

export async function multiQuerySearch(
  transport: SearchTransport,
  collection: string,
  vectors: Array<number[] | Float32Array>,
  options?: MultiQuerySearchOptions
): Promise<SearchResult[]> {
  requireMultiQueryVectorCount(vectors);
  const response = await transport.requestJson<{ results: SearchResult[] }>(
    'POST',
    `${collectionPath(collection)}/search/multi`,
    multiQueryBody(vectors, options)
  );

  throwOnError(response, `Collection '${collection}'`);

  return response.data?.results ?? [];
}

/**
 * Multi-query fusion search returning only IDs and scores (no payloads).
 *
 * Lighter than {@link multiQuerySearch} when payloads are not needed — the
 * server skips payload hydration. Its ids-only kernel takes no metadata
 * filter: a `filter` is sent on, so the server refuses it (`400`) instead of
 * the SDK dropping it. Use {@link multiQuerySearch} for filtered fusion.
 */
export async function multiQuerySearchIds(
  transport: SearchTransport,
  collection: string,
  vectors: Array<number[] | Float32Array>,
  options?: MultiQuerySearchOptions
): Promise<Array<{ id: number; score: number }>> {
  requireMultiQueryVectorCount(vectors);
  const response = await transport.requestJson<{
    results: Array<{ id: number; score: number }>;
  }>(
    'POST',
    `${collectionPath(collection)}/search/multi/ids`,
    multiQueryBody(vectors, options)
  );

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
  const body: Record<string, unknown> = {
    sparse_vectors: { [indexName]: transport.sparseToRest(query) },
    sparse_index: indexName,
    top_k: options?.k ?? 10,
    filter: normalizeFilter(options?.filter),
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
      filter: normalizeFilter(options?.filter),
      ...searchQualityToMode(options?.quality),
    }
  );

  throwOnError(response, `Collection '${collection}'`);

  return response.data?.results ?? [];
}
