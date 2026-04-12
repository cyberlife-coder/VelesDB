/**
 * VelesDB Client - Graph operation methods
 *
 * Standalone functions implementing knowledge graph operations
 * (edges, traversal, degree, graph collections, and Wave 4 graph
 * endpoints) for the VelesDB client class.
 * @packageDocumentation
 */

import type {
  IVelesDBBackend,
  AddEdgeRequest,
  GetEdgesOptions,
  GraphEdge,
  TraverseRequest,
  TraverseParallelRequest,
  TraverseResponse,
  DegreeResponse,
  GraphCollectionConfig,
  ListNodesResponse,
  GetNodeEdgesOptions,
  NodePayloadResponse,
  GraphSearchRequest,
  GraphSearchResponse,
  MatchQueryOptions,
  MatchQueryResponse,
} from '../types';
import { ValidationError } from '../types';
import { requireNonEmptyString } from './validation';

/** Add an edge to the collection's knowledge graph. */
export function addEdge(
  backend: IVelesDBBackend,
  collection: string,
  edge: AddEdgeRequest
): Promise<void> {
  if (!edge.label || typeof edge.label !== 'string') {
    throw new ValidationError('Edge label is required and must be a string');
  }

  if (typeof edge.source !== 'number' || typeof edge.target !== 'number') {
    throw new ValidationError('Edge source and target must be numbers');
  }

  return backend.addEdge(collection, edge);
}

/** Get edges from the collection's knowledge graph. */
export function getEdges(
  backend: IVelesDBBackend,
  collection: string,
  options?: GetEdgesOptions
): Promise<GraphEdge[]> {
  return backend.getEdges(collection, options);
}

/** Traverse the graph using BFS or DFS from a source node. */
export function traverseGraph(
  backend: IVelesDBBackend,
  collection: string,
  request: TraverseRequest
): Promise<TraverseResponse> {
  if (typeof request.source !== 'number') {
    throw new ValidationError('Source node ID must be a number');
  }

  if (request.strategy && !['bfs', 'dfs'].includes(request.strategy)) {
    throw new ValidationError("Strategy must be 'bfs' or 'dfs'");
  }

  return backend.traverseGraph(collection, request);
}

/** Multi-source parallel BFS traversal with deduplication. */
export function traverseParallel(
  backend: IVelesDBBackend,
  collection: string,
  request: TraverseParallelRequest
): Promise<TraverseResponse> {
  if (!Array.isArray(request.sources) || request.sources.length === 0) {
    throw new ValidationError('At least one source node ID is required');
  }

  return backend.traverseParallel(collection, request);
}

/** Get the in-degree and out-degree of a node. */
export function getNodeDegree(
  backend: IVelesDBBackend,
  collection: string,
  nodeId: number
): Promise<DegreeResponse> {
  if (typeof nodeId !== 'number') {
    throw new ValidationError('Node ID must be a number');
  }

  return backend.getNodeDegree(collection, nodeId);
}

/** Create a graph collection. */
export function createGraphCollection(
  backend: IVelesDBBackend,
  name: string,
  config?: GraphCollectionConfig
): Promise<void> {
  requireNonEmptyString(name, 'Collection name');
  return backend.createGraphCollection(name, config);
}

/** Execute a VelesQL `MATCH (...)` graph query scoped to a collection. */
export function matchQuery(
  backend: IVelesDBBackend,
  collection: string,
  queryString: string,
  params?: Record<string, unknown>,
  options?: MatchQueryOptions
): Promise<MatchQueryResponse> {
  requireNonEmptyString(collection, 'Collection');
  requireNonEmptyString(queryString, 'Query string');
  return backend.matchQuery(collection, queryString, params, options);
}

/** Remove a graph edge by ID. Returns `true` if removed, `false` if not found. */
export function removeEdge(
  backend: IVelesDBBackend,
  collection: string,
  edgeId: number
): Promise<boolean> {
  requireNonEmptyString(collection, 'Collection');
  return backend.removeEdge(collection, edgeId);
}

/** Total edge count in a graph collection. */
export function getEdgeCount(
  backend: IVelesDBBackend,
  collection: string
): Promise<number> {
  requireNonEmptyString(collection, 'Collection');
  return backend.getEdgeCount(collection);
}

/** List every node ID in a graph collection. */
export function listNodes(
  backend: IVelesDBBackend,
  collection: string
): Promise<ListNodesResponse> {
  requireNonEmptyString(collection, 'Collection');
  return backend.listNodes(collection);
}

/** Get edges adjacent to a node (filterable by direction + label). */
export function getNodeEdges(
  backend: IVelesDBBackend,
  collection: string,
  nodeId: number,
  options?: GetNodeEdgesOptions
): Promise<GraphEdge[]> {
  requireNonEmptyString(collection, 'Collection');
  return backend.getNodeEdges(collection, nodeId, options);
}

/** Read the JSON payload attached to a graph node. */
export function getNodePayload(
  backend: IVelesDBBackend,
  collection: string,
  nodeId: number
): Promise<NodePayloadResponse> {
  requireNonEmptyString(collection, 'Collection');
  return backend.getNodePayload(collection, nodeId);
}

/** Upsert (create or replace) the JSON payload of a graph node. */
export function upsertNodePayload(
  backend: IVelesDBBackend,
  collection: string,
  nodeId: number,
  payload: Record<string, unknown>
): Promise<void> {
  requireNonEmptyString(collection, 'Collection');
  return backend.upsertNodePayload(collection, nodeId, payload);
}

/** Vector similarity search scoped to graph nodes only. */
export function graphSearch(
  backend: IVelesDBBackend,
  collection: string,
  request: GraphSearchRequest
): Promise<GraphSearchResponse> {
  requireNonEmptyString(collection, 'Collection');
  return backend.graphSearch(collection, request);
}
