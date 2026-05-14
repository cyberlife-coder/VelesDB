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
}

/** Search options */
export interface SearchOptions {
  /** Number of results to return (default: 10) */
  k?: number;
  /** Filter expression (optional). Accepts typed `Filter` (recommended) or legacy raw JSON. */
  filter?: FilterInput;
  /** Include vectors in results (default: false) */
  includeVectors?: boolean;
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
   * **Backend support:** REST only. The WASM backend silently ignores this
   * field and uses the collection's single sparse index regardless.
   */
  sparseIndexName?: string;
  /** Search quality preset (default: 'balanced'). */
  quality?: SearchQuality;
}

/** Fusion strategy for multi-query search */
export type FusionStrategy = 'rrf' | 'average' | 'maximum' | 'weighted' | 'relative_score';

/** Multi-query search options */
export interface MultiQuerySearchOptions {
  /** Number of results to return (default: 10) */
  k?: number;
  /** Fusion strategy (default: 'rrf') */
  fusion?: FusionStrategy;
  /** Fusion parameters */
  fusionParams?: {
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
  };
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
  /** Vector data (if includeVectors is true) */
  vector?: number[];
}
