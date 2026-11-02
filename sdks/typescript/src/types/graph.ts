/**
 * VelesDB TypeScript SDK - Graph Type Definitions
 *
 * Knowledge graph types: edges, traversal, degree, graph collections.
 * @packageDocumentation
 */

import type { DistanceMetric } from './core';

// ============================================================================
// Knowledge Graph Types (EPIC-016 US-041)
// ============================================================================

/** Graph node/edge ID. Large u64 IDs may be returned as strings to preserve precision. */
export type GraphNodeId = number | string;

/** Graph edge representing a relationship between nodes */
export interface GraphEdge {
  /** Unique edge ID */
  id: GraphNodeId;
  /** Source node ID */
  source: GraphNodeId;
  /** Target node ID */
  target: GraphNodeId;
  /** Edge label (relationship type, e.g., "KNOWS", "FOLLOWS") */
  label: string;
  /** Edge properties */
  properties?: Record<string, unknown>;
}

/**
 * Request to add an edge to the graph.
 * Structurally identical to GraphEdge -- kept as a named alias for
 * semantic clarity (input vs stored model).
 */
export type AddEdgeRequest = GraphEdge;

/** Response containing edges */
export interface EdgesResponse {
  /** List of edges */
  edges: GraphEdge[];
  /** Total count of edges returned */
  count: number;
}

/** Options for querying edges */
export interface GetEdgesOptions {
  /** Filter by edge label */
  label?: string;
}

/** Request for graph traversal (EPIC-016 US-050) */
export interface TraverseRequest {
  /** Source node ID to start traversal from */
  source: GraphNodeId;
  /** Traversal strategy: 'bfs' or 'dfs' */
  strategy?: 'bfs' | 'dfs';
  /** Maximum traversal depth */
  maxDepth?: number;
  /** Maximum number of results to return */
  limit?: number;
  /** Filter by relationship types (empty = all types) */
  relTypes?: string[];
}

/** Request for multi-source parallel BFS traversal */
export interface TraverseParallelRequest {
  /** Source node IDs to start traversal from */
  sources: GraphNodeId[];
  /** Maximum traversal depth */
  maxDepth?: number;
  /** Maximum number of results to return */
  limit?: number;
  /** Filter by relationship types (empty = all types) */
  relTypes?: string[];
}

/** A single traversal result item */
export interface TraversalResultItem {
  /** Target node ID reached */
  targetId: GraphNodeId;
  /** Depth of traversal (number of hops from source) */
  depth: number;
  /** Path taken (list of edge IDs) */
  path: GraphNodeId[];
}

/** Statistics from traversal operation */
export interface TraversalStats {
  /** Number of nodes visited */
  visited: number;
  /** Maximum depth reached */
  depthReached: number;
}

/** Response from graph traversal */
export interface TraverseResponse {
  /** List of traversal results */
  results: TraversalResultItem[];
  /**
   * Whether the traversal stopped at `limit` (more nodes may be reachable).
   * There is no traversal cursor: raise `limit` to see further.
   */
  hasMore: boolean;
  /** Traversal statistics */
  stats: TraversalStats;
}

/** Response for node degree query */
export interface DegreeResponse {
  /** Number of incoming edges */
  inDegree: number;
  /** Number of outgoing edges */
  outDegree: number;
}

// ============================================================================
// Relation API Types (REST parity)
// ============================================================================

/** Request body for POST /collections/{name}/relations */
export interface RelateRequest {
  /** Source point ID */
  source: GraphNodeId;
  /** Target point ID */
  target: GraphNodeId;
  /** Relationship type label (e.g. "KNOWS", "RELATED_TO") */
  relType: string;
  /** Optional edge properties */
  properties?: Record<string, unknown>;
}

/** Response from POST /collections/{name}/relations */
export interface RelateResponse {
  /** Allocated edge ID */
  edgeId: GraphNodeId;
}

/** A single outgoing relation edge */
export interface RelationEdge {
  /** Edge ID */
  id: GraphNodeId;
  /** Source point ID */
  source: GraphNodeId;
  /** Target point ID */
  target: GraphNodeId;
  /** Relationship type label */
  relType: string;
  /** Edge properties */
  properties?: Record<string, unknown>;
}

/** Response from GET /collections/{name}/points/{id}/relations */
export interface RelationsResponse {
  /** Outgoing relation edges */
  edges: RelationEdge[];
  /** Total count */
  count: number;
}

// ============================================================================
// Graph Collection Types (Phase 8)
// ============================================================================

/** Schema mode for graph collections */
export type GraphSchemaMode = 'schemaless' | 'strict';

/** Property value type in a graph schema (`velesdb_core::ValueType`, serialized lowercase). */
export type GraphValueType = 'string' | 'integer' | 'float' | 'boolean' | 'vector';

/** A node type a strict graph accepts (`velesdb_core::NodeType`). */
export interface GraphNodeType {
  /** Node label. */
  name: string;
  /** Declared properties and their types (default: none). */
  properties?: Record<string, GraphValueType>;
}

/** An edge type a strict graph accepts (`velesdb_core::EdgeType`). */
export interface GraphEdgeType {
  /** Edge label. */
  name: string;
  /** Node type the edge leaves; must be one of `nodeTypes`. */
  fromType: string;
  /** Node type the edge reaches; must be one of `nodeTypes`. */
  toType: string;
  /** Declared properties and their types (default: none). */
  properties?: Record<string, GraphValueType>;
}

/** Graph collection configuration */
export interface GraphCollectionConfig {
  /** Optional embedding dimension for node vectors */
  dimension?: number;
  /** Distance metric for embeddings (default: 'cosine') */
  metric?: DistanceMetric;
  /**
   * Schema mode (default: 'schemaless'). Sent with `nodeTypes` and
   * `edgeTypes` as core's `graph_schema`. In a `'strict'` graph core checks
   * every label in a node payload's `_labels` against `nodeTypes` (a node
   * with no `_labels` is not checked), and refuses an edge unless both
   * endpoints exist and carry a label, and its label and endpoint types
   * (each endpoint's first `_labels` entry) match a declared `edgeTypes`
   * entry whose endpoint types are declared (`velesdb_core::GraphSchema::
   * validate_node_type` / `validate_edge_type`).
   */
  schemaMode?: GraphSchemaMode;
  /** Node types a `'strict'` graph accepts. Refused on a schemaless graph, which ignores them. */
  nodeTypes?: GraphNodeType[];
  /** Edge types a `'strict'` graph accepts. Refused on a schemaless graph, which ignores them. */
  edgeTypes?: GraphEdgeType[];
}
