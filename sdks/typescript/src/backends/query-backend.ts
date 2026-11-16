/**
 * Query Backend operations for VelesDB REST API.
 *
 * Extracted from rest.ts to keep file size manageable.
 * Implements: query, queryExplain, collectionSanity.
 */

import type {
  QueryApiResponse,
  QueryResponseMeta,
  ExplainResponse,
  ExplainPlanStep,
  ExplainCost,
  ExplainFeatures,
  ActualStats,
  NodeStats,
  CollectionSanityResponse,
} from '../types';
import type { BaseTransport, TransportResponse } from './shared';
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
    estimation_method?: string | null;
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
  cache_hit?: boolean | null;
  plan_reuse_count?: number | null;
  estimated_cost_ms?: number | null;
  actual_time_ms?: number | null;
  actual_stats?: {
    actual_rows: number;
    actual_time_ms: number;
    loops: number;
    nodes_visited: number;
    edges_traversed: number;
    traversal_counters_approximate?: boolean | null;
    executed_filter_strategy?: string | null;
  } | null;
  node_stats?: Array<{
    node_label: string;
    actual_time_ms: number;
    actual_rows_in: number;
    actual_rows_out: number;
    loops: number;
    estimated: boolean;
  }> | null;
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

/** Detect DDL, mutation, introspection, or admin statements that must always route to `/query`. */
export function isLikelyDdlOrMutationQuery(queryString: string): boolean {
  return /^\s*(CREATE|DROP)\s+(COLLECTION|GRAPH|METADATA|INDEX)\b/i.test(queryString)
    || /^\s*DELETE\s+(FROM|EDGE)\b/i.test(queryString)
    || /^\s*INSERT\s+(INTO|EDGE|NODE)\b/i.test(queryString)
    || /^\s*UPSERT\s+INTO\b/i.test(queryString)
    || /^\s*(SHOW|DESCRIBE|EXPLAIN)\b/i.test(queryString)
    || /^\s*(FLUSH|ANALYZE|TRUNCATE)\b/i.test(queryString)
    || /^\s*ALTER\s+COLLECTION\b/i.test(queryString)
    || /^\s*SELECT\s+EDGES\b/i.test(queryString);
}

/** Wrap the raw `result` payload of an `/aggregate` answer. */
function aggregationResponse(rawData: Record<string, unknown>): QueryApiResponse {
  return {
    result: rawData.result as Record<string, unknown> | unknown[],
    stats: {
      executionTimeMs: (rawData.timing_ms as number | undefined) ?? 0,
      strategy: 'aggregation',
      scannedNodes: 0,
    },
  };
}

/** Map the `meta` section of a `/query` response (`QueryResponseMeta`), when present. */
function queryMeta(rawData: Record<string, unknown> | undefined): QueryResponseMeta | undefined {
  const meta = rawData?.meta;
  if (meta == null) return undefined;
  const m = meta as Record<string, unknown>;
  return {
    velesqlContractVersion: m.velesql_contract_version as string,
    count: m.count as number,
  };
}

/** Project the rows of a `/query` answer — shape depends on the SELECT clause. */
function selectResponse(rawData: Record<string, unknown> | undefined): QueryApiResponse {
  return {
    results: (rawData?.results ?? []) as Record<string, unknown>[],
    stats: {
      executionTimeMs: (rawData?.timing_ms as number | undefined) ?? 0,
      strategy: 'select',
      scannedNodes: (rawData?.rows_returned as number | undefined) ?? 0,
    },
    tookMs: (rawData?.took_ms as number | undefined) ?? undefined,
    meta: queryMeta(rawData),
  };
}

/**
 * POST the query, falling back from `/aggregate` to `/query` when the
 * aggregation endpoint refuses the query as non-aggregating. That refusal is
 * HTTP 422 with `error.code = "VELESQL_AGGREGATION_ERROR"` on the wire
 * (`handlers/query/aggregation.rs`) — not `BAD_REQUEST` — so the predicate
 * matches the real code.
 */
async function postQuery(
  transport: QueryTransport,
  collection: string,
  queryString: string,
  params: Record<string, unknown> | undefined,
  useAggregate: boolean
): Promise<TransportResponse<Record<string, unknown>>> {
  const body = {
    query: queryString,
    params: params ?? {},
    collection,
  };
  let response = await transport.requestJson<Record<string, unknown>>(
    'POST',
    useAggregate ? '/aggregate' : '/query',
    body
  );
  if (useAggregate && response.error?.code === 'VELESQL_AGGREGATION_ERROR') {
    response = await transport.requestJson<Record<string, unknown>>(
      'POST',
      '/query',
      body
    );
  }
  return response;
}

export async function query(
  transport: QueryTransport,
  collection: string,
  queryString: string,
  params?: Record<string, unknown>
): Promise<QueryApiResponse> {
  // `/aggregate` is an optimisation endpoint that only accepts aggregation
  // queries; `/query` executes everything. The aggregation heuristic is a
  // regex over the raw text, so a SELECT whose string literals merely contain
  // "GROUP BY"/"COUNT(" would be misrouted and refused with a 422 — fall back
  // to `/query` (which runs aggregations too) instead of surfacing that
  // refusal as the query's error.
  const useAggregate =
    !isLikelyDdlOrMutationQuery(queryString) && isLikelyAggregationQuery(queryString);
  const response = await postQuery(transport, collection, queryString, params, useAggregate);

  throwOnError(response, `Collection '${collection}'`);

  const rawData = response.data; // Record<string, unknown> | undefined
  if (rawData != null && Object.prototype.hasOwnProperty.call(rawData, 'result')) {
    return aggregationResponse(rawData);
  }
  // v3.0.0: Results are projected rows — `took_ms` (whole-ms compat helper)
  // and `meta` are forwarded too — they are part of the server's
  // QueryResponse and were previously dropped.
  return selectResponse(rawData);
}

function mapPlanStep(step: QueryExplainApiResponse['plan'][number]): ExplainPlanStep {
  return {
    step: step.step,
    operation: step.operation,
    description: step.description,
    estimatedRows: step.estimated_rows,
    estimationMethod: step.estimation_method ?? null,
  };
}

function mapExplainCost(cost: QueryExplainApiResponse['estimated_cost']): ExplainCost {
  return {
    usesIndex: cost.uses_index,
    indexName: cost.index_name,
    selectivity: cost.selectivity,
    complexity: cost.complexity,
  };
}

function mapExplainFeatures(features: QueryExplainApiResponse['features']): ExplainFeatures {
  return {
    hasVectorSearch: features.has_vector_search,
    hasFilter: features.has_filter,
    hasOrderBy: features.has_order_by,
    hasGroupBy: features.has_group_by,
    hasAggregation: features.has_aggregation,
    hasJoin: features.has_join,
    hasFusion: features.has_fusion,
    limit: features.limit,
    offset: features.offset,
  };
}

function mapActualStats(
  stats: QueryExplainApiResponse['actual_stats']
): ActualStats | null | undefined {
  if (stats === null) return null;
  if (!stats) return undefined;
  return {
    actualRows: stats.actual_rows,
    actualTimeMs: stats.actual_time_ms,
    loops: stats.loops,
    nodesVisited: stats.nodes_visited,
    edgesTraversed: stats.edges_traversed,
    traversalCountersApproximate: stats.traversal_counters_approximate ?? undefined,
    executedFilterStrategy: stats.executed_filter_strategy ?? undefined,
  };
}

function mapNodeStats(
  nodeStats: QueryExplainApiResponse['node_stats']
): NodeStats[] | null | undefined {
  if (nodeStats === null) return null;
  if (!nodeStats) return undefined;
  return nodeStats.map((ns) => ({
    nodeLabel: ns.node_label,
    actualTimeMs: ns.actual_time_ms,
    actualRowsIn: ns.actual_rows_in,
    actualRowsOut: ns.actual_rows_out,
    loops: ns.loops,
    estimated: ns.estimated,
  }));
}

/** `?? undefined` as a helper so the mapping functions stay under the complexity budget. */
function nullish<T>(value: T | null | undefined): T | undefined {
  return value ?? undefined;
}

/** Map the wire EXPLAIN response to the SDK's camelCase {@link ExplainResponse}. */
function mapExplainData(data: QueryExplainApiResponse): ExplainResponse {
  return {
    query: data.query,
    queryType: data.query_type,
    collection: data.collection,
    plan: data.plan.map(mapPlanStep),
    estimatedCost: mapExplainCost(data.estimated_cost),
    features: mapExplainFeatures(data.features),
    cacheHit: nullish(data.cache_hit),
    planReuseCount: nullish(data.plan_reuse_count),
    estimatedCostMs: nullish(data.estimated_cost_ms),
    actualTimeMs: nullish(data.actual_time_ms),
    actualStats: mapActualStats(data.actual_stats),
    nodeStats: mapNodeStats(data.node_stats),
  };
}

export async function queryExplain(
  transport: QueryTransport,
  queryString: string,
  params?: Record<string, unknown>,
  options?: { analyze?: boolean }
): Promise<ExplainResponse> {
  const body: Record<string, unknown> = {
    query: queryString,
    params: params ?? {},
  };
  if (options?.analyze) {
    body.analyze = true;
  }

  const response = await transport.requestJson<QueryExplainApiResponse>(
    'POST',
    '/query/explain',
    body
  );

  throwOnError(response);

  return mapExplainData(response.data!);
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
