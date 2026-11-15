/**
 * VelesDB TypeScript SDK - Search Type Definitions
 *
 * Search options, results, and fusion types.
 * @packageDocumentation
 */

import type { FilterInput } from '../filter';
import type { SearchQuality, SparseVector } from './core';

/**
 * Options for `db.sparseSearchNamed()` — **pure sparse** query against a
 * named sparse index (issue #380).
 *
 * Use this when you have only a sparse vector and want to query a specific
 * named sparse index directly. The query carries no dense component.
 *
 * For **dense + sparse hybrid** against a named index, use
 * `db.search(..., { sparseVector, sparseIndexName })` instead
 * (see {@link SearchOptions.sparseIndexName}).
 *
 * **Backend support:** REST only. The WASM backend has no concept of named
 * sparse indexes; this method throws `wasmNotSupported` on WASM.
 */
export interface SparseSearchNamedOptions {
  /** Number of results to return (default: 10) */
  k?: number;
  /** Filter expression */
  filter?: FilterInput;
  /** Optional dense vector to combine with sparse for hybrid named search */
  vector?: number[] | Float32Array;
  /** Search quality preset */
  quality?: SearchQuality;
  /** HNSW `ef_search` (REST only). */
  efSearch?: number;
  /** Per-request timeout budget in ms (REST only). */
  timeoutMs?: number;
}

/**
 * Fusion options for a single `/search` request
 * (`SearchRequest.fusion` → `FusionRequest` on the wire).
 */
export interface SearchFusionOptions extends FusionParams {
  /** Fusion strategy. */
  strategy: FusionStrategy;
}

/** Search options */
export interface SearchOptions {
  /** Number of results to return (default: 10) */
  k?: number;
  /** Filter expression (optional). Accepts typed `Filter` (recommended) or legacy raw JSON. */
  filter?: FilterInput;
  /** Optional sparse vector for hybrid sparse+dense search */
  sparseVector?: SparseVector;
  /**
   * Named sparse index to combine with the dense query for **hybrid** search
   * (when the collection has multiple sparse indexes). When omitted, the
   * default sparse index is used.
   *
   * For a **pure sparse** query against a named index (no dense vector),
   * call `db.sparseSearchNamed()` instead — see {@link SparseSearchNamedOptions}.
   *
   * **Backend support:** REST only. A WASM collection has a single sparse
   * index, so the WASM backend refuses this field with `NOT_SUPPORTED`
   * (`capabilities().namedSparseIndexes` is `false`).
   */
  sparseIndexName?: string;
  /**
   * Named sparse vectors map (`SearchRequest.sparse_vectors`), one entry per
   * sparse index. Sends several sparse vectors in one request.
   *
   * **Backend support:** REST only (same reason as `sparseIndexName`).
   */
  sparseVectors?: Record<string, SparseVector>;
  /** Search quality preset (default: 'balanced'). */
  quality?: SearchQuality;
  /**
   * HNSW `ef_search`, in `[16, 4096]` — sent as `ef_search` on the wire. It
   * wins over the `mode` derived from `quality` for a dense search without a
   * filter. The server refuses an out-of-range value with a `400`.
   *
   * **Backend support:** REST only (the WASM binding expresses this through
   * `quality: 'custom:<ef>'`).
   */
  efSearch?: number;
  /**
   * Per-request timeout budget in milliseconds, sent as `timeout_ms`. It can
   * only shorten the server's own limit, never lengthen it; `0` keeps the
   * server limit. Running out answers `408`/`503`.
   *
   * **Backend support:** REST only (there is no server-side budget on WASM).
   */
  timeoutMs?: number;
  /**
   * Fusion configuration sent as `fusion` on `/search`
   * (`SearchRequest.fusion`).
   *
   * **Backend support:** REST only.
   */
  fusion?: SearchFusionOptions;
}

/**
 * One entry of `searchBatch`: a full per-entry `SearchRequest` on the server
 * (`BatchSearchRequest.searches: Vec<SearchRequest>`), so every field of
 * {@link SearchOptions} applies per entry (the server validates but does not
 * apply `mode`/`ef_search` for batch entries).
 */
export interface BatchSearchItem {
  /** Query vector for this entry. */
  vector: number[] | Float32Array;
  /** Number of results to return for this entry (default: 10). */
  k?: number;
  /** Filter expression (optional). */
  filter?: FilterInput;
  /** Search quality preset. */
  quality?: SearchQuality;
  /** HNSW `ef_search` (REST only). */
  efSearch?: number;
  /** Per-request timeout budget in ms (REST only). */
  timeoutMs?: number;
  /** Sparse vector for hybrid search. */
  sparseVector?: SparseVector;
  /** Named sparse index (REST only). */
  sparseIndexName?: string;
  /** Named sparse vectors map (REST only). */
  sparseVectors?: Record<string, SparseVector>;
  /** Fusion configuration (REST only). */
  fusion?: SearchFusionOptions;
}

/** Options for `hybridSearch` (`/search/hybrid`, `HybridSearchRequestWithRrf`). */
export interface HybridSearchOptions {
  /** Number of results to return (default: 10). */
  k?: number;
  /** Weight for vector similarity, 0.0-1.0 (text weight = 1 - this; default 0.5). */
  vectorWeight?: number;
  /** Filter expression (optional). */
  filter?: FilterInput;
  /**
   * RRF `k` parameter, sent as `rrf_k` (`HybridSearchRequestWithRrf`).
   *
   * **Backend support:** REST only.
   */
  rrfK?: number;
}

/** Fusion strategy for multi-query search */
export type FusionStrategy =
  | 'rrf'
  | 'weighted_rrf'
  | 'average'
  | 'maximum'
  | 'weighted'
  | 'relative_score';

/** Parameters of a multi-query fusion strategy. */
export interface FusionParams {
  /** RRF k parameter (default: 60) */
  k?: number;
  /** Weighted fusion: average weight (default: 0.6) */
  avgWeight?: number;
  /** Weighted fusion: max weight (default: 0.3) */
  maxWeight?: number;
  /** Weighted fusion: hit weight (default: 0.1) */
  hitWeight?: number;
  /** Relative score fusion: dense vector weight (default: 0.5) */
  denseWeight?: number;
  /** Relative score fusion: sparse vector weight (default: 0.5) */
  sparseWeight?: number;
  /**
   * Weighted RRF: one weight per query vector, in vector order. Required
   * under `weighted_rrf` — the one fusion parameter that has no default.
   */
  branchWeights?: number[];
}

/** Name of one {@link FusionParams} field. */
export type FusionParamName = keyof FusionParams;

/** Multi-query search options */
export interface MultiQuerySearchOptions {
  /** Number of results to return (default: 10) */
  k?: number;
  /** Fusion strategy (default: 'rrf') */
  fusion?: FusionStrategy;
  /**
   * Fusion parameters. `db.capabilities().multiQueryFusionParams` lists the
   * fields the backend applies; the WASM backend refuses the others.
   */
  fusionParams?: FusionParams;
  /** Filter expression (optional). Accepts typed `Filter` (recommended) or legacy raw JSON. */
  filter?: FilterInput;
}

/** Search result */
export interface SearchResult {
  /** Document ID */
  id: string | number;
  /** Similarity score */
  score: number;
  /** Document payload (if requested) */
  payload?: Record<string, unknown>;
}

/**
 * Full-text search result (`textSearch`). `score` is the BM25 score on the
 * REST backend and `null` on the WASM backend, whose text search selects by
 * case-insensitive substring and ranks nothing (velesdb-wasm 8.0.0): there
 * is no relevance to threshold or sort on, and none is invented.
 */
export interface TextSearchResult extends Omit<SearchResult, 'score'> {
  /** BM25 score (REST), or `null` when the backend does not rank (WASM). */
  score: number | null;
}
