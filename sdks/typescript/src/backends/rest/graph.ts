/**
 * Knowledge Graph operations for REST backend (EPIC-016)
 */

import type {
  AddEdgeRequest,
  GetEdgesOptions,
  GraphEdge,
  TraverseRequest,
  TraverseResponse,
  DegreeResponse,
} from '../../types';
import { NotFoundError, VelesDBError } from '../../types';
import type { HttpClient } from './http-client';

export async function addEdge(
  client: HttpClient, collection: string, edge: AddEdgeRequest
): Promise<void> {
  client.ensureInitialized();

  const response = await client.request(
    'POST',
    `/collections/${encodeURIComponent(collection)}/graph/edges`,
    {
      id: edge.id,
      source: edge.source,
      target: edge.target,
      label: edge.label,
      properties: edge.properties ?? {},
    }
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }
}

export async function getEdges(
  client: HttpClient, collection: string, options?: GetEdgesOptions
): Promise<GraphEdge[]> {
  client.ensureInitialized();

  const queryParams = options?.label ? `?label=${encodeURIComponent(options.label)}` : '';

  const response = await client.request<{ edges: GraphEdge[]; count: number }>(
    'GET',
    `/collections/${encodeURIComponent(collection)}/graph/edges${queryParams}`
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return response.data?.edges ?? [];
}

export async function traverseGraph(
  client: HttpClient, collection: string, request: TraverseRequest
): Promise<TraverseResponse> {
  client.ensureInitialized();

  const response = await client.request<{
    results: Array<{ target_id: number; depth: number; path: number[] }>;
    next_cursor: string | null;
    has_more: boolean;
    stats: { visited: number; depth_reached: number };
  }>(
    'POST',
    `/collections/${encodeURIComponent(collection)}/graph/traverse`,
    {
      source: request.source,
      strategy: request.strategy ?? 'bfs',
      max_depth: request.maxDepth ?? 3,
      limit: request.limit ?? 100,
      cursor: request.cursor,
      rel_types: request.relTypes ?? [],
    }
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
      targetId: r.target_id,
      depth: r.depth,
      path: r.path,
    })),
    nextCursor: data?.next_cursor ?? undefined,
    hasMore: data?.has_more ?? false,
    stats: {
      visited: data?.stats?.visited ?? 0,
      depthReached: data?.stats?.depth_reached ?? 0,
    },
  };
}

export async function getNodeDegree(
  client: HttpClient, collection: string, nodeId: number
): Promise<DegreeResponse> {
  client.ensureInitialized();

  const response = await client.request<{ in_degree: number; out_degree: number }>(
    'GET',
    `/collections/${encodeURIComponent(collection)}/graph/nodes/${nodeId}/degree`
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return {
    inDegree: response.data?.in_degree ?? 0,
    outDegree: response.data?.out_degree ?? 0,
  };
}
