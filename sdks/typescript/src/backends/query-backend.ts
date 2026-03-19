/**
 * Query Backend operations for VelesDB REST API.
 *
 * Extracted from rest.ts to keep file size manageable.
 * Implements: query, queryExplain, collectionSanity.
 */

import type {
  QueryOptions,
  QueryApiResponse,
  ExplainResponse,
  CollectionSanityResponse,
} from '../types';
import type { BaseTransport } from './shared';
import { throwOnError, collectionPath } from './shared';

/** REST API shape for EXPLAIN responses. */
export interface QueryExplainApiResponse {
  query: string;
  query_type: string;
  collection: string;
  plan: Array<{
    step: number;
    operation: string;
    description: string;
    estimated_rows: number | null;
  }>;
  estimated_cost: {
    uses_index: boolean;
    index_name: string | null;
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
    limit: number | null;
    offset: number | null;
  };
}

/** REST API shape for collection sanity check responses. */
export interface CollectionSanityApiResponse {
  collection: string;
  dimension: number;
  metric: string;
  point_count: number;
  is_empty: boolean;
  checks: {
    has_vectors: boolean;
    search_ready: boolean;
    dimension_configured: boolean;
  };
  diagnostics: {
    search_requests_total: number;
    dimension_mismatch_total: number;
    empty_search_results_total: number;
    filter_parse_errors_total: number;
  };
  hints: string[];
}

/** Minimal transport interface for query operations. */
export interface QueryTransport extends BaseTransport {
  parseNodeId(value: unknown): bigint | number;
}

export function isLikelyAggregationQuery(queryString: string): boolean {
  return /\bGROUP\s+BY\b|\bHAVING\b|\bCOUNT\s*\(|\bSUM\s*\(|\bAVG\s*\(|\bMIN\s*\(|\bMAX\s*\(/i.test(
    queryString
  );
}

export async function query(
  transport: QueryTransport,
  collection: string,
  queryString: string,
  params?: Record<string, unknown>,
  options?: QueryOptions
): Promise<QueryApiResponse> {
  const endpoint = isLikelyAggregationQuery(queryString) ? '/aggregate' : '/query';
  const response = await transport.requestJson<Record<string, unknown>>(
    'POST',
    endpoint,
    {
      query: queryString,
      params: params ?? {},
      collection,
      timeout_ms: options?.timeoutMs,
      stream: options?.stream ?? false,
    }
  );

  throwOnError(response, `Collection '${collection}'`);

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const rawData = response.data as any;
  if (rawData && Object.prototype.hasOwnProperty.call(rawData, 'result')) {
    return {
      result: rawData.result as Record<string, unknown> | unknown[],
      stats: {
        executionTimeMs: rawData.timing_ms ?? 0,
        strategy: 'aggregation',
        scannedNodes: 0,
      },
    };
  }

  // v3.0.0: Results are projected rows — shape depends on SELECT clause.
  return {
    results: (rawData?.results ?? []) as Record<string, unknown>[],
    stats: {
      executionTimeMs: rawData?.timing_ms ?? 0,
      strategy: 'select',
      scannedNodes: rawData?.rows_returned ?? 0,
    },
  };
}

export async function queryExplain(
  transport: QueryTransport,
  queryString: string,
  params?: Record<string, unknown>
): Promise<ExplainResponse> {
  const response = await transport.requestJson<QueryExplainApiResponse>(
    'POST',
    '/query/explain',
    {
      query: queryString,
      params: params ?? {},
    }
  );

  throwOnError(response);

  const data = response.data!;
  return {
    query: data.query,
    queryType: data.query_type,
    collection: data.collection,
    plan: data.plan.map(step => ({
      step: step.step,
      operation: step.operation,
      description: step.description,
      estimatedRows: step.estimated_rows,
    })),
    estimatedCost: {
      usesIndex: data.estimated_cost.uses_index,
      indexName: data.estimated_cost.index_name,
      selectivity: data.estimated_cost.selectivity,
      complexity: data.estimated_cost.complexity,
    },
    features: {
      hasVectorSearch: data.features.has_vector_search,
      hasFilter: data.features.has_filter,
      hasOrderBy: data.features.has_order_by,
      hasGroupBy: data.features.has_group_by,
      hasAggregation: data.features.has_aggregation,
      hasJoin: data.features.has_join,
      hasFusion: data.features.has_fusion,
      limit: data.features.limit,
      offset: data.features.offset,
    },
  };
}

export async function collectionSanity(
  transport: QueryTransport,
  collection: string
): Promise<CollectionSanityResponse> {
  const response = await transport.requestJson<CollectionSanityApiResponse>(
    'GET',
    `${collectionPath(collection)}/sanity`
  );

  throwOnError(response, `Collection '${collection}'`);

  const data = response.data!;
  return {
    collection: data.collection,
    dimension: data.dimension,
    metric: data.metric,
    pointCount: data.point_count,
    isEmpty: data.is_empty,
    checks: {
      hasVectors: data.checks.has_vectors,
      searchReady: data.checks.search_ready,
      dimensionConfigured: data.checks.dimension_configured,
    },
    diagnostics: {
      searchRequestsTotal: data.diagnostics.search_requests_total,
      dimensionMismatchTotal: data.diagnostics.dimension_mismatch_total,
      emptySearchResultsTotal: data.diagnostics.empty_search_results_total,
      filterParseErrorsTotal: data.diagnostics.filter_parse_errors_total,
    },
    hints: data.hints ?? [],
  };
}
