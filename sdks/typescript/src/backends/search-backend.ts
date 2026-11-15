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
  BatchSearchItem,
  HybridSearchOptions,
  SearchFusionOptions,
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

/** Map {@link SearchFusionOptions} to the `FusionRequest` wire (snake_case). */
function fusionToWire(fusion: SearchFusionOptions): Record<string, unknown> {
  return {
    strategy: fusion.strategy,
    k: fusion.k,
    dense_w: fusion.denseWeight,
    sparse_w: fusion.sparseWeight,
    avg_w: fusion.avgWeight,
    max_w: fusion.maxWeight,
    hit_w: fusion.hitWeight,
  };
}

/**
 * Build a full `SearchRequest` body (`api_types/requests.rs`) from the option
 * bag: every field the server accepts is forwarded — `top_k`, `mode` (from
 * `quality`), `ef_search`, `timeout_ms`, `filter`, `sparse_vector`,
 * `sparse_vectors`, `sparse_index`, and `fusion`.
 */
function searchBody(
  transport: SearchTransport,
  vector: number[] | Float32Array | undefined,
  options?: SearchOptions | BatchSearchItem
): Record<string, unknown> {
  const body: Record<string, unknown> = {
    top_k: options?.k ?? 10,
    filter: normalizeFilter(options?.filter),
    ...searchQualityToMode(options?.quality),
  };
  if (vector !== undefined) {
    body.vector = toNumberArray(vector);
  }
  if (options?.efSearch !== undefined) {
    body.ef_search = options.efSearch;
  }
  if (options?.timeoutMs !== undefined) {
    body.timeout_ms = options.timeoutMs;
  }
  if (options?.sparseVector) {
    body.sparse_vector = transport.sparseToRest(options.sparseVector);
  }
  if (options?.sparseVectors) {
    const named: Record<string, Record<string, number>> = {};
    for (const [indexName, sv] of Object.entries(options.sparseVectors)) {
      named[indexName] = transport.sparseToRest(sv);
    }
    body.sparse_vectors = named;
  }
  if (options?.sparseIndexName) {
    body.sparse_index = options.sparseIndexName;
  }
  if (options?.fusion) {
    body.fusion = fusionToWire(options.fusion);
  }
  return body;
}

export async function search(
  transport: SearchTransport,
  collection: string,
  query: number[] | Float32Array,
  options?: SearchOptions
): Promise<SearchResult[]> {
  const response = await transport.requestJson<{ results: SearchResult[] }>(
    'POST',
    `${collectionPath(collection)}/search`,
    searchBody(transport, query, options)
  );

  throwOnError(response, `Collection '${collection}'`);

  return response.data?.results ?? [];
}

export async function searchBatch(
  transport: SearchTransport,
  collection: string,
  searches: BatchSearchItem[]
): Promise<SearchResult[][]> {
  // Each entry is a full SearchRequest server-side, so forward every field.
  const formattedSearches = searches.map((s) => searchBody(transport, s.vector, s));

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
  options?: HybridSearchOptions
): Promise<SearchResult[]> {
  const body: Record<string, unknown> = {
    vector: toNumberArray(vector),
    query: textQuery,
    top_k: options?.k ?? 10,
    vector_weight: options?.vectorWeight ?? 0.5,
    filter: normalizeFilter(options?.filter),
  };
  // `rrf_k` comes from HybridSearchRequestWithRrf (flattened on the wire).
  if (options?.rrfK !== undefined) {
    body.rrf_k = options.rrfK;
  }

  const response = await transport.requestJson<{ results: SearchResult[] }>(
    'POST',
    `${collectionPath(collection)}/search/hybrid`,
    body
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
    branch_weights: options?.fusionParams?.branchWeights,
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
  const body = searchBody(transport, options?.vector, {
    k: options?.k,
    filter: options?.filter,
    quality: options?.quality,
    efSearch: options?.efSearch,
    timeoutMs: options?.timeoutMs,
    sparseVectors: { [indexName]: query },
    sparseIndexName: indexName,
  });

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
  const response = await transport.requestJson<{
    results: Array<{ id: number; score: number }>;
  }>(
    'POST',
    `${collectionPath(collection)}/search/ids`,
    searchBody(transport, query, options)
  );

  throwOnError(response, `Collection '${collection}'`);

  return response.data?.results ?? [];
}
