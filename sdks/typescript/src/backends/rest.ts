/**
 * REST Backend for VelesDB
 *
 * Connects to VelesDB server via REST API.
 * This is the composition root that delegates to focused backend modules.
 * HTTP infrastructure lives in rest-http.ts.
 */

import type {
  IVelesDBBackend,
  CollectionConfig,
  Collection,
  VectorDocument,
  SearchOptions,
  SearchQuality,
  SearchResult,
  MultiQuerySearchOptions,
  CreateIndexOptions,
  IndexInfo,
  AddEdgeRequest,
  GetEdgesOptions,
  GraphEdge,
  TraverseRequest,
  TraverseParallelRequest,
  TraverseResponse,
  DegreeResponse,
  QueryOptions,
  QueryApiResponse,
  ExplainResponse,
  CollectionSanityResponse,
  PqTrainOptions,
  GraphCollectionConfig,
  CollectionStatsResponse,
  CollectionConfigResponse,
  SemanticEntry,
  EpisodicEvent,
  ProceduralPattern,
  ScrollRequest,
  ScrollResponse,
  RebuildIndexResponse,
  GuardRailsUpdateRequest,
  GuardRailsConfigResponse,
  ListNodesResponse,
  GetNodeEdgesOptions,
  NodePayloadResponse,
  GraphSearchResponse,
  MatchQueryOptions,
  MatchQueryResponse,
  AggregateQueryOptions,
  AggregateResponse,
  StreamUpsertResponse,
} from '../types';
import type { FilterInput } from '../filter';
import type { CapabilityMap } from '../capabilities';
import { REST_CAPABILITIES } from '../capabilities';
import { ConnectionError } from '../types';
import type { GraphSearchRequest as GraphSearchReq } from '../types';

// HTTP infrastructure & transport adapters
import {
  request,
  buildBaseTransport,
  buildCrudTransport,
  buildSearchTransport,
  buildQueryTransport,
  buildStreamingTransport,
  buildAgentMemoryTransport,
  type RestHttpConfig,
} from './rest-http';

// Sub-backend delegates
import {
  rebuildIndex as _rebuildIndex,
  getGuardrails as _getGuardrails,
  updateGuardrails as _updateGuardrails,
  aggregate as _aggregate,
  matchQuery as _matchQuery,
  removeEdge as _removeEdge,
  getEdgeCount as _getEdgeCount,
  listNodes as _listNodes,
  getNodeEdges as _getNodeEdges,
  getNodePayload as _getNodePayload,
  upsertNodePayload as _upsertNodePayload,
  graphSearch as _graphSearch,
} from './missing-endpoints';
import {
  storeSemanticFact as _storeSemanticFact,
  searchSemanticMemory as _searchSemanticMemory,
  recordEpisodicEvent as _recordEpisodicEvent,
  recallEpisodicEvents as _recallEpisodicEvents,
  storeProceduralPattern as _storeProceduralPattern,
  matchProceduralPatterns as _matchProceduralPatterns,
} from './agent-memory-backend';
import {
  search as _search, searchBatch as _searchBatch,
  textSearch as _textSearch, hybridSearch as _hybridSearch,
  multiQuerySearch as _multiQuerySearch, searchIds as _searchIds,
} from './search-backend';
import {
  addEdge as _addEdge, getEdges as _getEdges,
  traverseGraph as _traverseGraph, traverseParallel as _traverseParallel,
  getNodeDegree as _getNodeDegree, createGraphCollection as _createGraphCollection,
} from './graph-backend';
import { query as _query, queryExplain as _queryExplain, collectionSanity as _collectionSanity } from './query-backend';
import { scroll as _scroll } from './scroll-backend';
import { getCollectionStats as _getCollectionStats, analyzeCollection as _analyzeCollection, getCollectionConfig as _getCollectionConfig } from './admin-backend';
import { createIndex as _createIndex, listIndexes as _listIndexes, hasIndex as _hasIndex, dropIndex as _dropIndex } from './index-backend';
import { trainPq as _trainPq, streamInsert as _streamInsert, streamUpsertPoints as _streamUpsertPoints } from './streaming-backend';
import {
  createCollection as _createCollection, deleteCollection as _deleteCollection,
  getCollection as _getCollection, listCollections as _listCollections,
  upsert as _upsert, upsertBatch as _upsertBatch,
  deletePoint as _deletePoint, get as _get, isEmpty as _isEmpty, flush as _flush,
} from './crud-backend';

// Re-export for backward compatibility
export { generateUniqueId, _resetIdState } from './agent-memory-backend';
export type { QueryExplainApiResponse, CollectionSanityApiResponse } from './query-backend';

/**
 * REST Backend
 *
 * Provides vector storage via VelesDB REST API server.
 */
export class RestBackend implements IVelesDBBackend {
  private readonly httpConfig: RestHttpConfig;
  private _initialized = false;

  constructor(url: string, apiKey?: string, timeout = 30000) {
    this.httpConfig = { baseUrl: url.replace(/\/$/, ''), apiKey, timeout };
  }

  async init(): Promise<void> {
    if (this._initialized) { return; }
    try {
      const response = await request<{ status: string }>(this.httpConfig, 'GET', '/health');
      if (response.error) { throw new Error(response.error.message); }
      this._initialized = true;
    } catch (error) {
      throw new ConnectionError(
        `Failed to connect to VelesDB server at ${this.httpConfig.baseUrl}`,
        error instanceof Error ? error : undefined
      );
    }
  }

  isInitialized(): boolean { return this._initialized; }
  capabilities(): Readonly<CapabilityMap> { return REST_CAPABILITIES; }
  async close(): Promise<void> { this._initialized = false; }

  private ensureInitialized(): void {
    if (!this._initialized) { throw new ConnectionError('REST backend not initialized'); }
  }

  // Collection CRUD
  async createCollection(n: string, c: CollectionConfig): Promise<void> { this.ensureInitialized(); return _createCollection(buildCrudTransport(this.httpConfig), n, c); }
  async deleteCollection(n: string): Promise<void> { this.ensureInitialized(); return _deleteCollection(buildCrudTransport(this.httpConfig), n); }
  async getCollection(n: string): Promise<Collection | null> { this.ensureInitialized(); return _getCollection(buildCrudTransport(this.httpConfig), n); }
  async listCollections(): Promise<Collection[]> { this.ensureInitialized(); return _listCollections(buildCrudTransport(this.httpConfig)); }
  async upsert(c: string, d: VectorDocument): Promise<void> { this.ensureInitialized(); return _upsert(buildCrudTransport(this.httpConfig), c, d); }
  async upsertBatch(c: string, d: VectorDocument[]): Promise<void> { this.ensureInitialized(); return _upsertBatch(buildCrudTransport(this.httpConfig), c, d); }
  async delete(c: string, id: string | number): Promise<boolean> { this.ensureInitialized(); return _deletePoint(buildCrudTransport(this.httpConfig), c, id); }
  async get(c: string, id: string | number): Promise<VectorDocument | null> { this.ensureInitialized(); return _get(buildCrudTransport(this.httpConfig), c, id); }
  async isEmpty(c: string): Promise<boolean> { this.ensureInitialized(); return _isEmpty(buildCrudTransport(this.httpConfig), c); }
  async flush(c: string): Promise<void> { this.ensureInitialized(); return _flush(buildCrudTransport(this.httpConfig), c); }

  // Additional REST endpoints (Sprint 2 Wave 4)
  async rebuildIndex(c: string): Promise<RebuildIndexResponse> { this.ensureInitialized(); return _rebuildIndex(buildBaseTransport(this.httpConfig), c); }
  async getGuardrails(): Promise<GuardRailsConfigResponse> { this.ensureInitialized(); return _getGuardrails(buildBaseTransport(this.httpConfig)); }
  async updateGuardrails(r: GuardRailsUpdateRequest): Promise<GuardRailsConfigResponse> { this.ensureInitialized(); return _updateGuardrails(buildBaseTransport(this.httpConfig), r); }
  async aggregate(q: string, p?: Record<string, unknown>, o?: AggregateQueryOptions): Promise<AggregateResponse> { this.ensureInitialized(); return _aggregate(buildBaseTransport(this.httpConfig), q, p, o); }
  async matchQuery(c: string, q: string, p?: Record<string, unknown>, o?: MatchQueryOptions): Promise<MatchQueryResponse> { this.ensureInitialized(); return _matchQuery(buildBaseTransport(this.httpConfig), c, q, p, o); }
  async removeEdge(c: string, id: number): Promise<boolean> { this.ensureInitialized(); return _removeEdge(buildBaseTransport(this.httpConfig), c, id); }
  async getEdgeCount(c: string): Promise<number> { this.ensureInitialized(); return _getEdgeCount(buildBaseTransport(this.httpConfig), c); }
  async listNodes(c: string): Promise<ListNodesResponse> { this.ensureInitialized(); return _listNodes(buildBaseTransport(this.httpConfig), c); }
  async getNodeEdges(c: string, id: number, o?: GetNodeEdgesOptions): Promise<GraphEdge[]> { this.ensureInitialized(); return _getNodeEdges(buildBaseTransport(this.httpConfig), c, id, o); }
  async getNodePayload(c: string, id: number): Promise<NodePayloadResponse> { this.ensureInitialized(); return _getNodePayload(buildBaseTransport(this.httpConfig), c, id); }
  async upsertNodePayload(c: string, id: number, p: Record<string, unknown>): Promise<void> { this.ensureInitialized(); return _upsertNodePayload(buildBaseTransport(this.httpConfig), c, id, p); }
  async graphSearch(c: string, r: GraphSearchReq): Promise<GraphSearchResponse> { this.ensureInitialized(); return _graphSearch(buildBaseTransport(this.httpConfig), c, r); }

  // Search
  async search(c: string, q: number[] | Float32Array, o?: SearchOptions): Promise<SearchResult[]> { this.ensureInitialized(); return _search(buildSearchTransport(this.httpConfig), c, q, o); }
  async searchBatch(c: string, s: Array<{ vector: number[] | Float32Array; k?: number; filter?: FilterInput; quality?: SearchQuality }>): Promise<SearchResult[][]> { this.ensureInitialized(); return _searchBatch(buildSearchTransport(this.httpConfig), c, s); }
  async textSearch(c: string, q: string, o?: { k?: number; filter?: FilterInput }): Promise<SearchResult[]> { this.ensureInitialized(); return _textSearch(buildSearchTransport(this.httpConfig), c, q, o); }
  async hybridSearch(c: string, v: number[] | Float32Array, t: string, o?: { k?: number; vectorWeight?: number; filter?: FilterInput }): Promise<SearchResult[]> { this.ensureInitialized(); return _hybridSearch(buildSearchTransport(this.httpConfig), c, v, t, o); }
  async multiQuerySearch(c: string, v: Array<number[] | Float32Array>, o?: MultiQuerySearchOptions): Promise<SearchResult[]> { this.ensureInitialized(); return _multiQuerySearch(buildSearchTransport(this.httpConfig), c, v, o); }
  async searchIds(c: string, q: number[] | Float32Array, o?: SearchOptions): Promise<Array<{ id: number; score: number }>> { this.ensureInitialized(); return _searchIds(buildSearchTransport(this.httpConfig), c, q, o); }

  // Query
  async query(c: string, q: string, p?: Record<string, unknown>, o?: QueryOptions): Promise<QueryApiResponse> { this.ensureInitialized(); return _query(buildQueryTransport(this.httpConfig), c, q, p, o); }
  async queryExplain(q: string, p?: Record<string, unknown>, o?: { analyze?: boolean }): Promise<ExplainResponse> { this.ensureInitialized(); return _queryExplain(buildQueryTransport(this.httpConfig), q, p, o); }
  async collectionSanity(c: string): Promise<CollectionSanityResponse> { this.ensureInitialized(); return _collectionSanity(buildQueryTransport(this.httpConfig), c); }

  // Scroll
  async scroll(c: string, r?: ScrollRequest): Promise<ScrollResponse> { this.ensureInitialized(); return _scroll(buildCrudTransport(this.httpConfig), c, r); }

  // Graph
  async addEdge(c: string, e: AddEdgeRequest): Promise<void> { this.ensureInitialized(); return _addEdge(buildCrudTransport(this.httpConfig), c, e); }
  async getEdges(c: string, o?: GetEdgesOptions): Promise<GraphEdge[]> { this.ensureInitialized(); return _getEdges(buildCrudTransport(this.httpConfig), c, o); }
  async traverseGraph(c: string, r: TraverseRequest): Promise<TraverseResponse> { this.ensureInitialized(); return _traverseGraph(buildCrudTransport(this.httpConfig), c, r); }
  async traverseParallel(c: string, r: TraverseParallelRequest): Promise<TraverseResponse> { this.ensureInitialized(); return _traverseParallel(buildCrudTransport(this.httpConfig), c, r); }
  async getNodeDegree(c: string, id: number): Promise<DegreeResponse> { this.ensureInitialized(); return _getNodeDegree(buildCrudTransport(this.httpConfig), c, id); }
  async createGraphCollection(n: string, c?: GraphCollectionConfig): Promise<void> { this.ensureInitialized(); return _createGraphCollection(buildCrudTransport(this.httpConfig), n, c); }

  // Index
  async createIndex(c: string, o: CreateIndexOptions): Promise<void> { this.ensureInitialized(); return _createIndex(buildCrudTransport(this.httpConfig), c, o); }
  async listIndexes(c: string): Promise<IndexInfo[]> { this.ensureInitialized(); return _listIndexes(buildCrudTransport(this.httpConfig), c); }
  async hasIndex(c: string, l: string, p: string): Promise<boolean> { this.ensureInitialized(); return _hasIndex(buildCrudTransport(this.httpConfig), c, l, p); }
  async dropIndex(c: string, l: string, p: string): Promise<boolean> { this.ensureInitialized(); return _dropIndex(buildCrudTransport(this.httpConfig), c, l, p); }

  // Admin
  async getCollectionStats(c: string): Promise<CollectionStatsResponse | null> { this.ensureInitialized(); return _getCollectionStats(buildCrudTransport(this.httpConfig), c); }
  async analyzeCollection(c: string): Promise<CollectionStatsResponse> { this.ensureInitialized(); return _analyzeCollection(buildCrudTransport(this.httpConfig), c); }
  async getCollectionConfig(c: string): Promise<CollectionConfigResponse> { this.ensureInitialized(); return _getCollectionConfig(buildCrudTransport(this.httpConfig), c); }

  // Streaming / PQ
  async trainPq(c: string, o?: PqTrainOptions): Promise<string> { this.ensureInitialized(); return _trainPq(buildStreamingTransport(this.httpConfig), c, o); }
  async streamInsert(c: string, d: VectorDocument[]): Promise<void> { this.ensureInitialized(); return _streamInsert(buildStreamingTransport(this.httpConfig), c, d); }
  async streamUpsertPoints(c: string, d: VectorDocument[]): Promise<StreamUpsertResponse> { this.ensureInitialized(); return _streamUpsertPoints(buildStreamingTransport(this.httpConfig), c, d); }

  // Agent Memory
  async storeSemanticFact(c: string, e: SemanticEntry): Promise<void> { this.ensureInitialized(); return _storeSemanticFact(buildAgentMemoryTransport(this.httpConfig, (col, emb, opts) => this.search(col, emb, opts)), c, e); }
  async searchSemanticMemory(c: string, e: number[], k = 5): Promise<SearchResult[]> { this.ensureInitialized(); return _searchSemanticMemory(buildAgentMemoryTransport(this.httpConfig, (col, emb, opts) => this.search(col, emb, opts)), c, e, k); }
  async recordEpisodicEvent(c: string, e: EpisodicEvent): Promise<void> { this.ensureInitialized(); return _recordEpisodicEvent(buildAgentMemoryTransport(this.httpConfig, (col, emb, opts) => this.search(col, emb, opts)), c, e); }
  async recallEpisodicEvents(c: string, e: number[], k = 5): Promise<SearchResult[]> { this.ensureInitialized(); return _recallEpisodicEvents(buildAgentMemoryTransport(this.httpConfig, (col, emb, opts) => this.search(col, emb, opts)), c, e, k); }
  async storeProceduralPattern(c: string, p: ProceduralPattern): Promise<void> { this.ensureInitialized(); return _storeProceduralPattern(buildAgentMemoryTransport(this.httpConfig, (col, emb, opts) => this.search(col, emb, opts)), c, p); }
  async matchProceduralPatterns(c: string, e: number[], k = 5): Promise<SearchResult[]> { this.ensureInitialized(); return _matchProceduralPatterns(buildAgentMemoryTransport(this.httpConfig, (col, emb, opts) => this.search(col, emb, opts)), c, e, k); }
}
