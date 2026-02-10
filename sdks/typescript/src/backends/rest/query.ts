/**
 * Query operations for REST backend (VelesQL, MATCH, EXPLAIN)
 */

import type {
  QueryOptions,
  QueryResponse,
  MatchQueryOptions,
  MatchQueryResponse,
  ExplainResponse,
} from '../../types';
import { NotFoundError, VelesDBError } from '../../types';
import type { HttpClient } from './http-client';
import type {
  ServerMatchQueryResponse,
  ServerSelectQueryResponse,
  ServerAggregationResponse,
  ServerExplainResponse,
} from './server-types';

/**
 * Execute a VelesQL SELECT query.
 * 
 * For MATCH queries, use `matchQuery()` or pass a MATCH query here
 * for automatic routing to the correct endpoint.
 */
export async function query(
  client: HttpClient,
  collection: string,
  queryString: string,
  params?: Record<string, unknown>,
  options?: QueryOptions
): Promise<QueryResponse> {
  client.ensureInitialized();

  // Smart routing: detect MATCH queries and delegate to matchQuery()
  const trimmed = queryString.trim();
  if (trimmed.toUpperCase().startsWith('MATCH')) {
    const matchResult = await matchQuery(client, collection, queryString, params, {
      vector: options?.vector,
      threshold: options?.threshold,
    });
    // Adapt MatchQueryResponse → QueryResponse for unified interface
    return {
      results: matchResult.results.map(r => ({
        nodeId: Object.values(r.bindings)[0] as bigint | number ?? 0,
        vectorScore: r.score,
        graphScore: null,
        fusedScore: r.score ?? 0,
        bindings: { ...r.bindings, ...r.projected },
        columnData: null,
      })),
      stats: {
        executionTimeMs: matchResult.tookMs,
        strategy: 'match',
        scannedNodes: matchResult.count,
      },
    };
  }

  // SELECT query → POST /query
  const response = await client.request<ServerSelectQueryResponse | ServerAggregationResponse>(
    'POST', '/query',
    { query: queryString, params: params ?? {} }
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  const rawData = response.data;

  // Detect aggregation response: has `result` (singular) instead of `results` (plural)
  if (rawData && 'result' in rawData && !('results' in rawData)) {
    const aggData = rawData as ServerAggregationResponse;
    return {
      results: [{
        nodeId: 0,
        vectorScore: null,
        graphScore: null,
        fusedScore: 0,
        bindings: typeof aggData.result === 'object' && aggData.result !== null
          ? aggData.result as Record<string, unknown>
          : { value: aggData.result },
        columnData: null,
      }],
      stats: {
        executionTimeMs: aggData.timing_ms ?? 0,
        strategy: 'aggregation',
        scannedNodes: 0,
      },
    };
  }

  // Standard SELECT response
  const selectData = rawData as ServerSelectQueryResponse;
  return {
    results: (selectData?.results ?? []).map((r) => ({
      nodeId: client.parseNodeId(r.id),
      vectorScore: r.score ?? null,
      graphScore: null,
      fusedScore: r.score ?? 0,
      bindings: r.payload ?? {},
      columnData: null,
    })),
    stats: {
      executionTimeMs: selectData?.timing_ms ?? 0,
      strategy: 'select',
      scannedNodes: selectData?.rows_returned ?? 0,
    },
  };
}

/**
 * Execute a MATCH graph traversal query.
 * Calls `POST /collections/{name}/match` on the server.
 */
export async function matchQuery(
  client: HttpClient,
  collection: string,
  queryString: string,
  params?: Record<string, unknown>,
  options?: MatchQueryOptions
): Promise<MatchQueryResponse> {
  client.ensureInitialized();

  const body: Record<string, unknown> = {
    query: queryString,
    params: params ?? {},
  };

  if (options?.vector) {
    body.vector = options.vector instanceof Float32Array
      ? Array.from(options.vector)
      : options.vector;
  }
  if (options?.threshold !== undefined) {
    body.threshold = options.threshold;
  }

  const response = await client.request<ServerMatchQueryResponse>(
    'POST',
    `/collections/${encodeURIComponent(collection)}/match`,
    body
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  const data = response.data;
  return {
    results: (data?.results ?? []).map(r => ({
      bindings: r.bindings,
      score: r.score ?? null,
      depth: r.depth,
      projected: r.projected ?? {},
    })),
    tookMs: data?.took_ms ?? 0,
    count: data?.count ?? 0,
  };
}

/**
 * Explain a VelesQL query without executing it.
 * Returns the query plan, estimated costs, and detected features.
 */
export async function explain(
  client: HttpClient,
  queryString: string,
  params?: Record<string, unknown>
): Promise<ExplainResponse> {
  client.ensureInitialized();

  const body: Record<string, unknown> = { query: queryString };
  if (params) {
    body.params = params;
  }

  const response = await client.request<ServerExplainResponse>(
    'POST',
    '/query/explain',
    body
  );

  if (response.error) {
    throw new VelesDBError(response.error.message, response.error.code);
  }

  const data = response.data;
  if (!data) {
    throw new VelesDBError('Empty response from explain endpoint', 'EMPTY_RESPONSE');
  }

  return {
    query: data.query,
    queryType: data.query_type,
    collection: data.collection,
    plan: data.plan.map(step => ({
      step: step.step,
      operation: step.operation,
      description: step.description,
      estimatedRows: step.estimated_rows ?? undefined,
    })),
    estimatedCost: {
      usesIndex: data.estimated_cost.uses_index,
      indexName: data.estimated_cost.index_name ?? undefined,
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
      limit: data.features.limit ?? undefined,
      offset: data.features.offset ?? undefined,
    },
  };
}
