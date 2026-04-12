/**
 * VelesDB TypeScript SDK - Additional Endpoint Type Definitions
 *
 * Types for Sprint 2 Wave 4 endpoints: rebuild index, guardrails,
 * aggregate, match query, graph node operations, and graph search.
 * @packageDocumentation
 */

// ============================================================================
// Additional endpoint types (Sprint 2 Wave 4 -- S2-NEW-10)
// ============================================================================

/** Result of `POST /collections/{name}/index/rebuild`. */
export interface RebuildIndexResponse {
  /** Informational message from the server. */
  message: string;
  /** Collection name. */
  collection: string;
  /** Number of tombstoned entries compacted during rebuild. */
  compactedEntries: number;
}

/** Guard-rails config sent to `PUT /guardrails` (partial update). */
export interface GuardRailsUpdateRequest {
  maxDepth?: number;
  maxCardinality?: number;
  memoryLimitBytes?: number;
  timeoutMs?: number;
  rateLimitQps?: number;
  circuitFailureThreshold?: number;
  circuitRecoverySeconds?: number;
}

/** Guard-rails config returned by `GET /guardrails` and `PUT /guardrails`. */
export interface GuardRailsConfigResponse {
  maxDepth: number;
  maxCardinality: number;
  memoryLimitBytes: number;
  timeoutMs: number;
  rateLimitQps: number;
  circuitFailureThreshold: number;
  circuitRecoverySeconds: number;
}

/** Options for `listNodes`. */
export interface ListNodesResponse {
  /** Node IDs in insertion order. */
  nodeIds: number[];
  /** Total count -- matches `nodeIds.length`. */
  count: number;
}

/** Options for `getNodeEdges`. Mirrors `NodeEdgeQueryParams` on the server. */
export interface GetNodeEdgesOptions {
  /** Edge direction: "in", "out" (default), or "both". */
  direction?: 'in' | 'out' | 'both';
  /** Optional label filter. */
  label?: string;
}

/** Result of `GET /collections/{name}/graph/nodes/{id}/payload`. */
export interface NodePayloadResponse {
  /** Node ID. */
  nodeId: number;
  /** Stored payload -- `null` if no payload has been set. */
  payload: Record<string, unknown> | null;
}

/** Request body for `POST /collections/{name}/graph/search`. */
export interface GraphSearchRequest {
  /** Query vector for embedding similarity. */
  vector: number[] | Float32Array;
  /** Number of results (default: 10). */
  k?: number;
}

/** Single result item from `graphSearch`. */
export interface GraphSearchResultItem {
  /** Node ID. */
  id: number;
  /** Similarity score. */
  score: number;
  /** Optional node payload (mirror of `GraphSearchResultItem.payload`). */
  payload?: Record<string, unknown> | null;
}

/** Response of `graphSearch`. */
export interface GraphSearchResponse {
  /** Result items ordered by score. */
  results: GraphSearchResultItem[];
}

/**
 * Options for `matchQuery`. Mirrors the extra fields accepted by
 * `velesdb_server::handlers::match_query::MatchQueryRequest`
 * beyond `query` and `params`.
 */
export interface MatchQueryOptions {
  /** Query vector for `similarity()` scoring inside the MATCH clause. */
  vector?: number[] | Float32Array;
  /** Similarity threshold (0.0-1.0). */
  threshold?: number;
}

/** Response from `POST /collections/{name}/match`. Mirrors the Rust
 * `MatchQueryResponse` struct -- intentionally distinct from the
 * `/query` and `/aggregate` response shapes. */
export interface MatchQueryResponse {
  /** Pattern matches returned by the MATCH clause. */
  results: MatchQueryResultItem[];
  /** Server-side execution time in whole milliseconds. */
  tookMs: number;
  /** Number of result rows (matches `results.length`). */
  count: number;
  /** Response metadata (VelesQL contract version). */
  meta: { velesqlContractVersion: string };
}

/** Single row of a `MatchQueryResponse`. */
export interface MatchQueryResultItem {
  /** Variable-binding map from the MATCH pattern. */
  bindings: Record<string, number>;
  /** Similarity score, present only when `similarity()` was used. */
  score?: number;
  /** Traversal depth reached to produce this row. */
  depth: number;
  /** Projected properties from the RETURN clause. */
  projected: Record<string, unknown>;
}

/** Options for `aggregate`. Mirrors the extra fields accepted by
 * `velesdb_core::api_types::QueryRequest` beyond `query` and `params`. */
export interface AggregateQueryOptions {
  /**
   * Optional collection name when the query string does not carry an
   * explicit `FROM <collection>` clause.
   */
  collection?: string;
}

/** Response from `POST /aggregate`. Mirrors the Rust `AggregationResponse`. */
export interface AggregateResponse {
  /** Aggregation result -- shape depends on the SELECT clause. */
  result: unknown;
  /** Query execution time in milliseconds. */
  timingMs: number;
  /** Response metadata. */
  meta: { velesqlContractVersion: string; count: number };
}
