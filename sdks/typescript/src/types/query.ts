/**
 * VelesDB TypeScript SDK - Query & Introspection Type Definitions
 *
 * VelesQL query types, scroll, column stats, EXPLAIN, and collection sanity.
 * @packageDocumentation
 */

import type { FilterInput } from '../filter';
import type { DistanceMetric, StorageMode } from './core';

// ============================================================================
// Scroll Types
// ============================================================================

/** Request parameters for cursor-based scroll pagination. */
export interface ScrollRequest {
  /** Cursor position to resume from. Omit to start from beginning. */
  cursor?: string | number;
  /** Number of points per page (1-10000, default 100). */
  batchSize?: number;
  /** Optional filter expression. Accepts typed `Filter` (recommended) or legacy raw JSON. */
  filter?: FilterInput;
}

/** Response from scroll pagination. */
export interface ScrollResponse {
  /** Points in this page. */
  points: Array<{
    id: string | number;
    vector?: number[];
    payload?: Record<string, unknown>;
  }>;
  /** Cursor for next page, or null if no more results. */
  nextCursor: string | number | null;
}

// ============================================================================
// Column Stats Types
// ============================================================================

/** Per-column statistics including histogram metadata. */
export interface ColumnStatsDetail {
  name: string;
  nullCount: number;
  distinctCount: number;
  minValue: unknown | null;
  maxValue: unknown | null;
  avgSizeBytes: number;
  histogramBuckets: number | null;
  histogramStale: boolean | null;
}

// ============================================================================
// EXPLAIN ANALYZE Types
// ============================================================================

/** Actual execution statistics from EXPLAIN ANALYZE. */
export interface ActualStats {
  actualRows: number;
  actualTimeMs: number;
  loops: number;
  nodesVisited: number;
  edgesTraversed: number;
}

/**
 * Per-node **estimated** execution statistics from EXPLAIN ANALYZE.
 *
 * All values are synthetic heuristics derived from the plan-global
 * `actualTimeMs` -- they are NOT individually measured per node.
 * Field names keep the `actual` prefix for API stability; check
 * the `estimated` flag to distinguish heuristic values from future
 * instrumented measurements.
 */
export interface NodeStats {
  nodeLabel: string;
  /** Estimated wall-clock time for this node (ms). */
  actualTimeMs: number;
  /** Estimated rows entering this node. */
  actualRowsIn: number;
  /** Estimated rows leaving this node. */
  actualRowsOut: number;
  loops: number;
  /** When true, all values are heuristic estimates, not measured. */
  estimated: boolean;
}

/** Collection statistics response */
export interface CollectionStatsResponse {
  totalPoints: number;
  totalSizeBytes: number;
  rowCount: number;
  deletedCount: number;
  avgRowSizeBytes: number;
  payloadSizeBytes: number;
  lastAnalyzedEpochMs: number;
  columnStats?: Record<string, ColumnStatsDetail>;
}

/** Collection configuration response. Mirrors `velesdb_core::api_types::CollectionConfigResponse`. */
export interface CollectionConfigResponse {
  name: string;
  dimension: number;
  metric: DistanceMetric;
  storageMode: StorageMode;
  pointCount: number;
  metadataOnly: boolean;
  graphSchema?: Record<string, unknown>;
  embeddingDimension?: number;
  /**
   * On-disk schema version. Increments when the persisted `config.json`
   * format changes in a way older `VelesDB` versions cannot safely read.
   */
  schemaVersion?: number;
  /** PQ rescore oversampling factor -- see `CollectionConfig.pqRescoreOversampling`. */
  pqRescoreOversampling?: number;
  /** Persisted HNSW parameters when customised at create time (raw server JSON). */
  hnswParams?: Record<string, unknown>;
  /** Deferred indexing configuration (`null` / absent when the feature is disabled for this collection). */
  deferredIndexing?: Record<string, unknown>;
  /** Async index builder configuration (`null` / absent when the feature is disabled for this collection). */
  asyncIndexBuilder?: Record<string, unknown>;
}

// ============================================================================
// VelesQL Multi-Model Query Types (EPIC-031 US-011)
// ============================================================================

/** VelesQL query options */
export interface QueryOptions {
  /** Timeout in milliseconds (default: 30000) */
  timeoutMs?: number;
  /** Enable streaming response */
  stream?: boolean;
}

/**
 * Query result row from VelesQL query.
 *
 * Shape depends on the SELECT clause:
 * - `SELECT *` -> `{id, field1, field2, ...}` (no vector)
 * - `SELECT col1, col2` -> `{col1, col2}`
 * - `SELECT similarity() AS score, title` -> `{score, title}`
 */
export type QueryResult = Record<string, unknown>;

/** Query execution statistics */
export interface QueryStats {
  /** Execution time in milliseconds */
  executionTimeMs: number;
  /** Execution strategy used */
  strategy: string;
  /** Number of nodes scanned */
  scannedNodes: number;
}

/** Full query response with results and stats */
export interface QueryResponse {
  /** Query results */
  results: QueryResult[];
  /** Execution statistics */
  stats: QueryStats;
}

/** Aggregation query response from VelesQL (`GROUP BY`, `COUNT`, `SUM`, etc.). */
export interface AggregationQueryResponse {
  /** Aggregation result payload as returned by server. */
  result: Record<string, unknown> | unknown[];
  /** Execution statistics */
  stats: QueryStats;
}

/** Unified response type for `query()` (rows, aggregation, or DDL).
 *
 * DDL statements (CREATE, DROP) and mutations (INSERT EDGE, DELETE) return
 * a standard `QueryResponse` with an empty `results` array.
 */
export type QueryApiResponse = QueryResponse | AggregationQueryResponse;

// ============================================================================
// EXPLAIN / Sanity Types
// ============================================================================

/** Query explain request/response metadata */
export interface ExplainPlanStep {
  step: number;
  operation: string;
  description: string;
  estimatedRows: number | null;
  estimationMethod: string | null;
}

export interface ExplainCost {
  usesIndex: boolean;
  indexName: string | null;
  selectivity: number;
  complexity: string;
}

export interface ExplainFeatures {
  hasVectorSearch: boolean;
  hasFilter: boolean;
  hasOrderBy: boolean;
  hasGroupBy: boolean;
  hasAggregation: boolean;
  hasJoin: boolean;
  hasFusion: boolean;
  limit: number | null;
  offset: number | null;
}

export interface ExplainResponse {
  query: string;
  queryType: string;
  collection: string;
  plan: ExplainPlanStep[];
  estimatedCost: ExplainCost;
  features: ExplainFeatures;
  actualStats?: ActualStats | null;
  nodeStats?: NodeStats[] | null;
}

export interface CollectionSanityChecks {
  hasVectors: boolean;
  searchReady: boolean;
  dimensionConfigured: boolean;
}

export interface CollectionSanityDiagnostics {
  searchRequestsTotal: number;
  dimensionMismatchTotal: number;
  emptySearchResultsTotal: number;
  filterParseErrorsTotal: number;
}

export interface CollectionSanityResponse {
  collection: string;
  dimension: number;
  metric: string;
  pointCount: number;
  isEmpty: boolean;
  checks: CollectionSanityChecks;
  diagnostics: CollectionSanityDiagnostics;
  hints: string[];
}
