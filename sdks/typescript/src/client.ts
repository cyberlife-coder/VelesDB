/**
 * VelesDB Client - Unified interface for all backends
 */

import type {
  VelesDBConfig,
  CollectionConfig,
  Collection,
  VectorDocument,
  SearchOptions,
  SearchQuality,
  SearchResult,
  IVelesDBBackend,
  MultiQuerySearchOptions,
  SparseSearchNamedOptions,
  SparseVector,
  CreateIndexOptions,
  AlterCollectionOptions,
  IndexInfo,
  AddEdgeRequest,
  GetEdgesOptions,
  GraphEdge,
  GraphNodeId,
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
  AgentMemoryConfig,
  ScrollRequest,
  ScrollResponse,
  RebuildIndexResponse,
  GuardRailsUpdateRequest,
  GuardRailsConfigResponse,
  ListNodesResponse,
  GetNodeEdgesOptions,
  NodePayloadResponse,
  GraphSearchRequest,
  GraphSearchResponse,
  AggregateQueryOptions,
  AggregateResponse,
  MatchQueryOptions,
  MatchQueryResponse,
  StreamUpsertResponse,
  StreamingConfig,
  RelateRequest,
  RelateResponse,
  RelationsResponse,
} from './types';
import type { FilterInput } from './filter';
import type { CapabilityMap } from './capabilities';
import { ValidationError } from './types';
import { WasmBackend } from './backends/wasm';
import { RestBackend } from './backends/rest';
import { AgentMemoryClient } from './agent-memory';
import {
  requireNonEmptyString,
  validateDocsBatch,
  validateDocument,
  validateRestPointId,
} from './client/validation';
import * as searchMethods from './client/search-methods';
import * as graphMethods from './client/graph-methods';

// Re-export for backward compatibility
export { AgentMemoryClient } from './agent-memory';

/**
 * VelesDB Client
 *
 * Provides a unified interface for interacting with VelesDB
 * using either WASM (browser/Node.js) or REST API backends.
 */
export class VelesDB {
  private readonly config: VelesDBConfig;
  private backend: IVelesDBBackend;
  private initialized = false;

  constructor(config: VelesDBConfig) {
    this.validateConfig(config);
    this.config = config;
    this.backend = this.createBackend(config);
  }

  private validateConfig(config: VelesDBConfig): void {
    // Runtime guard: callers from plain JS can bypass the compile-time type,
    // so validate `backend` as an untyped value rather than the narrowed union.
    const backend: string | undefined = config.backend;
    if (!backend) {
      throw new ValidationError('Backend type is required');
    }
    if (backend !== 'wasm' && backend !== 'rest') {
      throw new ValidationError(`Invalid backend type: ${backend}. Use 'wasm' or 'rest'`);
    }
    if (backend === 'rest' && !config.url) {
      throw new ValidationError('URL is required for REST backend');
    }
  }

  private createBackend(config: VelesDBConfig): IVelesDBBackend {
    switch (config.backend) {
      case 'wasm':
        return new WasmBackend();
      case 'rest': {
        if (!config.url) {
          throw new ValidationError('URL is required for REST backend');
        }
        return new RestBackend(config.url, config.apiKey, config.timeout);
      }
      default:
        throw new ValidationError(`Unknown backend: ${String(config.backend)}`);
    }
  }

  /** Initialize the client. Must be called before any other operations. */
  async init(): Promise<void> {
    if (this.initialized) { return; }
    await this.backend.init();
    this.initialized = true;
  }

  /** Check if client is initialized. */
  isInitialized(): boolean { return this.initialized; }

  private ensureInitialized(): void {
    if (!this.initialized) {
      throw new ValidationError('Client not initialized. Call init() first.');
    }
  }

  // ========================================================================
  // Collection CRUD
  // ========================================================================

  async createCollection(name: string, config: CollectionConfig): Promise<void> {
    this.ensureInitialized();
    requireNonEmptyString(name, 'Collection name');
    const isMetadataOnly = config.collectionType === 'metadata_only';
    if (!isMetadataOnly && (!config.dimension || config.dimension <= 0)) {
      throw new ValidationError('Dimension must be a positive integer for vector collections');
    }
    await this.backend.createCollection(name, config);
  }

  async createMetadataCollection(name: string): Promise<void> {
    this.ensureInitialized();
    requireNonEmptyString(name, 'Collection name');
    await this.backend.createCollection(name, { collectionType: 'metadata_only' });
  }

  async deleteCollection(name: string): Promise<void> {
    this.ensureInitialized();
    await this.backend.deleteCollection(name);
  }

  async getCollection(name: string): Promise<Collection | null> {
    this.ensureInitialized();
    return this.backend.getCollection(name);
  }

  async listCollections(): Promise<Collection[]> {
    this.ensureInitialized();
    return this.backend.listCollections();
  }

  // ========================================================================
  // Point CRUD
  // ========================================================================

  async upsert(collection: string, doc: VectorDocument): Promise<void> {
    this.ensureInitialized();
    validateDocument(doc, this.config);
    await this.backend.upsert(collection, doc);
  }

  async upsertBatch(collection: string, docs: VectorDocument[]): Promise<void> {
    this.ensureInitialized();
    validateDocsBatch(docs, doc => { validateDocument(doc, this.config); });
    await this.backend.upsertBatch(collection, docs);
  }

  /**
   * Bulk upsert via the binary wire format (REST backend only).
   *
   * Encodes `(id, vector)` pairs into the deterministic VRB1 binary layout
   * and sends them as a single `application/octet-stream` request, avoiding
   * per-point JSON overhead. Payloads are not carried — use
   * {@link upsertBatch} when you need them. Throws a not-supported error on
   * the WASM backend.
   *
   * @returns the number of points the server reports as inserted.
   */
  async upsertBatchRaw(collection: string, docs: VectorDocument[]): Promise<number> {
    this.ensureInitialized();
    validateDocsBatch(docs, doc => { validateDocument(doc, this.config); });
    return this.backend.upsertBatchRaw(collection, docs);
  }

  async delete(collection: string, id: string | number): Promise<boolean> {
    this.ensureInitialized();
    validateRestPointId(id, this.config);
    return this.backend.delete(collection, id);
  }

  async bulkDelete(collection: string, ids: Array<string | number>): Promise<number> {
    this.ensureInitialized();
    for (const id of ids) { validateRestPointId(id, this.config); }
    return this.backend.bulkDelete(collection, ids);
  }

  async get(collection: string, id: string | number): Promise<VectorDocument | null> {
    this.ensureInitialized();
    validateRestPointId(id, this.config);
    return this.backend.get(collection, id);
  }

  async isEmpty(collection: string): Promise<boolean> {
    this.ensureInitialized();
    return this.backend.isEmpty(collection);
  }

  async flush(collection: string): Promise<void> {
    this.ensureInitialized();
    await this.backend.flush(collection);
  }

  async close(): Promise<void> {
    if (this.initialized) {
      await this.backend.close();
      this.initialized = false;
    }
  }

  // ========================================================================
  // Search & Query -- delegates to client/search-methods.ts
  // ========================================================================

  async search(collection: string, query: number[] | Float32Array, options?: SearchOptions): Promise<SearchResult[]> {
    this.ensureInitialized();
    return searchMethods.search(this.backend, collection, query, options);
  }

  async searchBatch(collection: string, searches: Array<{ vector: number[] | Float32Array; k?: number; filter?: FilterInput; quality?: SearchQuality }>): Promise<SearchResult[][]> {
    this.ensureInitialized();
    return searchMethods.searchBatch(this.backend, collection, searches);
  }

  async textSearch(collection: string, query: string, options?: { k?: number; filter?: FilterInput }): Promise<SearchResult[]> {
    this.ensureInitialized();
    return searchMethods.textSearch(this.backend, collection, query, options);
  }

  async hybridSearch(collection: string, vector: number[] | Float32Array, textQuery: string, options?: { k?: number; vectorWeight?: number; filter?: FilterInput }): Promise<SearchResult[]> {
    this.ensureInitialized();
    return searchMethods.hybridSearch(this.backend, collection, vector, textQuery, options);
  }

  async multiQuerySearch(collection: string, vectors: Array<number[] | Float32Array>, options?: MultiQuerySearchOptions): Promise<SearchResult[]> {
    this.ensureInitialized();
    return searchMethods.multiQuerySearch(this.backend, collection, vectors, options);
  }

  /** Multi-query fusion search returning only IDs and scores (no payloads). */
  async multiQuerySearchIds(collection: string, vectors: Array<number[] | Float32Array>, options?: MultiQuerySearchOptions): Promise<Array<{ id: number; score: number }>> {
    this.ensureInitialized();
    return searchMethods.multiQuerySearchIds(this.backend, collection, vectors, options);
  }

  /**
   * Pure sparse search against a named sparse index.
   *
   * @see {@link SparseSearchNamedOptions} for the full pure-sparse vs hybrid comparison.
   * @see {@link VelesDB.search} for dense + sparse hybrid against a named index.
   */
  async sparseSearchNamed(collection: string, query: SparseVector, indexName: string, options?: SparseSearchNamedOptions): Promise<SearchResult[]> {
    this.ensureInitialized();
    return searchMethods.sparseSearchNamed(this.backend, collection, query, indexName, options);
  }

  async query(collection: string, queryString: string, params?: Record<string, unknown>, options?: QueryOptions): Promise<QueryApiResponse> {
    this.ensureInitialized();
    return searchMethods.query(this.backend, collection, queryString, params, options);
  }

  async queryExplain(queryString: string, params?: Record<string, unknown>, options?: { analyze?: boolean }): Promise<ExplainResponse> {
    this.ensureInitialized();
    return searchMethods.queryExplain(this.backend, queryString, params, options);
  }

  async collectionSanity(collection: string): Promise<CollectionSanityResponse> {
    this.ensureInitialized();
    return searchMethods.collectionSanity(this.backend, collection);
  }

  async scroll(collection: string, request?: ScrollRequest): Promise<ScrollResponse> {
    this.ensureInitialized();
    return searchMethods.scroll(this.backend, collection, request);
  }

  async trainPq(collection: string, options?: PqTrainOptions): Promise<string> {
    this.ensureInitialized();
    return searchMethods.trainPq(this.backend, collection, options);
  }

  async enableStreaming(collection: string, config?: StreamingConfig): Promise<void> {
    this.ensureInitialized();
    return searchMethods.enableStreaming(this.backend, collection, config);
  }

  async streamInsert(collection: string, docs: VectorDocument[]): Promise<void> {
    this.ensureInitialized();
    return searchMethods.streamInsert(this.backend, this.config, collection, docs);
  }

  async streamUpsertPoints(collection: string, docs: VectorDocument[]): Promise<StreamUpsertResponse> {
    this.ensureInitialized();
    return searchMethods.streamUpsertPoints(this.backend, this.config, collection, docs);
  }

  async searchIds(collection: string, query: number[] | Float32Array, options?: SearchOptions): Promise<Array<{ id: number; score: number }>> {
    this.ensureInitialized();
    return searchMethods.searchIds(this.backend, collection, query, options);
  }

  // ========================================================================
  // Admin / Stats -- delegates to client/search-methods.ts
  // ========================================================================

  async rebuildIndex(collection: string): Promise<RebuildIndexResponse> {
    this.ensureInitialized();
    return searchMethods.rebuildIndex(this.backend, collection);
  }

  async getGuardrails(): Promise<GuardRailsConfigResponse> {
    this.ensureInitialized();
    return searchMethods.getGuardrails(this.backend);
  }

  async updateGuardrails(req: GuardRailsUpdateRequest): Promise<GuardRailsConfigResponse> {
    this.ensureInitialized();
    return searchMethods.updateGuardrails(this.backend, req);
  }

  async aggregate(queryString: string, params?: Record<string, unknown>, options?: AggregateQueryOptions): Promise<AggregateResponse> {
    this.ensureInitialized();
    return searchMethods.aggregate(this.backend, queryString, params, options);
  }

  async getCollectionStats(collection: string): Promise<CollectionStatsResponse | null> {
    this.ensureInitialized();
    return searchMethods.getCollectionStats(this.backend, collection);
  }

  async analyzeCollection(collection: string): Promise<CollectionStatsResponse> {
    this.ensureInitialized();
    return searchMethods.analyzeCollection(this.backend, collection);
  }

  async getCollectionConfig(collection: string): Promise<CollectionConfigResponse> {
    this.ensureInitialized();
    return searchMethods.getCollectionConfig(this.backend, collection);
  }

  // ========================================================================
  // Index Management (EPIC-009)
  // ========================================================================

  async createIndex(collection: string, options: CreateIndexOptions): Promise<void> {
    this.ensureInitialized();
    if (!options.label || !options.property) {
      throw new ValidationError('Index requires label and property');
    }
    await this.backend.createIndex(collection, options);
  }

  async listIndexes(collection: string): Promise<IndexInfo[]> {
    this.ensureInitialized();
    return this.backend.listIndexes(collection);
  }

  async hasIndex(collection: string, label: string, property: string): Promise<boolean> {
    this.ensureInitialized();
    return this.backend.hasIndex(collection, label, property);
  }

  async dropIndex(collection: string, label: string, property: string): Promise<boolean> {
    this.ensureInitialized();
    return this.backend.dropIndex(collection, label, property);
  }

  // ========================================================================
  // Collection settings (ALTER COLLECTION)
  // ========================================================================

  /**
   * Toggle a collection's mutable settings at runtime via
   * `ALTER COLLECTION <name> SET(...)`.
   *
   * Typed wrapper over the raw VelesQL DDL; routes through the same
   * `/query` path as `db.query()`.
   *
   * @example
   * ```typescript
   * await db.alterCollection('docs', { autoReindex: true });
   * ```
   */
  async alterCollection(collection: string, options: AlterCollectionOptions): Promise<void> {
    this.ensureInitialized();
    const sets: string[] = [];
    if (options.autoReindex !== undefined) {
      sets.push(`auto_reindex=${options.autoReindex}`);
    }
    if (sets.length === 0) {
      throw new ValidationError('alterCollection requires at least one option');
    }
    const sql = `ALTER COLLECTION ${collection} SET(${sets.join(', ')})`;
    await searchMethods.query(this.backend, collection, sql);
  }

  /**
   * Enable or disable automatic index rebuilds on a collection.
   *
   * Convenience wrapper over {@link alterCollection}; emits
   * `ALTER COLLECTION <name> SET(auto_reindex=<enabled>)`.
   */
  async setAutoReindex(collection: string, enabled: boolean): Promise<void> {
    return this.alterCollection(collection, { autoReindex: enabled });
  }

  // ========================================================================
  // Knowledge Graph -- delegates to client/graph-methods.ts
  // ========================================================================

  async addEdge(collection: string, edge: AddEdgeRequest): Promise<void> {
    this.ensureInitialized();
    return graphMethods.addEdge(this.backend, collection, edge);
  }

  async getEdges(collection: string, options?: GetEdgesOptions): Promise<GraphEdge[]> {
    this.ensureInitialized();
    return graphMethods.getEdges(this.backend, collection, options);
  }

  async traverseGraph(collection: string, request: TraverseRequest): Promise<TraverseResponse> {
    this.ensureInitialized();
    return graphMethods.traverseGraph(this.backend, collection, request);
  }

  async traverseParallel(collection: string, request: TraverseParallelRequest): Promise<TraverseResponse> {
    this.ensureInitialized();
    return graphMethods.traverseParallel(this.backend, collection, request);
  }

  async getNodeDegree(collection: string, nodeId: number): Promise<DegreeResponse> {
    this.ensureInitialized();
    return graphMethods.getNodeDegree(this.backend, collection, nodeId);
  }

  async createGraphCollection(name: string, config?: GraphCollectionConfig): Promise<void> {
    this.ensureInitialized();
    return graphMethods.createGraphCollection(this.backend, name, config);
  }

  async matchQuery(collection: string, queryString: string, params?: Record<string, unknown>, options?: MatchQueryOptions): Promise<MatchQueryResponse> {
    this.ensureInitialized();
    return graphMethods.matchQuery(this.backend, collection, queryString, params, options);
  }

  async removeEdge(collection: string, edgeId: number): Promise<boolean> {
    this.ensureInitialized();
    return graphMethods.removeEdge(this.backend, collection, edgeId);
  }

  async getEdgeCount(collection: string): Promise<number> {
    this.ensureInitialized();
    return graphMethods.getEdgeCount(this.backend, collection);
  }

  async listNodes(collection: string): Promise<ListNodesResponse> {
    this.ensureInitialized();
    return graphMethods.listNodes(this.backend, collection);
  }

  async getNodeEdges(collection: string, nodeId: number, options?: GetNodeEdgesOptions): Promise<GraphEdge[]> {
    this.ensureInitialized();
    return graphMethods.getNodeEdges(this.backend, collection, nodeId, options);
  }

  async getNodePayload(collection: string, nodeId: number): Promise<NodePayloadResponse> {
    this.ensureInitialized();
    return graphMethods.getNodePayload(this.backend, collection, nodeId);
  }

  async upsertNodePayload(collection: string, nodeId: number, payload: Record<string, unknown>): Promise<void> {
    this.ensureInitialized();
    return graphMethods.upsertNodePayload(this.backend, collection, nodeId, payload);
  }

  async graphSearch(collection: string, request: GraphSearchRequest): Promise<GraphSearchResponse> {
    this.ensureInitialized();
    return graphMethods.graphSearch(this.backend, collection, request);
  }

  async relate(collection: string, req: RelateRequest): Promise<RelateResponse> {
    this.ensureInitialized();
    return graphMethods.relate(this.backend, collection, req);
  }

  async unrelate(collection: string, edgeId: GraphNodeId): Promise<boolean> {
    this.ensureInitialized();
    return graphMethods.unrelate(this.backend, collection, edgeId);
  }

  async getRelations(collection: string, pointId: GraphNodeId): Promise<RelationsResponse> {
    this.ensureInitialized();
    return graphMethods.getRelations(this.backend, collection, pointId);
  }

  async setTtlDurable(collection: string, pointId: GraphNodeId, ttlSeconds: number): Promise<void> {
    this.ensureInitialized();
    return graphMethods.setTtlDurable(this.backend, collection, pointId, ttlSeconds);
  }

  // ========================================================================
  // Capabilities & Backend Info
  // ========================================================================

  capabilities(): Readonly<CapabilityMap> { return this.backend.capabilities(); }

  get backendType(): string { return this.config.backend; }

  // ========================================================================
  // Agent Memory (Phase 8)
  // ========================================================================

  agentMemory(config?: AgentMemoryConfig): AgentMemoryClient {
    this.ensureInitialized();
    return new AgentMemoryClient(this.backend, config);
  }
}
