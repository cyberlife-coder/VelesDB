/**
 * VelesDB TypeScript SDK - Backend Interface
 *
 * The `IVelesDBBackend` interface that all backends must implement.
 * @packageDocumentation
 */

import type { FilterInput } from '../filter';
import type { CapabilityMap } from '../capabilities';
import type {
  CollectionConfig,
  Collection,
  VectorDocument,
  SearchQuality,
  PqTrainOptions,
} from './core';
import type {
  SearchOptions,
  SearchResult,
  MultiQuerySearchOptions,
} from './search';
import type {
  GraphEdge,
  AddEdgeRequest,
  GetEdgesOptions,
  TraverseRequest,
  TraverseParallelRequest,
  TraverseResponse,
  DegreeResponse,
  GraphCollectionConfig,
} from './graph';
import type {
  SemanticEntry,
  EpisodicEvent,
  ProceduralPattern,
} from './agent';
import type {
  ScrollRequest,
  ScrollResponse,
  CollectionStatsResponse,
  CollectionConfigResponse,
  QueryOptions,
  QueryApiResponse,
  ExplainResponse,
  CollectionSanityResponse,
} from './query';
import type {
  CreateIndexOptions,
  IndexInfo,
} from './index-types';
import type {
  RebuildIndexResponse,
  GuardRailsUpdateRequest,
  GuardRailsConfigResponse,
  ListNodesResponse,
  GetNodeEdgesOptions,
  NodePayloadResponse,
  GraphSearchRequest,
  GraphSearchResponse,
  MatchQueryOptions,
  MatchQueryResponse,
  AggregateQueryOptions,
  AggregateResponse,
} from './endpoints';

/** Backend interface that all backends must implement */
export interface IVelesDBBackend {
  /** Initialize the backend */
  init(): Promise<void>;

  /** Check if backend is initialized */
  isInitialized(): boolean;

  /**
   * Return the static capability map for this backend.
   *
   * The map is frozen at backend construction -- it does NOT round-trip
   * to a live server. Use it to gracefully degrade UI / workflow when
   * a feature is not available instead of catching a runtime
   * `NOT_SUPPORTED` error after the fact.
   */
  capabilities(): Readonly<CapabilityMap>;

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
      filter?: FilterInput;
      /** Per-sub-request search quality preset (default: server default). */
      quality?: SearchQuality;
    }>
  ): Promise<SearchResult[][]>;

  /** Full-text search using BM25 */
  textSearch(
    collection: string,
    query: string,
    options?: { k?: number; filter?: FilterInput }
  ): Promise<SearchResult[]>;

  /** Hybrid search combining vector and text */
  hybridSearch(
    collection: string,
    vector: number[] | Float32Array,
    textQuery: string,
    options?: { k?: number; vectorWeight?: number; filter?: FilterInput }
  ): Promise<SearchResult[]>;

  /** Execute VelesQL multi-model query (EPIC-031 US-011) */
  query(
    collection: string,
    queryString: string,
    params?: Record<string, unknown>,
    options?: QueryOptions
  ): Promise<QueryApiResponse>;

  /** Explain a VelesQL query without executing it */
  queryExplain(queryString: string, params?: Record<string, unknown>, options?: { analyze?: boolean }): Promise<ExplainResponse>;

  /** Scroll through collection points with cursor-based pagination */
  scroll(collection: string, request?: ScrollRequest): Promise<ScrollResponse>;

  /** Run collection sanity checks */
  collectionSanity(collection: string): Promise<CollectionSanityResponse>;

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

  /** Multi-source parallel BFS traversal with deduplication */
  traverseParallel(collection: string, request: TraverseParallelRequest): Promise<TraverseResponse>;

  /** Get the in-degree and out-degree of a node */
  getNodeDegree(collection: string, nodeId: number): Promise<DegreeResponse>;

  // Sparse / PQ / Streaming (v1.5)

  /** Train Product Quantization on a collection */
  trainPq(collection: string, options?: PqTrainOptions): Promise<string>;

  /** Stream-insert documents with backpressure support */
  streamInsert(collection: string, docs: VectorDocument[]): Promise<void>;

  // Graph Collection Management (Phase 8)

  /** Create a graph collection */
  createGraphCollection(name: string, config?: GraphCollectionConfig): Promise<void>;

  // Collection Stats / Config (Phase 8)

  /** Get collection statistics */
  getCollectionStats(collection: string): Promise<CollectionStatsResponse | null>;

  /** Analyze a collection */
  analyzeCollection(collection: string): Promise<CollectionStatsResponse>;

  /** Get collection configuration */
  getCollectionConfig(collection: string): Promise<CollectionConfigResponse>;

  /** Search returning only IDs and scores */
  searchIds(
    collection: string,
    query: number[] | Float32Array,
    options?: SearchOptions
  ): Promise<Array<{ id: number; score: number }>>;

  // Agent Memory (Phase 8)

  /** Store a semantic fact */
  storeSemanticFact(collection: string, entry: SemanticEntry): Promise<void>;

  /** Search semantic memory */
  searchSemanticMemory(
    collection: string,
    embedding: number[],
    k?: number
  ): Promise<SearchResult[]>;

  /** Record an episodic event */
  recordEpisodicEvent(collection: string, event: EpisodicEvent): Promise<void>;

  /** Recall episodic events */
  recallEpisodicEvents(
    collection: string,
    embedding: number[],
    k?: number
  ): Promise<SearchResult[]>;

  /** Store a procedural pattern */
  storeProceduralPattern(collection: string, pattern: ProceduralPattern): Promise<void>;

  /** Match procedural patterns */
  matchProceduralPatterns(
    collection: string,
    embedding: number[],
    k?: number
  ): Promise<SearchResult[]>;

  // Sprint 2 Wave 4 -- S2-NEW-10: missing REST endpoint wrappers

  /** Rebuild a collection's HNSW index (compacts tombstones). */
  rebuildIndex(collection: string): Promise<RebuildIndexResponse>;

  /** Read the current process-wide guard-rails configuration. */
  getGuardrails(): Promise<GuardRailsConfigResponse>;

  /** Partial-update the process-wide guard-rails configuration. */
  updateGuardrails(req: GuardRailsUpdateRequest): Promise<GuardRailsConfigResponse>;

  /** Execute a VelesQL aggregate query (COUNT/AVG/GROUP BY/...). */
  aggregate(
    queryString: string,
    params?: Record<string, unknown>,
    options?: AggregateQueryOptions
  ): Promise<AggregateResponse>;

  /** Execute a VelesQL `MATCH (...)` graph query scoped to a collection. */
  matchQuery(
    collection: string,
    queryString: string,
    params?: Record<string, unknown>,
    options?: MatchQueryOptions
  ): Promise<MatchQueryResponse>;

  /** Remove a graph edge by ID. Returns `true` if removed, `false` if not found. */
  removeEdge(collection: string, edgeId: number): Promise<boolean>;

  /** Total edge count in a graph collection. */
  getEdgeCount(collection: string): Promise<number>;

  /** List every node ID in a graph collection. */
  listNodes(collection: string): Promise<ListNodesResponse>;

  /** Get edges adjacent to a node (filterable by direction + label). */
  getNodeEdges(
    collection: string,
    nodeId: number,
    options?: GetNodeEdgesOptions
  ): Promise<GraphEdge[]>;

  /** Read the JSON payload attached to a graph node. */
  getNodePayload(collection: string, nodeId: number): Promise<NodePayloadResponse>;

  /** Upsert (create or replace) the JSON payload of a graph node. */
  upsertNodePayload(
    collection: string,
    nodeId: number,
    payload: Record<string, unknown>
  ): Promise<void>;

  /** Vector similarity search scoped to graph nodes only. */
  graphSearch(
    collection: string,
    request: GraphSearchRequest
  ): Promise<GraphSearchResponse>;
}
