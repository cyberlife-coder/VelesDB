/**
 * Internal server response types (snake_case contracts)
 * 
 * These interfaces match the exact JSON structure returned by the VelesDB server.
 * They are NOT exported from the package — only used internally for type-safe mapping.
 */

import type { SearchResult } from '../../types';

/** REST API response wrapper */
export interface ApiResponse<T> {
  data?: T;
  error?: {
    code: string;
    message: string;
  };
}

/** Batch search response structure */
export interface BatchSearchResponse {
  results: Array<{ results: SearchResult[] }>;
}

/** Server-side MATCH response (snake_case contract) */
export interface ServerMatchQueryResponse {
  results: Array<{
    bindings: Record<string, unknown>;
    score?: number | null;
    depth: number;
    projected?: Record<string, unknown>;
  }>;
  took_ms: number;
  count: number;
}

/** Server SELECT /query response */
export interface ServerSelectQueryResponse {
  results: Array<{
    id: number;
    score: number;
    payload: Record<string, unknown> | null;
  }>;
  timing_ms: number;
  rows_returned: number;
}

/** Server aggregation /query response */
export interface ServerAggregationResponse {
  result: unknown;
  timing_ms: number;
}

/** Server-side EXPLAIN response (snake_case contract) */
export interface ServerExplainResponse {
  query: string;
  query_type: string;
  collection: string;
  plan: Array<{
    step: number;
    operation: string;
    description: string;
    estimated_rows?: number | null;
  }>;
  estimated_cost: {
    uses_index: boolean;
    index_name?: string | null;
    selectivity: number;
    complexity: string;
  };
  features: {
    has_vector_search: boolean;
    has_filter: boolean;
    has_order_by: boolean;
    has_group_by: boolean;
    has_aggregation: boolean;
    has_join: boolean;
    has_fusion: boolean;
    limit?: number | null;
    offset?: number | null;
  };
}
