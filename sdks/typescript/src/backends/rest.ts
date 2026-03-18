/**
 * REST Backend for VelesDB
 *
 * Connects to VelesDB server via REST API.
 * This is the composition root that delegates to focused backend modules.
 */

import type {
  IVelesDBBackend,
  CollectionConfig,
  Collection,
  VectorDocument,
  SearchOptions,
  SearchResult,
  MultiQuerySearchOptions,
  CreateIndexOptions,
  IndexInfo,
  AddEdgeRequest,
  GetEdgesOptions,
  GraphEdge,
  TraverseRequest,
  TraverseResponse,
  DegreeResponse,
  QueryOptions,
  QueryApiResponse,
  ExplainResponse,
  CollectionSanityResponse,
  PqTrainOptions,
  SparseVector,
  GraphCollectionConfig,
  CollectionStatsResponse,
  CollectionConfigResponse,
  SemanticEntry,
  EpisodicEvent,
  ProceduralPattern,
} from '../types';
import { ConnectionError } from '../types';
import {
  storeSemanticFact as _storeSemanticFact,
  searchSemanticMemory as _searchSemanticMemory,
  recordEpisodicEvent as _recordEpisodicEvent,
  recallEpisodicEvents as _recallEpisodicEvents,
  storeProceduralPattern as _storeProceduralPattern,
  matchProceduralPatterns as _matchProceduralPatterns,
} from './agent-memory-backend';
import type { AgentMemoryTransport } from './agent-memory-backend';
import {
  search as _search,
  searchBatch as _searchBatch,
  textSearch as _textSearch,
  hybridSearch as _hybridSearch,
  multiQuerySearch as _multiQuerySearch,
  searchIds as _searchIds,
} from './search-backend';
import type { SearchTransport } from './search-backend';
import {
  addEdge as _addEdge,
  getEdges as _getEdges,
  traverseGraph as _traverseGraph,
  getNodeDegree as _getNodeDegree,
  createGraphCollection as _createGraphCollection,
} from './graph-backend';
import {
  query as _query,
  queryExplain as _queryExplain,
  collectionSanity as _collectionSanity,
} from './query-backend';
import type { QueryTransport } from './query-backend';
import {
  getCollectionStats as _getCollectionStats,
  analyzeCollection as _analyzeCollection,
  getCollectionConfig as _getCollectionConfig,
} from './admin-backend';
import {
  createIndex as _createIndex,
  listIndexes as _listIndexes,
  hasIndex as _hasIndex,
  dropIndex as _dropIndex,
} from './index-backend';
import {
  trainPq as _trainPq,
  streamInsert as _streamInsert,
} from './streaming-backend';
import type { StreamingTransport } from './streaming-backend';
import {
  createCollection as _createCollection,
  deleteCollection as _deleteCollection,
  getCollection as _getCollection,
  listCollections as _listCollections,
  insert as _insert,
  insertBatch as _insertBatch,
  deletePoint as _deletePoint,
  get as _get,
  isEmpty as _isEmpty,
  flush as _flush,
  parseRestPointId,
  sparseVectorToRestFormat,
} from './crud-backend';
import type { CrudTransport } from './crud-backend';

import type { TransportResponse } from './shared';

// Re-export for backward compatibility
export { generateUniqueId, _resetIdState } from './agent-memory-backend';
export type { QueryExplainApiResponse, CollectionSanityApiResponse } from './query-backend';

/**
 * REST Backend
 *
 * Provides vector storage via VelesDB REST API server.
 */
export class RestBackend implements IVelesDBBackend {
  private readonly baseUrl: string;
  private readonly apiKey?: string;
  private readonly timeout: number;
  private _initialized = false;

  constructor(url: string, apiKey?: string, timeout = 30000) {
    this.baseUrl = url.replace(/\/$/, '');
    this.apiKey = apiKey;
    this.timeout = timeout;
  }

  async init(): Promise<void> {
    if (this._initialized) {
      return;
    }
    try {
      const response = await this.request<{ status: string }>('GET', '/health');
      if (response.error) {
        throw new Error(response.error.message);
      }
      this._initialized = true;
    } catch (error) {
      throw new ConnectionError(
        `Failed to connect to VelesDB server at ${this.baseUrl}`,
        error instanceof Error ? error : undefined
      );
    }
  }

  isInitialized(): boolean {
    return this._initialized;
  }

  private ensureInitialized(): void {
    if (!this._initialized) {
      throw new ConnectionError('REST backend not initialized');
    }
  }

  private mapStatusToErrorCode(status: number): string {
    switch (status) {
      case 400: return 'BAD_REQUEST';
      case 401: return 'UNAUTHORIZED';
      case 403: return 'FORBIDDEN';
      case 404: return 'NOT_FOUND';
      case 409: return 'CONFLICT';
      case 429: return 'RATE_LIMITED';
      case 500: return 'INTERNAL_ERROR';
      case 503: return 'SERVICE_UNAVAILABLE';
      default:  return 'UNKNOWN_ERROR';
    }
  }

  private extractErrorPayload(data: unknown): { code?: string; message?: string } {
    if (!data || typeof data !== 'object') {
      return {};
    }
    const payload = data as Record<string, unknown>;
    const nestedError =
      payload.error && typeof payload.error === 'object'
        ? (payload.error as Record<string, unknown>)
        : undefined;
    const codeField = nestedError?.code ?? payload.code;
    const code = typeof codeField === 'string' ? codeField : undefined;
    const messageField = nestedError?.message ?? payload.message ?? payload.error;
    const message = typeof messageField === 'string' ? messageField : undefined;
    return { code, message };
  }

  private parseNodeId(value: unknown): bigint | number {
    if (value === null || value === undefined) {
      return 0;
    }
    if (typeof value === 'bigint') {
      return value;
    }
    if (typeof value === 'string') {
      const num = Number(value);
      return num > Number.MAX_SAFE_INTEGER ? BigInt(value) : num;
    }
    if (typeof value === 'number') {
      return value;
    }
    return 0;
  }

  private async request<T>(method: string, path: string, body?: unknown): Promise<TransportResponse<T>> {
    const url = `${this.baseUrl}${path}`;
    const headers: Record<string, string> = { 'Content-Type': 'application/json' };
    if (this.apiKey) {
      headers['Authorization'] = `Bearer ${this.apiKey}`;
    }
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), this.timeout);

    try {
      const response = await fetch(url, {
        method,
        headers,
        body: body ? JSON.stringify(body) : undefined,
        signal: controller.signal,
      });
      clearTimeout(timeoutId);
      const data = await response.json().catch(() => ({}));
      if (!response.ok) {
        const errorPayload = this.extractErrorPayload(data);
        return {
          error: {
            code: errorPayload.code ?? this.mapStatusToErrorCode(response.status),
            message: errorPayload.message ?? `HTTP ${response.status}`,
          },
        };
      }
      return { data };
    } catch (error) {
      clearTimeout(timeoutId);
      if (error instanceof Error && error.name === 'AbortError') {
        throw new ConnectionError('Request timeout');
      }
      throw new ConnectionError(
        `Request failed: ${error instanceof Error ? error.message : 'Unknown error'}`,
        error instanceof Error ? error : undefined
      );
    }
  }

  // ==========================================================================
  // Transport adapters
  // ==========================================================================

  private asCrudTransport(): CrudTransport {
    return {
      requestJson: <T>(m: string, p: string, b?: unknown) => this.request<T>(m, p, b),
    };
  }

  private asSearchTransport(): SearchTransport {
    return {
      requestJson: <T>(m: string, p: string, b?: unknown) => this.request<T>(m, p, b),
      sparseToRest: (sv: SparseVector) => sparseVectorToRestFormat(sv),
    };
  }

  private asAgentMemoryTransport(): AgentMemoryTransport {
    return {
      requestJson: <T>(m: string, p: string, b?: unknown) => this.request<T>(m, p, b),
      searchVectors: (c: string, e: number[], k: number, f: Record<string, string>) =>
        this.search(c, e, { k, filter: f }),
    };
  }

  private asQueryTransport(): QueryTransport {
    return {
      requestJson: <T>(m: string, p: string, b?: unknown) => this.request<T>(m, p, b),
      parseNodeId: (v: unknown) => this.parseNodeId(v),
    };
  }

  private asStreamingTransport(): StreamingTransport {
    return {
      requestJson: <T>(m: string, p: string, b?: unknown) => this.request<T>(m, p, b),
      baseUrl: this.baseUrl,
      apiKey: this.apiKey,
      timeout: this.timeout,
      parseRestPointId,
      sparseVectorToRestFormat,
      mapStatusToErrorCode: (s: number) => this.mapStatusToErrorCode(s),
      extractErrorPayload: (d: unknown) => this.extractErrorPayload(d),
    };
  }

  // ==========================================================================
  // Collection CRUD — delegates to crud-backend.ts
  // ==========================================================================

  async createCollection(name: string, config: CollectionConfig): Promise<void> {
    this.ensureInitialized();
    return _createCollection(this.asCrudTransport(), name, config);
  }

  async deleteCollection(name: string): Promise<void> {
    this.ensureInitialized();
    return _deleteCollection(this.asCrudTransport(), name);
  }

  async getCollection(name: string): Promise<Collection | null> {
    this.ensureInitialized();
    return _getCollection(this.asCrudTransport(), name);
  }

  async listCollections(): Promise<Collection[]> {
    this.ensureInitialized();
    return _listCollections(this.asCrudTransport());
  }

  async insert(collection: string, doc: VectorDocument): Promise<void> {
    this.ensureInitialized();
    return _insert(this.asCrudTransport(), collection, doc);
  }

  async insertBatch(collection: string, docs: VectorDocument[]): Promise<void> {
    this.ensureInitialized();
    return _insertBatch(this.asCrudTransport(), collection, docs);
  }

  async delete(collection: string, id: string | number): Promise<boolean> {
    this.ensureInitialized();
    return _deletePoint(this.asCrudTransport(), collection, id);
  }

  async get(collection: string, id: string | number): Promise<VectorDocument | null> {
    this.ensureInitialized();
    return _get(this.asCrudTransport(), collection, id);
  }

  async isEmpty(collection: string): Promise<boolean> {
    this.ensureInitialized();
    return _isEmpty(this.asCrudTransport(), collection);
  }

  async flush(collection: string): Promise<void> {
    this.ensureInitialized();
    return _flush(this.asCrudTransport(), collection);
  }

  async close(): Promise<void> {
    this._initialized = false;
  }

  // ==========================================================================
  // Search — delegates to search-backend.ts
  // ==========================================================================

  async search(c: string, q: number[] | Float32Array, o?: SearchOptions): Promise<SearchResult[]> {
    this.ensureInitialized();
    return _search(this.asSearchTransport(), c, q, o);
  }

  async searchBatch(
    collection: string,
    searches: Array<{ vector: number[] | Float32Array; k?: number; filter?: Record<string, unknown> }>
  ): Promise<SearchResult[][]> {
    this.ensureInitialized();
    return _searchBatch(this.asSearchTransport(), collection, searches);
  }

  async textSearch(
    c: string, q: string, o?: { k?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    return _textSearch(this.asSearchTransport(), c, q, o);
  }

  async hybridSearch(
    c: string, v: number[] | Float32Array, t: string,
    o?: { k?: number; vectorWeight?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    return _hybridSearch(this.asSearchTransport(), c, v, t, o);
  }

  async multiQuerySearch(
    c: string, v: Array<number[] | Float32Array>, o?: MultiQuerySearchOptions
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    return _multiQuerySearch(this.asSearchTransport(), c, v, o);
  }

  async searchIds(
    c: string, q: number[] | Float32Array, o?: SearchOptions
  ): Promise<Array<{ id: number; score: number }>> {
    this.ensureInitialized();
    return _searchIds(this.asSearchTransport(), c, q, o);
  }

  // ==========================================================================
  // Query — delegates to query-backend.ts
  // ==========================================================================

  async query(
    c: string, q: string, p?: Record<string, unknown>, o?: QueryOptions
  ): Promise<QueryApiResponse> {
    this.ensureInitialized();
    return _query(this.asQueryTransport(), c, q, p, o);
  }

  async queryExplain(q: string, p?: Record<string, unknown>): Promise<ExplainResponse> {
    this.ensureInitialized();
    return _queryExplain(this.asQueryTransport(), q, p);
  }

  async collectionSanity(collection: string): Promise<CollectionSanityResponse> {
    this.ensureInitialized();
    return _collectionSanity(this.asQueryTransport(), collection);
  }

  // ==========================================================================
  // Graph — delegates to graph-backend.ts
  // ==========================================================================

  async addEdge(collection: string, edge: AddEdgeRequest): Promise<void> {
    this.ensureInitialized();
    return _addEdge(this.asCrudTransport(), collection, edge);
  }

  async getEdges(collection: string, options?: GetEdgesOptions): Promise<GraphEdge[]> {
    this.ensureInitialized();
    return _getEdges(this.asCrudTransport(), collection, options);
  }

  async traverseGraph(collection: string, req: TraverseRequest): Promise<TraverseResponse> {
    this.ensureInitialized();
    return _traverseGraph(this.asCrudTransport(), collection, req);
  }

  async getNodeDegree(collection: string, nodeId: number): Promise<DegreeResponse> {
    this.ensureInitialized();
    return _getNodeDegree(this.asCrudTransport(), collection, nodeId);
  }

  async createGraphCollection(name: string, config?: GraphCollectionConfig): Promise<void> {
    this.ensureInitialized();
    return _createGraphCollection(this.asCrudTransport(), name, config);
  }

  // ==========================================================================
  // Index — delegates to index-backend.ts
  // ==========================================================================

  async createIndex(collection: string, options: CreateIndexOptions): Promise<void> {
    this.ensureInitialized();
    return _createIndex(this.asCrudTransport(), collection, options);
  }

  async listIndexes(collection: string): Promise<IndexInfo[]> {
    this.ensureInitialized();
    return _listIndexes(this.asCrudTransport(), collection);
  }

  async hasIndex(collection: string, label: string, property: string): Promise<boolean> {
    this.ensureInitialized();
    return _hasIndex(this.asCrudTransport(), collection, label, property);
  }

  async dropIndex(collection: string, label: string, property: string): Promise<boolean> {
    this.ensureInitialized();
    return _dropIndex(this.asCrudTransport(), collection, label, property);
  }

  // ==========================================================================
  // Admin — delegates to admin-backend.ts
  // ==========================================================================

  async getCollectionStats(collection: string): Promise<CollectionStatsResponse | null> {
    this.ensureInitialized();
    return _getCollectionStats(this.asCrudTransport(), collection);
  }

  async analyzeCollection(collection: string): Promise<CollectionStatsResponse> {
    this.ensureInitialized();
    return _analyzeCollection(this.asCrudTransport(), collection);
  }

  async getCollectionConfig(collection: string): Promise<CollectionConfigResponse> {
    this.ensureInitialized();
    return _getCollectionConfig(this.asCrudTransport(), collection);
  }

  // ==========================================================================
  // Streaming / PQ — delegates to streaming-backend.ts
  // ==========================================================================

  async trainPq(collection: string, options?: PqTrainOptions): Promise<string> {
    this.ensureInitialized();
    return _trainPq(this.asStreamingTransport(), collection, options);
  }

  async streamInsert(collection: string, docs: VectorDocument[]): Promise<void> {
    this.ensureInitialized();
    return _streamInsert(this.asStreamingTransport(), collection, docs);
  }

  // ==========================================================================
  // Agent Memory — delegates to agent-memory-backend.ts
  // ==========================================================================

  async storeSemanticFact(collection: string, entry: SemanticEntry): Promise<void> {
    this.ensureInitialized();
    return _storeSemanticFact(this.asAgentMemoryTransport(), collection, entry);
  }

  async searchSemanticMemory(collection: string, embedding: number[], k = 5): Promise<SearchResult[]> {
    return _searchSemanticMemory(this.asAgentMemoryTransport(), collection, embedding, k);
  }

  async recordEpisodicEvent(collection: string, event: EpisodicEvent): Promise<void> {
    this.ensureInitialized();
    return _recordEpisodicEvent(this.asAgentMemoryTransport(), collection, event);
  }

  async recallEpisodicEvents(collection: string, embedding: number[], k = 5): Promise<SearchResult[]> {
    return _recallEpisodicEvents(this.asAgentMemoryTransport(), collection, embedding, k);
  }

  async storeProceduralPattern(collection: string, pattern: ProceduralPattern): Promise<void> {
    this.ensureInitialized();
    return _storeProceduralPattern(this.asAgentMemoryTransport(), collection, pattern);
  }

  async matchProceduralPatterns(collection: string, embedding: number[], k = 5): Promise<SearchResult[]> {
    return _matchProceduralPatterns(this.asAgentMemoryTransport(), collection, embedding, k);
  }
}
