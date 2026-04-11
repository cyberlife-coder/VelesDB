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
  TraverseRequest,
  TraverseParallelRequest,
  TraverseResponse,
  DegreeResponse,
  GraphCollectionConfig,
} from '../types';
import type { BaseTransport } from './shared';
import { throwOnError, collectionPath } from './shared';

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
 * The `toGraphEdge` helper coerces them back to the `number` shape
 * declared on the public `GraphEdge` interface (PR #586 fix applied
 * to the pre-existing `getEdges` wrapper for consistency with the
 * `getNodeEdges` fix landed earlier in the same PR).
 */
interface EdgeWire {
  id: number | string;
  source: number | string;
  target: number | string;
  label: string;
  properties?: Record<string, unknown>;
}

function toGraphEdge(e: EdgeWire): GraphEdge {
  const toNum = (v: number | string): number =>
    typeof v === 'string' ? Number(v) : v;
  return {
    id: toNum(e.id),
    source: toNum(e.source),
    target: toNum(e.target),
    label: e.label,
    properties: e.properties,
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
  const response = await transport.requestJson<{
    results: Array<{ target_id: number; depth: number; path: number[] }>;
    next_cursor: string | null;
    has_more: boolean;
    stats: { visited: number; depth_reached: number };
  }>(
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

  const data = response.data!;
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
  const response = await transport.requestJson<{
    results: Array<{ target_id: number; depth: number; path: number[] }>;
    next_cursor: string | null;
    has_more: boolean;
    stats: { visited: number; depth_reached: number };
  }>(
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

  const data = response.data!;
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
