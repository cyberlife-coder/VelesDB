/**
 * Missing REST Endpoint Wrappers (Sprint 2 Wave 4 — S2-NEW-10)
 *
 * Thin wrappers for the 12 `velesdb-server` endpoints that the pre-v1.13
 * TypeScript SDK did not expose. Each function takes a `BaseTransport`
 * and delegates to `requestJson`, keeping the REST backend compositional
 * and extractable per-endpoint. A single file holds all 12 so the diff
 * is auditable; see CHANGELOG for the scoping decision that leaves
 * `streamTraverse` (SSE) out of this commit.
 *
 * Endpoints covered:
 * - Admin: rebuildIndex, getGuardrails, updateGuardrails
 * - Query: aggregate, matchQuery
 * - Graph: removeEdge, getEdgeCount, listNodes, getNodeEdges,
 *          getNodePayload, upsertNodePayload, graphSearch
 */

import type {
  RebuildIndexResponse,
  GuardRailsUpdateRequest,
  GuardRailsConfigResponse,
  ListNodesResponse,
  GetNodeEdgesOptions,
  NodePayloadResponse,
  GraphSearchRequest,
  GraphSearchResponse,
  GraphSearchResultItem,
  GraphEdge,
  MatchQueryOptions,
  MatchQueryResponse,
  MatchQueryResultItem,
  AggregateQueryOptions,
  AggregateResponse,
} from '../types';
import type { BaseTransport } from './shared';
import {
  throwOnError,
  collectionPath,
  toNumberArray,
  isNotFoundError,
} from './shared';

// ============================================================================
// Admin
// ============================================================================

/** Raw wire shape returned by `POST /collections/{name}/index/rebuild`. */
interface RebuildIndexWire {
  message: string;
  collection: string;
  compacted_entries: number;
}

/**
 * Rebuild the HNSW index of a collection, reclaiming memory from
 * tombstoned entries.
 */
export async function rebuildIndex(
  transport: BaseTransport,
  collection: string
): Promise<RebuildIndexResponse> {
  const response = await transport.requestJson<RebuildIndexWire>(
    'POST',
    `${collectionPath(collection)}/index/rebuild`
  );
  throwOnError(response, `Collection '${collection}'`);
  const data = response.data!;
  return {
    message: data.message,
    collection: data.collection,
    compactedEntries: data.compacted_entries,
  };
}

/** Raw wire shape of `GET /guardrails` / `PUT /guardrails`. */
interface GuardRailsWire {
  max_depth: number;
  max_cardinality: number;
  memory_limit_bytes: number;
  timeout_ms: number;
  rate_limit_qps: number;
  circuit_failure_threshold: number;
  circuit_recovery_seconds: number;
}

function mapGuardRailsWire(data: GuardRailsWire): GuardRailsConfigResponse {
  return {
    maxDepth: data.max_depth,
    maxCardinality: data.max_cardinality,
    memoryLimitBytes: data.memory_limit_bytes,
    timeoutMs: data.timeout_ms,
    rateLimitQps: data.rate_limit_qps,
    circuitFailureThreshold: data.circuit_failure_threshold,
    circuitRecoverySeconds: data.circuit_recovery_seconds,
  };
}

function toGuardRailsWireUpdate(
  req: GuardRailsUpdateRequest
): Record<string, unknown> {
  const wire: Record<string, unknown> = {};
  if (req.maxDepth !== undefined) wire.max_depth = req.maxDepth;
  if (req.maxCardinality !== undefined) wire.max_cardinality = req.maxCardinality;
  if (req.memoryLimitBytes !== undefined) wire.memory_limit_bytes = req.memoryLimitBytes;
  if (req.timeoutMs !== undefined) wire.timeout_ms = req.timeoutMs;
  if (req.rateLimitQps !== undefined) wire.rate_limit_qps = req.rateLimitQps;
  if (req.circuitFailureThreshold !== undefined) {
    wire.circuit_failure_threshold = req.circuitFailureThreshold;
  }
  if (req.circuitRecoverySeconds !== undefined) {
    wire.circuit_recovery_seconds = req.circuitRecoverySeconds;
  }
  return wire;
}

/** Read the current guard-rails configuration (process-wide). */
export async function getGuardrails(
  transport: BaseTransport
): Promise<GuardRailsConfigResponse> {
  const response = await transport.requestJson<GuardRailsWire>('GET', '/guardrails');
  throwOnError(response);
  return mapGuardRailsWire(response.data!);
}

/**
 * Partial-update the guard-rails configuration. Any omitted field is
 * left unchanged on the server.
 */
export async function updateGuardrails(
  transport: BaseTransport,
  req: GuardRailsUpdateRequest
): Promise<GuardRailsConfigResponse> {
  const response = await transport.requestJson<GuardRailsWire>(
    'PUT',
    '/guardrails',
    toGuardRailsWireUpdate(req)
  );
  throwOnError(response);
  return mapGuardRailsWire(response.data!);
}

// ============================================================================
// Query
// ============================================================================

/** Raw wire shape of `POST /aggregate`. */
interface AggregateResponseWire {
  result: unknown;
  timing_ms: number;
  meta: { velesql_contract_version: string; count: number };
}

/**
 * Execute a VelesQL aggregate query (`SELECT COUNT(*) / AVG(...) / ...`).
 *
 * The server exposes a dedicated `/aggregate` endpoint optimised for
 * group-by + aggregation queries; this wrapper forwards the query
 * string and optional bind parameters verbatim. The return type is
 * the dedicated `AggregateResponse` — NOT the generic `QueryApiResponse`
 * — because the wire format is distinct (`{ result, timing_ms, meta }`,
 * not `{ rows, stats }`).
 */
export async function aggregate(
  transport: BaseTransport,
  queryString: string,
  params?: Record<string, unknown>,
  options?: AggregateQueryOptions
): Promise<AggregateResponse> {
  const body: Record<string, unknown> = {
    query: queryString,
    params: params ?? {},
  };
  if (options?.collection !== undefined) {
    body.collection = options.collection;
  }
  const response = await transport.requestJson<AggregateResponseWire>(
    'POST',
    '/aggregate',
    body
  );
  throwOnError(response);
  const data = response.data!;
  return {
    result: data.result,
    timingMs: data.timing_ms,
    meta: {
      velesqlContractVersion: data.meta.velesql_contract_version,
      count: data.meta.count,
    },
  };
}

/** Raw wire shape of `POST /collections/{name}/match`. */
interface MatchQueryResponseWire {
  results: Array<{
    bindings: Record<string, number>;
    score?: number;
    depth: number;
    projected?: Record<string, unknown>;
  }>;
  took_ms: number;
  count: number;
  meta: { velesql_contract_version: string };
}

/**
 * Execute a VelesQL `MATCH (...)` graph query against a specific
 * collection. Thin wrapper around `POST /collections/{name}/match`.
 *
 * Returns the dedicated `MatchQueryResponse` type (pattern-binding
 * rows + meta) rather than the generic `QueryApiResponse`, matching
 * the actual `MatchQueryResponse` struct emitted by the server. The
 * optional `vector` + `threshold` in `options` are forwarded to the
 * MATCH similarity scorer when the query uses `similarity(node.vec, $v)`.
 */
export async function matchQuery(
  transport: BaseTransport,
  collection: string,
  queryString: string,
  params?: Record<string, unknown>,
  options?: MatchQueryOptions
): Promise<MatchQueryResponse> {
  const body: Record<string, unknown> = {
    query: queryString,
    params: params ?? {},
  };
  if (options?.vector !== undefined) {
    body.vector = toNumberArray(options.vector);
  }
  if (options?.threshold !== undefined) {
    body.threshold = options.threshold;
  }
  const response = await transport.requestJson<MatchQueryResponseWire>(
    'POST',
    `${collectionPath(collection)}/match`,
    body
  );
  throwOnError(response, `Collection '${collection}'`);
  const data = response.data!;
  const items: MatchQueryResultItem[] = data.results.map((r) => ({
    bindings: r.bindings,
    score: r.score,
    depth: r.depth,
    projected: r.projected ?? {},
  }));
  return {
    results: items,
    tookMs: data.took_ms,
    count: data.count,
    meta: { velesqlContractVersion: data.meta.velesql_contract_version },
  };
}

// ============================================================================
// Graph — extended endpoints
// ============================================================================

/**
 * Remove an edge by ID. Returns `true` if removed, `false` if not found.
 *
 * Uses [`isNotFoundError`] to absorb both the legacy status-derived
 * `'NOT_FOUND'` code and the typed `VELES-020 EdgeNotFound` code so
 * the function behaves identically whether the server handler has
 * been migrated to `core_error_response` or still uses
 * `error_response` (PR #586 Devin fix).
 */
export async function removeEdge(
  transport: BaseTransport,
  collection: string,
  edgeId: number
): Promise<boolean> {
  const response = await transport.requestJson(
    'DELETE',
    `${collectionPath(collection)}/graph/edges/${edgeId}`
  );
  if (response.error !== undefined) {
    if (isNotFoundError(response.error.code)) {
      return false;
    }
    throwOnError(response, `Collection '${collection}'`);
  }
  return true;
}

/** Total edge count in the graph collection. */
export async function getEdgeCount(
  transport: BaseTransport,
  collection: string
): Promise<number> {
  const response = await transport.requestJson<{ count: number }>(
    'GET',
    `${collectionPath(collection)}/graph/edges/count`
  );
  throwOnError(response, `Collection '${collection}'`);
  return response.data?.count ?? 0;
}

/** List all node IDs in the graph collection. */
export async function listNodes(
  transport: BaseTransport,
  collection: string
): Promise<ListNodesResponse> {
  const response = await transport.requestJson<{ node_ids: number[]; count: number }>(
    'GET',
    `${collectionPath(collection)}/graph/nodes`
  );
  throwOnError(response, `Collection '${collection}'`);
  const data = response.data!;
  return { nodeIds: data.node_ids, count: data.count };
}

/** Raw wire shape of an edge as returned by the server. */
interface EdgeWire {
  id: number;
  source: number;
  target: number;
  label: string;
  properties?: Record<string, unknown>;
}

/**
 * Get edges adjacent to a node, filtered by direction and optional label.
 */
export async function getNodeEdges(
  transport: BaseTransport,
  collection: string,
  nodeId: number,
  options?: GetNodeEdgesOptions
): Promise<GraphEdge[]> {
  const params = new URLSearchParams();
  if (options?.direction) params.set('direction', options.direction);
  if (options?.label) params.set('label', options.label);
  const qs = params.toString();
  const url =
    `${collectionPath(collection)}/graph/nodes/${nodeId}/edges` +
    (qs ? `?${qs}` : '');

  const response = await transport.requestJson<{
    edges: EdgeWire[];
    count: number;
  }>('GET', url);
  throwOnError(response, `Collection '${collection}'`);

  return (response.data?.edges ?? []).map((e) => ({
    id: e.id,
    source: e.source,
    target: e.target,
    label: e.label,
    properties: e.properties,
  }));
}

/** Read the JSON payload attached to a graph node. */
export async function getNodePayload(
  transport: BaseTransport,
  collection: string,
  nodeId: number
): Promise<NodePayloadResponse> {
  const response = await transport.requestJson<{
    node_id: number | string;
    payload: Record<string, unknown> | null;
  }>('GET', `${collectionPath(collection)}/graph/nodes/${nodeId}/payload`);
  throwOnError(response, `Collection '${collection}'`);
  const data = response.data!;
  return {
    nodeId: typeof data.node_id === 'string' ? Number(data.node_id) : data.node_id,
    payload: data.payload,
  };
}

/** Upsert (create or replace) the JSON payload of a graph node. */
export async function upsertNodePayload(
  transport: BaseTransport,
  collection: string,
  nodeId: number,
  payload: Record<string, unknown>
): Promise<void> {
  const response = await transport.requestJson(
    'PUT',
    `${collectionPath(collection)}/graph/nodes/${nodeId}/payload`,
    { payload }
  );
  throwOnError(response, `Collection '${collection}'`);
}

/**
 * Find graph nodes whose embedding is closest to a query vector.
 *
 * This is the graph-scoped equivalent of `/collections/{name}/search`;
 * results carry only node IDs and scores (no payload hydration, no
 * filtering).
 */
export async function graphSearch(
  transport: BaseTransport,
  collection: string,
  request: GraphSearchRequest
): Promise<GraphSearchResponse> {
  const response = await transport.requestJson<{
    results: Array<{ id: number | string; score: number }>;
  }>(
    'POST',
    `${collectionPath(collection)}/graph/search`,
    {
      vector: toNumberArray(request.vector),
      top_k: request.k ?? 10,
    }
  );
  throwOnError(response, `Collection '${collection}'`);
  const items: GraphSearchResultItem[] = (response.data?.results ?? []).map(
    (r) => ({
      id: typeof r.id === 'string' ? Number(r.id) : r.id,
      score: r.score,
    })
  );
  return { results: items };
}
