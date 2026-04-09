/**
 * WASM Backend — Search & Query Operations
 *
 * Extracted from wasm.ts to keep file NLOC under 500.
 * All functions receive a WasmContext to access collections and the WASM module.
 */

import type {
  SearchOptions,
  SearchResult,
  MultiQuerySearchOptions,
  QueryOptions,
  QueryApiResponse,
} from '../types';
import { NotFoundError, VelesDBError } from '../types';
import type { WasmContext } from './wasm-types';

// ---------------------------------------------------------------------------
// Dense search (optionally with sparse/hybrid/filter)
// ---------------------------------------------------------------------------

function searchSparseOnly(
  ctx: WasmContext,
  collection: ReturnType<WasmContext['getCollection']>,
  indices: number[],
  values: number[],
  k: number
): SearchResult[] {
  const sparseResults = collection!.store.sparse_search(
    new Uint32Array(indices),
    new Float32Array(values),
    k
  ) as Array<{ doc_id: bigint | number; score: number }>;

  return sparseResults.map(r => ({
    id: String(r.doc_id),
    score: r.score,
    payload: collection!.payloads.get(ctx.canonicalPayloadKeyFromResultId(r.doc_id)),
  }));
}

function searchHybridFusion(
  ctx: WasmContext,
  collection: ReturnType<WasmContext['getCollection']>,
  queryVector: Float32Array,
  indices: number[],
  values: number[],
  k: number
): SearchResult[] {
  const denseResults = collection!.store.search(queryVector, k) as Array<[bigint, number]>;
  const sparseResults = collection!.store.sparse_search(
    new Uint32Array(indices),
    new Float32Array(values),
    k
  ) as Array<{ doc_id: bigint | number; score: number }>;

  const denseForFuse = denseResults.map(([id, score]: [bigint, number]) => [Number(id), score]);
  const sparseForFuse = sparseResults.map(
    (r: { doc_id: bigint | number; score: number }) => [Number(r.doc_id), r.score]
  );

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const fused = (ctx.wasmModule as any).hybrid_search_fuse(
    denseForFuse, sparseForFuse, 60
  ) as Array<{ doc_id: bigint | number; score: number }>;

  return fused.slice(0, k).map(r => ({
    id: String(r.doc_id),
    score: r.score,
    payload: collection!.payloads.get(ctx.canonicalPayloadKeyFromResultId(r.doc_id)),
  }));
}

function searchWithFilter(
  ctx: WasmContext,
  collection: ReturnType<WasmContext['getCollection']>,
  queryVector: Float32Array,
  k: number,
  filter: Record<string, unknown>
): SearchResult[] {
  const results = collection!.store.search_with_filter(queryVector, k, filter) as Array<{
    id: bigint;
    score: number;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    payload: any;
  }>;

  return results.map(r => ({
    id: String(r.id),
    score: r.score,
    payload: r.payload || collection!.payloads.get(ctx.canonicalPayloadKeyFromResultId(r.id)),
  }));
}

function searchDenseOnly(
  ctx: WasmContext,
  collection: ReturnType<WasmContext['getCollection']>,
  queryVector: Float32Array,
  k: number
): SearchResult[] {
  const rawResults = collection!.store.search(queryVector, k) as Array<[bigint, number]>;

  return rawResults.map(([id, score]: [bigint, number]) => {
    const result: SearchResult = { id: String(id), score };
    const payload = collection!.payloads.get(ctx.canonicalPayloadKeyFromResultId(id));
    if (payload) {
      result.payload = payload;
    }
    return result;
  });
}

// ---------------------------------------------------------------------------
// Exported search functions
// ---------------------------------------------------------------------------

export async function wasmSearch(
  ctx: WasmContext,
  collectionName: string,
  query: number[] | Float32Array,
  options?: SearchOptions
): Promise<SearchResult[]> {
  const collection = ctx.getCollection(collectionName);
  if (!collection) {
    throw new NotFoundError(`Collection '${collectionName}'`);
  }

  const queryVector = query instanceof Float32Array ? query : new Float32Array(query);
  if (queryVector.length !== collection.config.dimension) {
    throw new VelesDBError(
      `Query dimension mismatch: expected ${collection.config.dimension}, got ${queryVector.length}`,
      'DIMENSION_MISMATCH'
    );
  }

  const k = options?.k ?? 10;

  if (options?.sparseVector) {
    const { indices, values } = ctx.sparseVectorToArrays(options.sparseVector);
    const hasDense = queryVector.length > 0
      && collection.config.dimension !== undefined
      && collection.config.dimension > 0;

    return hasDense
      ? searchHybridFusion(ctx, collection, queryVector, indices, values, k)
      : searchSparseOnly(ctx, collection, indices, values, k);
  }

  if (options?.filter) {
    return searchWithFilter(ctx, collection, queryVector, k, options.filter);
  }

  return searchDenseOnly(ctx, collection, queryVector, k);
}

export async function wasmSearchBatch(
  ctx: WasmContext,
  collectionName: string,
  searches: Array<{
    vector: number[] | Float32Array;
    k?: number;
    filter?: Record<string, unknown>;
  }>
): Promise<SearchResult[][]> {
  const results: SearchResult[][] = [];
  for (const s of searches) {
    results.push(await wasmSearch(ctx, collectionName, s.vector, { k: s.k, filter: s.filter }));
  }
  return results;
}

// ---------------------------------------------------------------------------
// Text / Hybrid search
// ---------------------------------------------------------------------------

/** Map a WASM search result (tuple or object) to a SearchResult. */
function mapWasmResult(
  ctx: WasmContext,
  collection: ReturnType<WasmContext['getCollection']>,
  r: [bigint | number, number] | { id: bigint | number; score: number; payload?: Record<string, unknown> }
): SearchResult {
  if (Array.isArray(r)) {
    const key = ctx.canonicalPayloadKeyFromResultId(r[0]);
    return { id: String(r[0]), score: r[1], payload: collection!.payloads.get(key) };
  }
  const key = ctx.canonicalPayloadKeyFromResultId(r.id);
  return { id: String(r.id), score: r.score, payload: r.payload ?? collection!.payloads.get(key) };
}

export async function wasmTextSearch(
  ctx: WasmContext,
  collectionName: string,
  query: string,
  options?: { k?: number; filter?: Record<string, unknown> }
): Promise<SearchResult[]> {
  const collection = ctx.getCollection(collectionName);
  if (!collection) {
    throw new NotFoundError(`Collection '${collectionName}'`);
  }
  const k = options?.k ?? 10;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const raw = collection.store.text_search(query, k, undefined) as Array<any>;
  return raw.map(r => mapWasmResult(ctx, collection, r));
}

export async function wasmHybridSearch(
  ctx: WasmContext,
  collectionName: string,
  vector: number[] | Float32Array,
  textQuery: string,
  options?: { k?: number; vectorWeight?: number; filter?: Record<string, unknown> }
): Promise<SearchResult[]> {
  const collection = ctx.getCollection(collectionName);
  if (!collection) {
    throw new NotFoundError(`Collection '${collectionName}'`);
  }
  const queryVector = vector instanceof Float32Array ? vector : new Float32Array(vector);
  const k = options?.k ?? 10;
  const vectorWeight = options?.vectorWeight ?? 0.5;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const raw = collection.store.hybrid_search(queryVector, textQuery, k, vectorWeight) as Array<any>;
  return raw.map((r: { id: bigint | number; score: number; payload?: Record<string, unknown> }) => {
    const key = ctx.canonicalPayloadKeyFromResultId(r.id);
    return { id: String(r.id), score: r.score, payload: r.payload ?? collection.payloads.get(key) };
  });
}

// ---------------------------------------------------------------------------
// Multi-query search
// ---------------------------------------------------------------------------

export async function wasmMultiQuerySearch(
  ctx: WasmContext,
  collectionName: string,
  vectors: Array<number[] | Float32Array>,
  options?: MultiQuerySearchOptions
): Promise<SearchResult[]> {
  const collection = ctx.getCollection(collectionName);
  if (!collection) {
    throw new NotFoundError(`Collection '${collectionName}'`);
  }
  if (vectors.length === 0) {
    return [];
  }

  const numVectors = vectors.length;
  const dimension = collection.config.dimension ?? 0;
  const flat = new Float32Array(numVectors * dimension);
  vectors.forEach((vector, idx) => {
    const src = vector instanceof Float32Array ? vector : new Float32Array(vector);
    flat.set(src, idx * dimension);
  });

  const strategy = options?.fusion ?? 'rrf';
  if (strategy === 'weighted') {
    throw new VelesDBError(
      "Fusion strategy 'weighted' is not supported in WASM backend.",
      'NOT_SUPPORTED'
    );
  }
  const raw = collection.store.multi_query_search(
    flat,
    numVectors,
    options?.k ?? 10,
    strategy,
    options?.fusionParams?.k ?? 60
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  ) as Array<any>;

  return raw.map(r => mapWasmResult(ctx, collection, r));
}

// ---------------------------------------------------------------------------
// Query (VelesQL over WASM)
// ---------------------------------------------------------------------------

export async function wasmQuery(
  ctx: WasmContext,
  collectionName: string,
  _queryString: string,
  params?: Record<string, unknown>,
  _options?: QueryOptions
): Promise<QueryApiResponse> {
  const collection = ctx.getCollection(collectionName);
  if (!collection) {
    throw new NotFoundError(`Collection '${collectionName}'`);
  }
  const paramsVector = params?.q;
  if (!Array.isArray(paramsVector) && !(paramsVector instanceof Float32Array)) {
    throw new VelesDBError(
      'WASM query() expects params.q to contain the query embedding vector.',
      'BAD_REQUEST'
    );
  }
  const requestedK = params?.k;
  const k =
    typeof requestedK === 'number' && Number.isInteger(requestedK) && requestedK > 0
      ? requestedK
      : 10;
  const raw = collection.store.query(
    paramsVector instanceof Float32Array ? paramsVector : new Float32Array(paramsVector),
    k
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  ) as Array<any>;

  return {
    results: raw as Record<string, unknown>[],
    stats: {
      executionTimeMs: 0,
      strategy: 'wasm-query',
      scannedNodes: raw.length,
    },
  };
}
