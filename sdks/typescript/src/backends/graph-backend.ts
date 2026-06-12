/**
 * Graph Backend operations for VelesDB REST API.
 *
 * Extracted from rest.ts to keep file size manageable.
 * Implements: addEdge, getEdges, traverseGraph, getNodeDegree,
 * createGraphCollection.
 */

import type {
  AddEdgeRequest,
  GetEdgesOptions,
  GraphEdge,
  GraphNodeId,
  TraverseRequest,
  TraverseParallelRequest,
  TraverseResponse,
  DegreeResponse,
  GraphCollectionConfig,
  RelateRequest,
  RelateResponse,
  RelationsResponse,
} from '../types';
import type { BaseTransport } from './shared';
import { throwOnError, collectionPath } from './shared';
import { parseVelesError, EdgeNotFoundError } from '../errors';

/** Minimal transport interface for graph operations. */
export type GraphTransport = BaseTransport;

export async function addEdge(
  transport: GraphTransport,
  collection: string,
  edge: AddEdgeRequest
): Promise<void> {
  const response = await transport.requestJson(
    'POST',
    `${collectionPath(collection)}/graph/edges`,
    {
      id: edge.id,
      source: edge.source,
      target: edge.target,
      label: edge.label,
      properties: edge.properties ?? {},
    }
  );

  throwOnError(response, `Collection '${collection}'`);
}

/**
 * Raw wire shape of an edge from the server.
 *
 * `id`/`source`/`target` arrive as **strings** because the server's
 * `EdgeResponse` struct uses `serialize_id_as_string` to avoid
 * JavaScript `Number.MAX_SAFE_INTEGER` precision loss on u64 values.
 * The `toGraphEdge` helper preserves string IDs so u64 values above
 * `Number.MAX_SAFE_INTEGER` remain exact in JavaScript.
 */
interface EdgeWire {
  id: number | string;
  source: number | string;
  target: number | string;
  label: string;
  properties?: Record<string, unknown>;
}

function toGraphEdge(e: EdgeWire): GraphEdge {
  return {
    id: e.id,
    source: e.source,
    target: e.target,
    label: e.label,
    properties: e.properties,
  };
}

/**
 * Raw wire shape of a traverse / traverse-parallel response.
 *
 * `target_id` and `path` arrive as `number | string` to preserve u64 node IDs
 * above `Number.MAX_SAFE_INTEGER` (see {@link EdgeWire}).
 */
interface TraverseWire {
  results: Array<{ target_id: number | string; depth: number; path: Array<number | string> }>;
  next_cursor: string | null;
  has_more: boolean;
  stats: { visited: number; depth_reached: number };
}

/** Maps a raw traverse wire response to the public {@link TraverseResponse}. */
function toTraverseResponse(data: TraverseWire): TraverseResponse {
  return {
    results: data.results.map(r => ({
      targetId: r.target_id,
      depth: r.depth,
      path: r.path,
    })),
    nextCursor: data.next_cursor ?? undefined,
    hasMore: data.has_more,
    stats: {
      visited: data.stats.visited,
      depthReached: data.stats.depth_reached,
    },
  };
}

export async function getEdges(
  transport: GraphTransport,
  collection: string,
  options?: GetEdgesOptions
): Promise<GraphEdge[]> {
  const queryParams = options?.label ? `?label=${encodeURIComponent(options.label)}` : '';

  const response = await transport.requestJson<{ edges: EdgeWire[]; count: number }>(
    'GET',
    `${collectionPath(collection)}/graph/edges${queryParams}`
  );

  throwOnError(response, `Collection '${collection}'`);

  return (response.data?.edges ?? []).map(toGraphEdge);
}

export async function traverseGraph(
  transport: GraphTransport,
  collection: string,
  request: TraverseRequest
): Promise<TraverseResponse> {
  const response = await transport.requestJson<TraverseWire>(
    'POST',
    `${collectionPath(collection)}/graph/traverse`,
    {
      source: request.source,
      strategy: request.strategy ?? 'bfs',
      max_depth: request.maxDepth ?? 3,
      limit: request.limit ?? 100,
      cursor: request.cursor,
      rel_types: request.relTypes ?? [],
    }
  );

  throwOnError(response, `Collection '${collection}'`);

  return toTraverseResponse(response.data!);
}

export async function getNodeDegree(
  transport: GraphTransport,
  collection: string,
  nodeId: number
): Promise<DegreeResponse> {
  const response = await transport.requestJson<{ in_degree: number; out_degree: number }>(
    'GET',
    `${collectionPath(collection)}/graph/nodes/${nodeId}/degree`
  );

  throwOnError(response, `Collection '${collection}'`);

  return {
    inDegree: response.data?.in_degree ?? 0,
    outDegree: response.data?.out_degree ?? 0,
  };
}

export async function createGraphCollection(
  transport: GraphTransport,
  name: string,
  config?: GraphCollectionConfig
): Promise<void> {
  const response = await transport.requestJson('POST', '/collections', {
    name,
    collection_type: 'graph',
    dimension: config?.dimension,
    metric: config?.metric ?? 'cosine',
    schema_mode: config?.schemaMode ?? 'schemaless',
  });

  throwOnError(response);
}

export async function traverseParallel(
  transport: GraphTransport,
  collection: string,
  request: TraverseParallelRequest
): Promise<TraverseResponse> {
  const response = await transport.requestJson<TraverseWire>(
    'POST',
    `${collectionPath(collection)}/graph/traverse/parallel`,
    {
      sources: request.sources,
      max_depth: request.maxDepth ?? 3,
      limit: request.limit ?? 100,
      rel_types: request.relTypes ?? [],
    }
  );

  throwOnError(response, `Collection '${collection}'`);

  return toTraverseResponse(response.data!);
}

/** Wire shape of a RelateResponse from the server. */
interface RelateWire {
  edge_id: number | string;
}

/** Wire shape of a RelationEdge from the server. */
interface RelationEdgeWire {
  id: number | string;
  source: number | string;
  target: number | string;
  rel_type: string;
  properties?: Record<string, unknown>;
}

/** Wire shape of a RelationsResponse from the server. */
interface RelationsWire {
  edges: RelationEdgeWire[];
  count: number;
}

/** Create a typed relation edge between two points. Returns the allocated edge ID. */
export async function relate(
  transport: GraphTransport,
  collection: string,
  req: RelateRequest
): Promise<RelateResponse> {
  const response = await transport.requestJson<RelateWire>(
    'POST',
    `${collectionPath(collection)}/relations`,
    {
      source: req.source,
      target: req.target,
      rel_type: req.relType,
      properties: req.properties ?? {},
    }
  );

  throwOnError(response, `Collection '${collection}'`);

  return { edgeId: response.data!.edge_id };
}

/** Remove a relation edge by ID. Returns `true` if removed. */
export async function unrelate(
  transport: GraphTransport,
  collection: string,
  edgeId: GraphNodeId
): Promise<boolean> {
  const response = await transport.requestJson(
    'DELETE',
    `${collectionPath(collection)}/relations/${encodeURIComponent(String(edgeId))}`
  );

  if (response.error !== undefined) {
    const { code, message } = response.error;
    const err = parseVelesError(code, message);
    if (err instanceof EdgeNotFoundError) { return false; }
    if (code === 'NOT_FOUND') { return false; }
    throwOnError(response, `Collection '${collection}'`);
  }
  return true;
}

/** List outgoing relation edges for a point. */
export async function getRelations(
  transport: GraphTransport,
  collection: string,
  pointId: GraphNodeId
): Promise<RelationsResponse> {
  const response = await transport.requestJson<RelationsWire>(
    'GET',
    `${collectionPath(collection)}/points/${encodeURIComponent(String(pointId))}/relations`
  );

  throwOnError(response, `Collection '${collection}'`);

  const raw = response.data!;
  return {
    edges: raw.edges.map(e => ({
      id: e.id,
      source: e.source,
      target: e.target,
      relType: e.rel_type,
      properties: e.properties,
    })),
    count: raw.count,
  };
}

/** Durably set (or refresh) the TTL of a point. */
export async function setTtlDurable(
  transport: GraphTransport,
  collection: string,
  pointId: GraphNodeId,
  ttlSeconds: number
): Promise<void> {
  const response = await transport.requestJson(
    'PATCH',
    `${collectionPath(collection)}/points/${encodeURIComponent(String(pointId))}/ttl`,
    { ttl_seconds: ttlSeconds }
  );

  throwOnError(response, `Collection '${collection}'`);
}
