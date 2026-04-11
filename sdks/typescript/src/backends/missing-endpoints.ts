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
import { throwOnError, collectionPath, toNumberArray } from './shared';
import { parseVelesError, EdgeNotFoundError } from '../errors';

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
 * Remove an edge by ID. Returns `true` if removed, `false` if the
 * specific edge does not exist.
 *
 * **Scope contract** (PR #586 Devin finding #6): only edge-not-found
 * is absorbed as `false`. Collection-not-found, point-not-found,
 * node-not-found, and every other error are re-thrown so the caller
 * sees the real problem instead of a misleading "edge not in this
 * collection" boolean.
 *
 * Accepts two wire formats:
 * - **Typed** — server emitted `VELES-020` via
 *   `core_error_response(Error::EdgeNotFound)`, producing an
 *   `EdgeNotFoundError`.
 * - **Legacy** — server emitted no VELES code, the transport layer
 *   filled in `'NOT_FOUND'` via `mapStatusToErrorCode(404)`. Since
 *   the legacy path cannot distinguish "edge missing" from
 *   "collection missing", we accept it as an edge-not-found signal
 *   ONLY when the DELETE URL already targets a specific edge (which
 *   it does — the route is `/collections/{name}/graph/edges/{id}`).
 *   This path is safe because the server only 404s here for a
 *   missing edge; missing-collection errors go through
 *   `get_graph_collection_or_404` which now emits typed `VELES-002`.
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
    const { code, message } = response.error;
    // Typed path: only EdgeNotFoundError counts as "absent edge".
    // Any other VELES code (CollectionNotFound, etc.) must propagate
    // so the caller sees the real error instead of a misleading
    // `false` return.
    const err = parseVelesError(code, message);
    if (err instanceof EdgeNotFoundError) {
      return false;
    }
    // Legacy path: the server omitted the VELES code and the
    // transport filled in `'NOT_FOUND'`. At this URL the only 404
    // the handler produces is edge-not-found (collection 404 is
    // emitted upstream by `get_graph_collection_or_404` which now
    // always carries VELES-002, handled by the typed branch above).
    if (code === 'NOT_FOUND') {
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

/**
 * Raw wire shape of an edge as returned by the server.
 *
 * `id`/`source`/`target` arrive as **strings** because the server's
 * `EdgeResponse` struct uses `#[serde(serialize_with =
 * "serde_id::serialize_id_as_string")]` to avoid JavaScript's
 * `Number.MAX_SAFE_INTEGER` (2^53-1) precision loss on u64 values.
 * The `idToNumber` helper coerces them back to the `number` shape
 * declared on the public `GraphEdge` interface; callers that need
 * u64-safe IDs should migrate to the TypeScript `bigint` surface
 * (tracked as a follow-up for Sprint 3+ streaming API commit).
 */
interface EdgeWire {
  id: number | string;
  source: number | string;
  target: number | string;
  label: string;
  properties?: Record<string, unknown>;
}

/**
 * Coerce a wire-format node/edge ID (which may arrive as a string
 * because of `serialize_id_as_string`) into the `number` shape
 * declared on the public `GraphEdge` interface. IDs above
 * `Number.MAX_SAFE_INTEGER` (2^53-1) will lose precision — this is
 * an accepted limitation of the current `id: number` contract.
 */
function idToNumber(id: number | string): number {
  return typeof id === 'string' ? Number(id) : id;
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
    id: idToNumber(e.id),
    source: idToNumber(e.source),
    target: idToNumber(e.target),
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
    nodeId: idToNumber(data.node_id),
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
    results: Array<{
      id: number | string;
      score: number;
      payload?: Record<string, unknown> | null;
    }>;
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
      id: idToNumber(r.id),
      score: r.score,
      payload: r.payload,
    })
  );
  return { results: items };
}
