/**
 * VelesDB TypeScript SDK - Search Type Definitions
 *
 * Search options, results, and fusion types.
 * @packageDocumentation
 */

import type { FilterInput } from '../filter';
import type { SearchQuality, SparseVector } from './core';

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
