/**
 * VelesDB TypeScript SDK - Type Definitions
 * @packageDocumentation
 */

/** Supported distance metrics for vector similarity */
export type DistanceMetric = 'cosine' | 'euclidean' | 'dot' | 'hamming' | 'jaccard';

/** Storage mode for vector quantization */
export type StorageMode = 'full' | 'sq8' | 'binary';

/** Backend type for VelesDB connection */
export type BackendType = 'wasm' | 'rest';

/** Configuration options for VelesDB client */
export interface VelesDBConfig {
  /** Backend type: 'wasm' for browser/Node.js, 'rest' for server */
  backend: BackendType;
  /** REST API URL (required for 'rest' backend) */
  url?: string;
  /** API key for authentication (optional) */
  apiKey?: string;
  /** Request timeout in milliseconds (default: 30000) */
  timeout?: number;
}

/** Collection type */
export type CollectionType = 'vector' | 'metadata_only';

/** Collection configuration */
export interface CollectionConfig {
  /** Vector dimension (e.g., 768 for BERT, 1536 for GPT). Required for vector collections. */
  dimension?: number;
  /** Distance metric (default: 'cosine') */
  metric?: DistanceMetric;
  /** Storage mode for vector quantization (default: 'full')
   * - 'full': Full f32 precision (3 KB/vector for 768D)
   * - 'sq8': 8-bit scalar quantization, 4x memory reduction (~1% recall loss)
   * - 'binary': 1-bit binary quantization, 32x memory reduction (edge/IoT)
   */
  storageMode?: StorageMode;
  /** Collection type: 'vector' (default) or 'metadata_only' */
  collectionType?: CollectionType;
  /** Optional collection description */
  description?: string;
}

/** Collection metadata */
export interface Collection {
  /** Collection name */
  name: string;
  /** Vector dimension */
  dimension: number;
  /** Distance metric */
  metric: DistanceMetric;
  /** Storage mode */
  storageMode?: StorageMode;
  /** Number of vectors */
  count: number;
  /** Creation timestamp */
  createdAt?: Date;
}

/** Vector document to insert */
export interface VectorDocument {
  /** Unique identifier */
  id: string | number;
  /** Vector data */
  vector: number[] | Float32Array;
  /** Optional payload/metadata */
  payload?: Record<string, unknown>;
}

/** Search options */
export interface SearchOptions {
  /** Number of results to return (default: 10) */
  k?: number;
  /** Filter expression (optional) */
  filter?: Record<string, unknown>;
  /** Include vectors in results (default: false) */
  includeVectors?: boolean;
}

/** Fusion strategy for multi-query search */
export type FusionStrategy = 'rrf' | 'average' | 'maximum' | 'weighted';

/** Multi-query search options */
export interface MultiQuerySearchOptions {
  /** Number of results to return (default: 10) */
  k?: number;
  /** Fusion strategy (default: 'rrf') */
  fusion?: FusionStrategy;
  /** Fusion parameters */
  fusionParams?: {
    /** RRF k parameter (default: 60) */
    k?: number;
    /** Weighted fusion: average weight (default: 0.6) */
    avgWeight?: number;
    /** Weighted fusion: max weight (default: 0.3) */
    maxWeight?: number;
    /** Weighted fusion: hit weight (default: 0.1) */
    hitWeight?: number;
  };
  /** Filter expression (optional) */
  filter?: Record<string, unknown>;
}

/** Search result */
export interface SearchResult {
  /** Document ID */
  id: string | number;
  /** Similarity score */
  score: number;
  /** Document payload (if requested) */
  payload?: Record<string, unknown>;
  /** Vector data (if includeVectors is true) */
  vector?: number[];
}

// ============================================================================
// Knowledge Graph Types (EPIC-016 US-041)
// ============================================================================

/** Graph edge representing a relationship between nodes */
export interface GraphEdge {
  /** Unique edge ID */
  id: number;
  /** Source node ID */
  source: number;
  /** Target node ID */
  target: number;
  /** Edge label (relationship type, e.g., "KNOWS", "FOLLOWS") */
  label: string;
  /** Edge properties */
  properties?: Record<string, unknown>;
}

/** Request to add an edge to the graph */
export interface AddEdgeRequest {
  /** Unique edge ID */
  id: number;
  /** Source node ID */
  source: number;
  /** Target node ID */
  target: number;
  /** Edge label (relationship type) */
  label: string;
  /** Edge properties (optional) */
  properties?: Record<string, unknown>;
}

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
  source: number;
  /** Traversal strategy: 'bfs' or 'dfs' */
  strategy?: 'bfs' | 'dfs';
  /** Maximum traversal depth */
  maxDepth?: number;
  /** Maximum number of results to return */
  limit?: number;
  /** Optional cursor for pagination */
  cursor?: string;
  /** Filter by relationship types (empty = all types) */
  relTypes?: string[];
}

/** A single traversal result item */
export interface TraversalResultItem {
  /** Target node ID reached */
  targetId: number;
  /** Depth of traversal (number of hops from source) */
  depth: number;
  /** Path taken (list of edge IDs) */
  path: number[];
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
  /** Cursor for next page (if applicable) */
  nextCursor?: string;
  /** Whether more results are available */
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
// VelesQL Multi-Model Query Types (EPIC-031 US-011)
// ============================================================================

/** VelesQL query options */
export interface QueryOptions {
  /** Timeout in milliseconds (default: 30000) */
  timeoutMs?: number;
  /** Enable streaming response */
  stream?: boolean;
  /** Optional vector for MATCH queries with similarity matching (pass-through to matchQuery) */
  vector?: number[] | Float32Array;
  /** Similarity threshold for MATCH queries (pass-through to matchQuery) */
  threshold?: number;
}

/** Query result from multi-model VelesQL query */
export interface QueryResult {
  /** Node/point ID */
  nodeId: bigint | number;
  /** Vector similarity score (if applicable) */
  vectorScore: number | null;
  /** Graph relevance score (if applicable) */
  graphScore: number | null;
  /** Combined fused score */
  fusedScore: number;
  /** Variable bindings from MATCH clause */
  bindings: Record<string, unknown>;
  /** Column data from JOIN (if applicable) */
  columnData: Record<string, unknown> | null;
}

/** Query execution statistics */
export interface QueryStats {
  /** Execution time in milliseconds */
  executionTimeMs: number;
  /** Execution strategy used */
  strategy: string;
  /** Number of nodes scanned */
  scannedNodes: number;
}

/** Full query response with results and stats */
export interface QueryResponse {
  /** Query results */
  results: QueryResult[];
  /** Execution statistics */
  stats: QueryStats;
}

// ============================================================================
// Index Management Types (EPIC-009)
// ============================================================================

/** Index type for property indexes */
export type IndexType = 'hash' | 'range';

/** Index information */
export interface IndexInfo {
  /** Node label (e.g., "Person") */
  label: string;
  /** Property name (e.g., "email") */
  property: string;
  /** Index type: 'hash' for O(1) equality, 'range' for O(log n) range queries */
  indexType: IndexType;
  /** Number of unique values indexed (for hash indexes) */
  cardinality: number;
  /** Memory usage in bytes */
  memoryBytes: number;
}

/** Options for creating an index */
export interface CreateIndexOptions {
  /** Node label to index */
  label: string;
  /** Property name to index */
  property: string;
  /** Index type: 'hash' (default) or 'range' */
  indexType?: IndexType;
}

/** Backend interface that all backends must implement */
export interface IVelesDBBackend {
  /** Initialize the backend */
  init(): Promise<void>;
  
  /** Check if backend is initialized */
  isInitialized(): boolean;
  
  /** Create a new collection */
  createCollection(name: string, config: CollectionConfig): Promise<void>;
  
  /** Delete a collection */
  deleteCollection(name: string): Promise<void>;
  
  /** Get collection info */
  getCollection(name: string): Promise<Collection | null>;
  
  /** List all collections */
  listCollections(): Promise<Collection[]>;
  
  /** Insert a single vector */
  insert(collection: string, doc: VectorDocument): Promise<void>;
  
  /** Insert multiple vectors */
  insertBatch(collection: string, docs: VectorDocument[]): Promise<void>;
  
  /** Search for similar vectors */
  search(
    collection: string,
    query: number[] | Float32Array,
    options?: SearchOptions
  ): Promise<SearchResult[]>;
  
  /** Delete a vector by ID */
  delete(collection: string, id: string | number): Promise<boolean>;
  
  /** Get a vector by ID */
  get(collection: string, id: string | number): Promise<VectorDocument | null>;

  /** Search for multiple vectors in batch */
  searchBatch(
    collection: string,
    searches: Array<{
      vector: number[] | Float32Array;
      k?: number;
      filter?: Record<string, unknown>;
    }>
  ): Promise<SearchResult[][]>;
  
  /** Full-text search using BM25 */
  textSearch(
    collection: string,
    query: string,
    options?: { k?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]>;
  
  /** Hybrid search combining vector and text */
  hybridSearch(
    collection: string,
    vector: number[] | Float32Array,
    textQuery: string,
    options?: { k?: number; vectorWeight?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]>;
  
  /** Execute VelesQL multi-model query (EPIC-031 US-011) */
  query(
    collection: string,
    queryString: string,
    params?: Record<string, unknown>,
    options?: QueryOptions
  ): Promise<QueryResponse>;

  /** Multi-query fusion search */
  multiQuerySearch(
    collection: string,
    vectors: Array<number[] | Float32Array>,
    options?: MultiQuerySearchOptions
  ): Promise<SearchResult[]>;
  
  /** Check if collection is empty */
  isEmpty(collection: string): Promise<boolean>;
  
  /** Flush pending changes to disk */
  flush(collection: string): Promise<void>;
  
  /** Close/cleanup the backend */
  close(): Promise<void>;

  // Index Management (EPIC-009)
  
  /** Create a property index for O(1) equality lookups */
  createIndex(collection: string, options: CreateIndexOptions): Promise<void>;
  
  /** List all indexes on a collection */
  listIndexes(collection: string): Promise<IndexInfo[]>;
  
  /** Check if an index exists */
  hasIndex(collection: string, label: string, property: string): Promise<boolean>;
  
  /** Drop an index */
  dropIndex(collection: string, label: string, property: string): Promise<boolean>;

  // Knowledge Graph (EPIC-016 US-041, US-050)
  
  /** Add an edge to the collection's knowledge graph */
  addEdge(collection: string, edge: AddEdgeRequest): Promise<void>;
  
  /** Get edges from the collection's knowledge graph */
  getEdges(collection: string, options?: GetEdgesOptions): Promise<GraphEdge[]>;

  /** Traverse the graph using BFS or DFS from a source node */
  traverseGraph(collection: string, request: TraverseRequest): Promise<TraverseResponse>;

  /** Get the in-degree and out-degree of a node */
  getNodeDegree(collection: string, nodeId: number): Promise<DegreeResponse>;

  // MATCH Query (Graph Traversal via VelesQL)

  /** Execute a MATCH graph traversal query */
  matchQuery(
    collection: string,
    queryString: string,
    params?: Record<string, unknown>,
    options?: MatchQueryOptions
  ): Promise<MatchQueryResponse>;
}

// ============================================================================
// MATCH Query Types
// ============================================================================

/** Options for MATCH graph traversal queries */
export interface MatchQueryOptions {
  /** Optional vector for similarity matching within MATCH */
  vector?: number[] | Float32Array;
  /** Similarity threshold for vector matching */
  threshold?: number;
}

/** A single result item from a MATCH query */
export interface MatchQueryResultItem {
  /** Variable bindings from MATCH pattern (e.g., { a: 123, b: 456 }) */
  bindings: Record<string, unknown>;
  /** Similarity score (if vector matching was used) */
  score: number | null;
  /** Traversal depth */
  depth: number;
  /** Projected fields from RETURN clause */
  projected: Record<string, unknown>;
}

/** Full response from a MATCH query */
export interface MatchQueryResponse {
  /** MATCH query results */
  results: MatchQueryResultItem[];
  /** Execution time in milliseconds */
  tookMs: number;
  /** Total result count */
  count: number;
}

/** Error types */
export class VelesDBError extends Error {
  constructor(
    message: string,
    public readonly code: string,
    public readonly cause?: Error
  ) {
    super(message);
    this.name = 'VelesDBError';
  }
}

export class ConnectionError extends VelesDBError {
  constructor(message: string, cause?: Error) {
    super(message, 'CONNECTION_ERROR', cause);
    this.name = 'ConnectionError';
  }
}

export class ValidationError extends VelesDBError {
  constructor(message: string) {
    super(message, 'VALIDATION_ERROR');
    this.name = 'ValidationError';
  }
}

export class NotFoundError extends VelesDBError {
  constructor(resource: string) {
    super(`${resource} not found`, 'NOT_FOUND');
    this.name = 'NotFoundError';
  }
}
