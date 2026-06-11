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
  GraphNodeId,
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
  RelateRequest,
  RelateResponse,
  RelationsResponse,
} from '../types';
import { ValidationError } from '../types';
import { requireNonEmptyString } from './validation';

function isGraphNodeId(value: unknown): value is GraphNodeId {
  return typeof value === 'number' || typeof value === 'string';
}

/** Add an edge to the collection's knowledge graph. */
export function addEdge(
  backend: IVelesDBBackend,
  collection: string,
  edge: AddEdgeRequest
): Promise<void> {
  if (!edge.label || typeof edge.label !== 'string') {
    throw new ValidationError('Edge label is required and must be a string');
  }

  if (!isGraphNodeId(edge.source) || !isGraphNodeId(edge.target)) {
    throw new ValidationError('Edge source and target must be numbers or strings');
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
  if (!isGraphNodeId(request.source)) {
    throw new ValidationError('Source node ID must be a number or string');
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
  if (!request.sources.every(isGraphNodeId)) {
    throw new ValidationError('Source node IDs must be numbers or strings');
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

/** Create a typed relation edge between two points. Returns the allocated edge ID. */
export function relate(
  backend: IVelesDBBackend,
  collection: string,
  req: RelateRequest
): Promise<RelateResponse> {
  requireNonEmptyString(collection, 'Collection');
  if (!req.relType || typeof req.relType !== 'string') {
    throw new ValidationError('Relation type is required and must be a string');
  }
  if (!isGraphNodeId(req.source) || !isGraphNodeId(req.target)) {
    throw new ValidationError('Source and target must be numbers or strings');
  }
  return backend.relate(collection, req);
}

/** Remove a relation edge by ID. Returns `true` if removed. */
export function unrelate(
  backend: IVelesDBBackend,
  collection: string,
  edgeId: GraphNodeId
): Promise<boolean> {
  requireNonEmptyString(collection, 'Collection');
  if (!isGraphNodeId(edgeId)) {
    throw new ValidationError('Edge ID must be a number or string');
  }
  return backend.unrelate(collection, edgeId);
}

/** List outgoing relation edges for a point. */
export function getRelations(
  backend: IVelesDBBackend,
  collection: string,
  pointId: GraphNodeId
): Promise<RelationsResponse> {
  requireNonEmptyString(collection, 'Collection');
  if (!isGraphNodeId(pointId)) {
    throw new ValidationError('Point ID must be a number or string');
  }
  return backend.getRelations(collection, pointId);
}

/** Durably set (or refresh) the TTL of a point. */
export function setTtlDurable(
  backend: IVelesDBBackend,
  collection: string,
  pointId: GraphNodeId,
  ttlSeconds: number
): Promise<void> {
  requireNonEmptyString(collection, 'Collection');
  if (!isGraphNodeId(pointId)) {
    throw new ValidationError('Point ID must be a number or string');
  }
  if (typeof ttlSeconds !== 'number' || ttlSeconds < 0) {
    throw new ValidationError('ttlSeconds must be a non-negative number');
  }
  return backend.setTtlDurable(collection, pointId, ttlSeconds);
}
