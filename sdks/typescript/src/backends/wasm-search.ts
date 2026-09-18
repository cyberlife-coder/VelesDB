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
  FusionParams,
  FusionParamName,
  FusionStrategy,
  SearchQuality,
} from '../types';
import type { FilterInput } from '../filter';
import { NotFoundError, VelesDBError } from '../types';
import { wasmNotSupported } from './shared';
import {
  isSet,
  requireWasmCapability,
  requireWasmFieldsListed,
  requireWasmFilterSupport,
} from './wasm-capability-guards';
import { describeWasmThrow } from './wasm-helpers';
import { sparseHits } from './wasm-sparse';
import type {
  CollectionData,
  WasmContext,
  WasmDenseResult,
  WasmSparseResult,
  WasmFilteredResult,
  WasmHybridResult,
  WasmSearchResultItem,
} from './wasm-types';

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
  return sparseHits(collection!.sparseIds, indices, values, k).map(
    ([id, score]) => ({
      id: String(id),
      score,
      payload: collection!.payloads.get(ctx.canonicalPayloadKeyFromResultId(id)),
    })
  );
}

function searchHybridFusion(
  ctx: WasmContext,
  collection: ReturnType<WasmContext['getCollection']>,
  queryVector: Float32Array,
  indices: number[],
  values: number[],
  k: number,
  quality: SearchQuality
): SearchResult[] {
  const denseResults: WasmDenseResult[] = collection!.store.search_with_quality(
    queryVector, k, quality
  );
  const denseForFuse: Array<[number, number]> = denseResults.map(
    ([id, score]) => [Number(id), score]
  );
  const sparseForFuse = sparseHits(collection!.sparseIds, indices, values, k);

  const fused: WasmSparseResult[] = ctx.wasmModule.hybrid_search_fuse(
    denseForFuse, sparseForFuse, 60, k
  );

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
  filter: FilterInput
): SearchResult[] {
  const results: WasmFilteredResult[] = collection!.store.search_with_filter(
    queryVector, k, filter
  );

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
  k: number,
  quality: SearchQuality
): SearchResult[] {
  const rawResults: WasmDenseResult[] = collection!.store.search_with_quality(
    queryVector, k, quality
  );

  return rawResults.map(([id, score]) => {
    const result: SearchResult = { id: String(id), score };
    const payload = collection!.payloads.get(ctx.canonicalPayloadKeyFromResultId(id));
    if (payload) {
      result.payload = payload;
    }
    return result;
  });
}

/**
 * The preset a dense WASM search runs under when the caller names none.
 *
 * WASM search is brute force, so every preset scans the same vectors; the
 * binding still parses the string, and `balanced` is the one core treats as
 * the default.
 */
const DEFAULT_SEARCH_QUALITY: SearchQuality = 'balanced';

/**
 * Refuse a `quality` preset the binding's own parser refuses.
 *
 * `parse_search_quality` (velesdb-wasm) is the single implementation of the
 * preset grammar — `fast`/`balanced`/`accurate`/`perfect`/`autotune`,
 * `custom:<ef>`, `adaptive:<min>:<max>`. Restating it here would be a second
 * copy free to drift from it, so the check delegates to the binding, which
 * exposes the parser on `search_with_quality` alone. A metadata-only store
 * holds no vectors and has dimension 0, so it parses the preset and scans
 * nothing: every search path is refused alike, including the filtered and
 * sparse-only paths, which have no quality-taking binding method, and a
 * `k <= 0` search, which runs no search at all.
 *
 * An untyped (JavaScript) caller can pass any value here, which is why the
 * refusal cannot be left to the type.
 *
 * The binding's own refusal is a bare string ({@link describeWasmThrow}), so
 * letting it escape would hand the caller something no `instanceof` narrows
 * — the opposite of what this SDK promises. It is re-raised as a
 * `VelesDBError` carrying the binding's words verbatim.
 */
function requireParsableQuality(ctx: WasmContext, quality: unknown): void {
  if (!isSet(quality)) {
    return;
  }
  const probe = ctx.wasmModule.VectorStore.new_metadata_only();
  try {
    probe.search_with_quality(new Float32Array(0), 0, quality as SearchQuality);
  } catch (thrown) {
    throw new VelesDBError(
      `WASM backend: ${describeWasmThrow(thrown)}`,
      'BAD_REQUEST'
    );
  } finally {
    probe.free();
  }
}

/**
 * Refuse the `SearchOptions` this backend cannot apply. `quality` is applied
 * — handed to `search_with_quality` — but has nothing to tune: WASM search
 * scans every stored vector, with no graph index whose recall a preset would
 * trade for speed. What it must not do is pass unread, so an unparseable
 * preset is refused here as the REST server refuses it.
 */
function refuseUnhonouredSearchOptions(options: SearchOptions | undefined): void {
  if (options?.includeVectors === true) {
    requireWasmCapability('includeVectors', 'search with includeVectors: true');
  }
  if (isSet(options?.sparseIndexName)) {
    requireWasmCapability('namedSparseIndexes', 'search with a sparseIndexName');
  }
  if (options?.sparseVector) {
    requireWasmCapability('sparseSearch', 'search with a sparseVector');
    requireWasmFilterSupport('sparseSearch', options.filter);
  } else {
    requireWasmFilterSupport('search', options?.filter);
  }
}

/**
 * How many query vectors core's multi-query search takes
 * (`validate_multi_query_inputs`, `crates/velesdb-core/src/collection/search/batch.rs`).
 */
const MULTI_QUERY_VECTORS = { min: 1, max: 10 } as const;

/**
 * The largest integer velesdb-wasm's `usize` carries: the binding is built for
 * wasm32, where `usize` is 32-bit and a larger `k` wraps modulo 2^32. It is
 * also the largest `u32`, the type of REST's `rrf_k`.
 */
const LARGEST_U32 = 2 ** 32 - 1;

/**
 * Name a refused value in a message without coercing it: a number by its
 * value, anything else by its type. `String()` and template interpolation
 * throw on an object with no prototype.
 */
function describeValue(value: unknown): string {
  return typeof value === 'number' ? String(value) : `a value of type ${typeof value}`;
}

/**
 * `value` as a number, or `BAD_REQUEST` naming its type. The binding would
 * coerce a string, and throw a `TypeError` on an object with no prototype,
 * where REST's JSON number refuses both.
 */
function requireNumber(name: string, value: unknown): number {
  if (typeof value !== 'number') {
    throw new VelesDBError(`${name} must be a number; got ${describeValue(value)}`, 'BAD_REQUEST');
  }
  return value;
}

/**
 * `value` as a finite number, or `BAD_REQUEST`: what REST's `f32` fields
 * take. The REST backend's `JSON.stringify` sends NaN and ±Infinity as
 * `null`, which such a field refuses, so this backend refuses them too
 * rather than hand them to the binding.
 */
function requireFiniteNumber(name: string, value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw new VelesDBError(
      `${name} must be a finite number; got ${describeValue(value)}`,
      'BAD_REQUEST'
    );
  }
  return value;
}

/**
 * `value` as an integer from 0 to 2^32 - 1, or `BAD_REQUEST`; `bound` names
 * the type that bounds it.
 */
function requireU32(name: string, value: unknown, bound: string): number {
  const n = requireNumber(name, value);
  if (!Number.isInteger(n) || n < 0 || n > LARGEST_U32) {
    throw new VelesDBError(
      `${name} must be an integer from 0 to ${LARGEST_U32} (${bound}); got ${describeValue(n)}`,
      'BAD_REQUEST'
    );
  }
  return n;
}

/**
 * Validate a search's inputs before anything else, as core does
 * (`validated_hybrid_params`, `validate_multi_query_inputs`): the number of
 * query vectors, when the search bounds it; every vector's dimension; and
 * `k`, a non-negative integer as core's `usize` is, at most 2^32 - 1 since
 * velesdb-wasm's `usize` is 32-bit. Returns `k`; at 0 the caller answers
 * with no results and never calls the binding. Every search runs it first,
 * so no early return can skip it.
 */
function validateSearchInputs(
  collection: CollectionData,
  vectors: ReadonlyArray<ArrayLike<number>>,
  k: unknown,
  vectorCount?: { readonly min: number; readonly max: number }
): number {
  if (vectorCount && (vectors.length < vectorCount.min || vectors.length > vectorCount.max)) {
    throw new VelesDBError(
      `a multi-query search takes ${vectorCount.min} to ${vectorCount.max} vectors, as core's ` +
        `does; got ${vectors.length}`,
      'BAD_REQUEST'
    );
  }
  const dimension = collection.config.dimension ?? 0;
  for (const vector of vectors) {
    if (vector.length !== dimension) {
      throw new VelesDBError(
        `Query dimension mismatch: expected ${dimension}, got ${vector.length}`,
        'DIMENSION_MISMATCH'
      );
    }
  }
  return requireU32('k', k, "core's usize, 32-bit in velesdb-wasm");
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
  const k = validateSearchInputs(collection, [queryVector], options?.k ?? 10);
  refuseUnhonouredSearchOptions(options);
  requireParsableQuality(ctx, options?.quality);
  const quality = options?.quality ?? DEFAULT_SEARCH_QUALITY;
  if (k <= 0) {
    return [];
  }

  if (options?.sparseVector) {
    const { indices, values } = ctx.sparseVectorToArrays(options.sparseVector);
    const hasDense = queryVector.length > 0
      && collection.config.dimension !== undefined
      && collection.config.dimension > 0;

    return hasDense
      ? searchHybridFusion(ctx, collection, queryVector, indices, values, k, quality)
      : searchSparseOnly(ctx, collection, indices, values, k);
  }

  if (options?.filter) {
    return searchWithFilter(ctx, collection, queryVector, k, options.filter);
  }

  return searchDenseOnly(ctx, collection, queryVector, k, quality);
}

export async function wasmSearchBatch(
  ctx: WasmContext,
  collectionName: string,
  searches: Array<{
    vector: number[] | Float32Array;
    k?: number;
    filter?: FilterInput;
    /**
     * Search quality preset, refused in the pre-loop below if the binding
     * cannot parse it, then forwarded to `wasmSearch`. It has nothing to
     * tune there: WASM search scans every stored vector, which meets the
     * recall of any preset.
     */
    quality?: import('../types').SearchQuality;
  }>
): Promise<SearchResult[][]> {
  const collection = ctx.getCollection(collectionName);
  if (!collection) {
    throw new NotFoundError(`Collection '${collectionName}'`);
  }
  // One pre-loop refuses every entry's unhonourable option before any entry
  // searches: a batch runs whole or not at all. `wasmSearch` checks the same
  // things again per entry, which is what makes a single search safe; what it
  // cannot do is speak for the entries after it.
  for (const s of searches) {
    validateSearchInputs(collection, [s.vector], s.k ?? 10);
    requireWasmFilterSupport('searchBatch', s.filter);
    requireParsableQuality(ctx, s.quality);
  }
  const results: SearchResult[][] = [];
  for (const s of searches) {
    results.push(
      await wasmSearch(ctx, collectionName, s.vector, {
        k: s.k,
        filter: s.filter,
        quality: s.quality,
      })
    );
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
  r: WasmSearchResultItem
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
  options?: { k?: number; filter?: FilterInput }
): Promise<SearchResult[]> {
  const collection = ctx.getCollection(collectionName);
  if (!collection) {
    throw new NotFoundError(`Collection '${collectionName}'`);
  }
  const k = validateSearchInputs(collection, [], options?.k ?? 10);
  requireWasmFilterSupport('textSearch', options?.filter);
  if (k <= 0) {
    return [];
  }
  // The binding's third argument names one payload field to match. It is
  // not a filter, which is why a filter is refused above.
  const raw: WasmSearchResultItem[] = collection.store.text_search(query, k, null);
  return raw.map(r => mapWasmResult(ctx, collection, r));
}

export async function wasmHybridSearch(
  ctx: WasmContext,
  collectionName: string,
  vector: number[] | Float32Array,
  textQuery: string,
  options?: { k?: number; vectorWeight?: number; filter?: FilterInput }
): Promise<SearchResult[]> {
  const collection = ctx.getCollection(collectionName);
  if (!collection) {
    throw new NotFoundError(`Collection '${collectionName}'`);
  }
  const queryVector = vector instanceof Float32Array ? vector : new Float32Array(vector);
  const k = validateSearchInputs(collection, [queryVector], options?.k ?? 10);
  requireWasmFilterSupport('hybridSearch', options?.filter);
  const vectorWeight = requireFiniteNumber('vectorWeight', options?.vectorWeight ?? 0.5);
  if (k <= 0) {
    return [];
  }
  const raw: WasmHybridResult[] = collection.store.hybrid_search(
    queryVector, textQuery, k, vectorWeight
  );
  return raw.map(r => {
    const key = ctx.canonicalPayloadKeyFromResultId(r.id);
    return { id: String(r.id), score: r.score, payload: r.payload ?? collection.payloads.get(key) };
  });
}

// ---------------------------------------------------------------------------
// Multi-query search
// ---------------------------------------------------------------------------

/** The weighted-fusion fields velesdb-wasm takes as one `[avg, max, hit]` argument. */
const WEIGHTED_TRIPLE = ['avgWeight', 'maxWeight', 'hitWeight'] as const;

/** Every `fusionParams` weight: each an `f32` field of REST's `MultiQuerySearchRequest`. */
const FUSION_WEIGHTS = [...WEIGHTED_TRIPLE, 'denseWeight', 'sparseWeight'] as const;

/**
 * The `fusionParams` fields each strategy reads, as core's builders read
 * them (velesdb-server's `build_fusion_strategy`, velesdb-wasm's
 * `fuse_results`). A field the chosen strategy does not read is ignored, as
 * core ignores it; one it reads and this backend cannot apply is refused.
 */
const FUSION_PARAMS_READ: Readonly<Record<FusionStrategy, readonly FusionParamName[]>> = {
  rrf: ['k'],
  weighted: WEIGHTED_TRIPLE,
  relative_score: ['denseWeight', 'sparseWeight'],
  average: [],
  maximum: [],
};

/**
 * The strategy names core accepts, as velesdb-wasm's `fuse_results` and
 * velesdb-server's `build_fusion_strategy` read them: lowercased first, with
 * these aliases.
 */
const FUSION_STRATEGY_NAMES: ReadonlyMap<string, FusionStrategy> = new Map([
  ['rrf', 'rrf'],
  ['average', 'average'],
  ['avg', 'average'],
  ['maximum', 'maximum'],
  ['max', 'maximum'],
  ['weighted', 'weighted'],
  ['relative_score', 'relative_score'],
  ['rsf', 'relative_score'],
]);

/**
 * The canonical strategy `name` stands for, as core reads it. `name` comes
 * from the caller unchecked, and an untyped (JavaScript) caller can pass any
 * value: one that is not a string, or a name core does not know, is refused.
 * The refusal names a value that is not a string by its type, never by
 * coercing it: `String()` throws on an object with no prototype.
 */
function canonicalStrategy(name: unknown): FusionStrategy {
  const strategy =
    typeof name === 'string' ? FUSION_STRATEGY_NAMES.get(name.toLowerCase()) : undefined;
  if (strategy === undefined) {
    const named = typeof name === 'string' ? `'${name}'` : `of type ${typeof name}`;
    throw new VelesDBError(
      `Unknown fusion strategy ${named}: core accepts average (avg), maximum (max), ` +
        'rrf, weighted and relative_score (rsf), in any case',
      'BAD_REQUEST'
    );
  }
  return strategy;
}

/**
 * How far from 1.0 a weighted triple may sum: core's `validate_weight_sum`
 * (`crates/velesdb-core/src/fusion/strategy.rs`), an f32 `0.001`. The
 * binding checks it again, but reports a failure as a bare string.
 */
const WEIGHTED_SUM_TOLERANCE = Math.fround(0.001);

/**
 * Refuse, as core does, a weighted triple with a negative weight, one that
 * is not finite once rounded to f32, or one that does not sum to 1.0.
 * `wasmFusionArgs` has already refused a weight that is not a finite number.
 *
 * The check runs in f32, as core's `validate_non_negative` and
 * `validate_weight_sum` do on the `Float32Array` the binding receives: each
 * weight rounded to f32, summed `(avg + max) + hit` with every step rounded,
 * then `|sum - 1| > 0.001`. In f64 the two disagree both ways near the
 * tolerance: `[0.5, 0.5, 0.001]` sums to 1.0010000467 in f32, and
 * `[0.3, 0.3, 0.399]` to 0.9990000129.
 */
function validateWeightedTriple(weights: readonly number[]): void {
  const f32 = weights.map((weight) => Math.fround(weight));
  const sum = f32.reduce((total, weight) => Math.fround(total + weight), 0);
  const invalid = f32.some((weight) => !Number.isFinite(weight) || weight < 0);
  if (invalid || Math.abs(Math.fround(sum - 1)) > WEIGHTED_SUM_TOLERANCE) {
    throw new VelesDBError(
      'multiQuerySearch weighted fusion: avgWeight, maxWeight and hitWeight must be ' +
        `finite, non-negative and sum to 1.0 within ${WEIGHTED_SUM_TOLERANCE}; ` +
        `got ${weights.join(', ')}`,
      'BAD_REQUEST'
    );
  }
}

/**
 * Translate `fusionParams` into velesdb-wasm's `multi_query_search`
 * arguments, refusing what the binding cannot apply.
 *
 * Only the fields the strategy reads count (`FUSION_PARAMS_READ`): the
 * weighted triple under `weighted`, the dense/sparse weights under
 * `relative_score`. Under `weighted`, the three weights travel as one
 * argument, and the binding applies core's defaults only when that argument
 * is absent: a partial triple is therefore refused rather than completed
 * with guessed values, and a complete one is checked against core's rule.
 *
 * Every field given is checked for its type first, whichever strategy reads
 * it: REST deserializes the whole request before it reads the strategy, so a
 * `k` that is not a `u32`, or a weight that is not a finite `f32`, refuses
 * the request even where the strategy would ignore the field. A weight
 * given as `null` counts as given: the REST backend sends it as JSON
 * `null`, which an `f32` field refuses. (`k` differs: both backends read a
 * `null` `k` as absent, 60.)
 */
function wasmFusionArgs(
  strategy: FusionStrategy,
  params: FusionParams | undefined
): {
  rrfK: number;
  weights: Float32Array | null;
} {
  const rrfK = requireU32('fusionParams.k', params?.k ?? 60, "core's rrf_k, a u32");
  for (const name of FUSION_WEIGHTS) {
    const weight = params?.[name];
    if (weight !== undefined) {
      requireFiniteNumber(`fusionParams.${name}`, weight);
    }
  }
  const read: FusionParams = {};
  for (const name of FUSION_PARAMS_READ[strategy]) {
    read[name] = params?.[name];
  }
  requireWasmFieldsListed('multiQueryFusionParams', 'multiQuerySearch fusionParams', read);
  const weights = WEIGHTED_TRIPLE.map((name) => read[name]).filter(isSet);
  if (weights.length === 0) {
    return { rrfK, weights: null };
  }
  if (weights.length !== WEIGHTED_TRIPLE.length) {
    wasmNotSupported(
      "multiQuerySearch under 'weighted' with only some of fusionParams avgWeight, maxWeight " +
        "and hitWeight (capability 'multiQueryFusionParams' lists the three, which velesdb-wasm " +
        'takes together)'
    );
  }
  validateWeightedTriple(weights);
  return { rrfK, weights: new Float32Array(weights) };
}

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
  const k = validateSearchInputs(collection, vectors, options?.k ?? 10, MULTI_QUERY_VECTORS);
  requireWasmFilterSupport('multiQuerySearch', options?.filter);
  const strategy = canonicalStrategy(options?.fusion ?? 'rrf');
  const { rrfK, weights } = wasmFusionArgs(strategy, options?.fusionParams);
  if (k <= 0) {
    return [];
  }

  const numVectors = vectors.length;
  const dimension = collection.config.dimension ?? 0;
  const flat = new Float32Array(numVectors * dimension);
  vectors.forEach((vector, idx) => {
    const src = vector instanceof Float32Array ? vector : new Float32Array(vector);
    flat.set(src, idx * dimension);
  });

  const raw: WasmSearchResultItem[] = collection.store.multi_query_search(
    flat,
    numVectors,
    k,
    strategy,
    rrfK,
    weights
  );

  return raw.map(r => mapWasmResult(ctx, collection, r));
}

// ---------------------------------------------------------------------------
// Query (VelesQL over WASM)
// ---------------------------------------------------------------------------

/**
 * The only VelesQL shape the WASM backend can execute faithfully: a pure
 * top-k NEAR scan — `SELECT * FROM <collection> WHERE vector NEAR $param
 * [LIMIT n]` (case-insensitive, optional trailing semicolon).
 *
 * `vector` is the literal keyword from the grammar
 * (`vector_search = { ^"vector" ~ ^"NEAR" ~ vector_value }`), not a column
 * name — any other identifier left of NEAR is a parse error on
 * velesdb-server, so it must be rejected here too or the query would work
 * in WASM and break on REST.
 *
 * `VectorStore.query()` is a brute-force k-NN that evaluates no other
 * clause. Anything else (WHERE predicates, JOIN, GROUP BY, MATCH, set
 * operations, FUSION, …) must be rejected loudly instead of silently
 * dropping clauses and returning unfiltered neighbours.
 */
const PURE_NEAR_QUERY =
  /^\s*select\s+\*\s+from\s+([a-z_]\w*)\s+where\s+vector\s+near\s+\$([a-z_]\w*)\s*(?:limit\s+(\d+))?\s*;?\s*$/i;

interface PureNearQuery {
  /** Collection named in the FROM clause. */
  from: string;
  /** Name of the `$param` holding the query embedding. */
  param: string;
  /** `LIMIT n` value when present. */
  limit?: number;
}

/** The largest `LIMIT` core's parser takes: it reads the value as a u64 (`velesql/parser/helpers.rs`). */
const LARGEST_U64 = 2n ** 64n - 1n;

/** Parse `queryString` against the pure-NEAR shape or throw `NOT_SUPPORTED`. */
function parsePureNearQuery(queryString: string): PureNearQuery {
  const match = PURE_NEAR_QUERY.exec(queryString);
  if (!match) {
    throw new VelesDBError(
      'The WASM backend only executes pure top-k NEAR queries of the form ' +
        '"SELECT * FROM <collection> WHERE vector NEAR $param [LIMIT n]". ' +
        'WHERE predicates, JOIN, GROUP BY, MATCH, set operations and FUSION ' +
        'are not evaluated in WASM — use the REST backend (velesdb-server) ' +
        `for full VelesQL. Received: ${queryString}`,
      'NOT_SUPPORTED'
    );
  }
  const parsed: PureNearQuery = { from: match[1]!, param: match[2]! };
  if (match[3] !== undefined) {
    if (BigInt(match[3]) > LARGEST_U64) {
      throw new VelesDBError(
        `Invalid LIMIT value '${match[3]}': core's parser reads LIMIT as a u64`,
        'BAD_REQUEST'
      );
    }
    parsed.limit = Number(match[3]);
  }
  return parsed;
}

/**
 * The rows a statement without `LIMIT` returns: core's `DEFAULT_SELECT_LIMIT`
 * (`crates/velesdb-core/src/velesql/ast/select.rs`). Core reads no `k` from a
 * query's params, so neither does this backend: on REST, `{ k: 0 }` is only
 * an unused parameter.
 */
const DEFAULT_SELECT_LIMIT = 10;

/**
 * The most rows a statement returns: core's `MAX_LIMIT`, which caps `LIMIT`
 * in `compute_fetch_limit`
 * (`crates/velesdb-core/src/collection/search/query/query_pipeline.rs`).
 */
const MAX_LIMIT = 100_000;

export async function wasmQuery(
  ctx: WasmContext,
  collectionName: string,
  queryString: string,
  params?: Record<string, unknown>,
  options?: QueryOptions
): Promise<QueryApiResponse> {
  const collection = ctx.getCollection(collectionName);
  if (!collection) {
    throw new NotFoundError(`Collection '${collectionName}'`);
  }
  requireWasmFieldsListed('queryOptions', 'query', options);
  const parsed = parsePureNearQuery(queryString);
  if (parsed.from !== collectionName) {
    throw new VelesDBError(
      `Query targets collection '${parsed.from}' but was executed against '${collectionName}'.`,
      'BAD_REQUEST'
    );
  }
  const paramsVector = params?.[parsed.param];
  if (!Array.isArray(paramsVector) && !(paramsVector instanceof Float32Array)) {
    throw new VelesDBError(
      `WASM query() expects params.${parsed.param} to contain the query embedding vector.`,
      'BAD_REQUEST'
    );
  }
  const queryVector =
    paramsVector instanceof Float32Array ? paramsVector : new Float32Array(paramsVector);
  const k = validateSearchInputs(
    collection,
    [queryVector],
    Math.min(parsed.limit ?? DEFAULT_SELECT_LIMIT, MAX_LIMIT)
  );
  // `LIMIT 0` asks for no rows: there is nothing to ask the binding.
  const raw: Record<string, unknown>[] = k <= 0 ? [] : collection.store.query(queryVector, k);

  return {
    results: raw,
    stats: {
      executionTimeMs: 0,
      strategy: 'wasm-query',
      scannedNodes: raw.length,
    },
  };
}
