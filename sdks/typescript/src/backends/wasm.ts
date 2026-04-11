/**
 * WASM Backend for VelesDB
 *
 * Uses velesdb-wasm for in-browser/Node.js vector operations.
 * Search/query logic lives in wasm-search.ts; unsupported-feature
 * stubs live in wasm-stubs.ts.
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
} from '../types';
import type { FilterInput } from '../filter';
import { ConnectionError, NotFoundError, VelesDBError } from '../types';
import type { SparseVector } from '../types';
import type { WasmModule, CollectionData, WasmContext } from './wasm-types';

// Search & query delegates
import {
  wasmSearch,
  wasmSearchBatch,
  wasmTextSearch,
  wasmHybridSearch,
  wasmMultiQuerySearch,
  wasmQuery,
} from './wasm-search';

// Unsupported-feature stubs
import {
  wasmCreateIndex,
  wasmListIndexes,
  wasmHasIndex,
  wasmDropIndex,
  wasmAddEdge,
  wasmGetEdges,
  wasmTraverseGraph,
  wasmTraverseParallel,
  wasmGetNodeDegree,
  wasmQueryExplain,
  wasmCollectionSanity,
  wasmScroll,
  wasmTrainPq,
  wasmStreamInsert,
  wasmCreateGraphCollection,
  wasmGetCollectionStats,
  wasmAnalyzeCollection,
  wasmGetCollectionConfig,
  wasmSearchIds,
  wasmStoreSemanticFact,
  wasmSearchSemanticMemory,
  wasmRecordEpisodicEvent,
  wasmRecallEpisodicEvents,
  wasmStoreProceduralPattern,
  wasmMatchProceduralPatterns,
} from './wasm-stubs';

/**
 * WASM Backend
 *
 * Provides vector storage using WebAssembly for optimal performance
 * in browser and Node.js environments.
 */
export class WasmBackend implements IVelesDBBackend {
  private wasmModule: WasmModule | null = null;
  private collections: Map<string, CollectionData> = new Map();
  private _initialized = false;

  // ========================================================================
  // Lifecycle
  // ========================================================================

  async init(): Promise<void> {
    if (this._initialized) {
      return;
    }

    try {
      this.wasmModule = await import('@wiscale/velesdb-wasm') as WasmModule;
      await this.wasmModule.default();
      this._initialized = true;
    } catch (error) {
      throw new ConnectionError(
        'Failed to initialize WASM module',
        error instanceof Error ? error : undefined
      );
    }
  }

  isInitialized(): boolean {
    return this._initialized;
  }

  async close(): Promise<void> {
    for (const [, data] of this.collections) {
      data.store.free();
    }
    this.collections.clear();
    this._initialized = false;
  }

  // ========================================================================
  // Internal helpers
  // ========================================================================

  private ensureInitialized(): void {
    if (!this._initialized || !this.wasmModule) {
      throw new ConnectionError('WASM backend not initialized');
    }
  }

  private normalizeIdString(id: string): string | null {
    const trimmed = id.trim();
    return /^\d+$/.test(trimmed) ? trimmed : null;
  }

  private canonicalPayloadKeyFromResultId(id: bigint | number | string): string {
    if (typeof id === 'bigint') {
      return id.toString();
    }
    if (typeof id === 'number') {
      return String(Math.trunc(id));
    }
    const normalized = this.normalizeIdString(id);
    if (normalized !== null) {
      return normalized.replace(/^0+(?=\d)/, '');
    }
    return String(this.toNumericId(id));
  }

  private canonicalPayloadKey(id: string | number): string {
    if (typeof id === 'number') {
      return String(Math.trunc(id));
    }
    const normalized = this.normalizeIdString(id);
    if (normalized !== null) {
      return normalized.replace(/^0+(?=\d)/, '');
    }
    return String(this.toNumericId(id));
  }

  private sparseVectorToArrays(sv: SparseVector): { indices: number[]; values: number[] } {
    const indices: number[] = [];
    const values: number[] = [];
    for (const [k, v] of Object.entries(sv)) {
      indices.push(Number(k));
      values.push(v);
    }
    return { indices, values };
  }

  private toNumericId(id: string | number): number {
    if (typeof id === 'number') {
      return id;
    }
    const normalized = this.normalizeIdString(id);
    if (normalized !== null) {
      const parsed = Number(normalized);
      if (Number.isSafeInteger(parsed)) {
        return parsed;
      }
    }
    let hash = 0;
    for (let i = 0; i < id.length; i++) {
      const char = id.charCodeAt(i);
      hash = ((hash << 5) - hash) + char;
      hash = hash & hash;
    }
    return Math.abs(hash);
  }

  /** Build a WasmContext for the extracted search/query modules. */
  private context(): WasmContext {
    return {
      wasmModule: this.wasmModule!,
      getCollection: (name: string) => this.collections.get(name),
      canonicalPayloadKeyFromResultId: (id) => this.canonicalPayloadKeyFromResultId(id),
      canonicalPayloadKey: (id) => this.canonicalPayloadKey(id),
      sparseVectorToArrays: (sv) => this.sparseVectorToArrays(sv),
      toNumericId: (id) => this.toNumericId(id),
    };
  }

  // ========================================================================
  // Collection management
  // ========================================================================

  async createCollection(name: string, config: CollectionConfig): Promise<void> {
    this.ensureInitialized();

    if (this.collections.has(name)) {
      throw new VelesDBError(`Collection '${name}' already exists`, 'COLLECTION_EXISTS');
    }

    const metric = config.metric ?? 'cosine';
    const store = new this.wasmModule!.VectorStore(config.dimension, metric);

    this.collections.set(name, {
      config: { ...config, metric },
      store,
      payloads: new Map(),
      createdAt: new Date(),
    });
  }

  async deleteCollection(name: string): Promise<void> {
    this.ensureInitialized();

    const collection = this.collections.get(name);
    if (!collection) {
      throw new NotFoundError(`Collection '${name}'`);
    }

    collection.store.free();
    this.collections.delete(name);
  }

  async getCollection(name: string): Promise<Collection | null> {
    this.ensureInitialized();

    const collection = this.collections.get(name);
    if (!collection) {
      return null;
    }

    return {
      name,
      dimension: collection.config.dimension ?? 0,
      metric: collection.config.metric ?? 'cosine',
      count: collection.store.len,
      createdAt: collection.createdAt,
    };
  }

  async listCollections(): Promise<Collection[]> {
    this.ensureInitialized();

    const result: Collection[] = [];
    for (const [name, data] of this.collections) {
      result.push({
        name,
        dimension: data.config.dimension ?? 0,
        metric: data.config.metric ?? 'cosine',
        count: data.store.len,
        createdAt: data.createdAt,
      });
    }
    return result;
  }

  // ========================================================================
  // Point CRUD
  // ========================================================================

  async insert(collectionName: string, doc: VectorDocument): Promise<void> {
    this.ensureInitialized();

    const collection = this.collections.get(collectionName);
    if (!collection) {
      throw new NotFoundError(`Collection '${collectionName}'`);
    }

    const id = this.toNumericId(doc.id);
    const vector = doc.vector instanceof Float32Array
      ? doc.vector
      : new Float32Array(doc.vector);

    if (vector.length !== collection.config.dimension) {
      throw new VelesDBError(
        `Vector dimension mismatch: expected ${collection.config.dimension}, got ${vector.length}`,
        'DIMENSION_MISMATCH'
      );
    }

    if (doc.payload) {
      collection.store.insert_with_payload(BigInt(id), vector, doc.payload);
    } else {
      collection.store.insert(BigInt(id), vector);
    }

    if (doc.payload) {
      collection.payloads.set(this.canonicalPayloadKey(doc.id), doc.payload);
    }
  }

  async insertBatch(collectionName: string, docs: VectorDocument[]): Promise<void> {
    this.ensureInitialized();

    const collection = this.collections.get(collectionName);
    if (!collection) {
      throw new NotFoundError(`Collection '${collectionName}'`);
    }

    for (const doc of docs) {
      if (doc.vector.length !== collection.config.dimension) {
        throw new VelesDBError(
          `Vector dimension mismatch for doc ${doc.id}: expected ${collection.config.dimension}, got ${doc.vector.length}`,
          'DIMENSION_MISMATCH'
        );
      }
    }

    collection.store.reserve(docs.length);

    const batch: Array<[bigint, number[]]> = [];
    for (const doc of docs) {
      const id = BigInt(this.toNumericId(doc.id));
      const vector = doc.vector instanceof Float32Array
        ? doc.vector
        : new Float32Array(doc.vector);

      if (doc.payload) {
        collection.store.insert_with_payload(id, vector, doc.payload);
      } else {
        batch.push([id, Array.from(vector)]);
      }
    }

    if (batch.length > 0) {
      collection.store.insert_batch(batch);
    }

    for (const doc of docs) {
      if (doc.payload) {
        collection.payloads.set(this.canonicalPayloadKey(doc.id), doc.payload);
      }
    }
  }

  async delete(collectionName: string, id: string | number): Promise<boolean> {
    this.ensureInitialized();

    const collection = this.collections.get(collectionName);
    if (!collection) {
      throw new NotFoundError(`Collection '${collectionName}'`);
    }

    const numericId = this.toNumericId(id);
    const removed = collection.store.remove(BigInt(numericId));

    if (removed) {
      collection.payloads.delete(this.canonicalPayloadKey(id));
    }

    return removed;
  }

  async get(collectionName: string, id: string | number): Promise<VectorDocument | null> {
    this.ensureInitialized();

    const collection = this.collections.get(collectionName);
    if (!collection) {
      throw new NotFoundError(`Collection '${collectionName}'`);
    }

    const numericId = this.toNumericId(id);
    const point = collection.store.get(BigInt(numericId)) as
      | { id: bigint | number; vector: number[] | Float32Array; payload?: Record<string, unknown> | null }
      | null;
    if (!point) {
      return null;
    }

    const payload =
      point.payload ??
      collection.payloads.get(this.canonicalPayloadKey(numericId));

    return {
      id: String(point.id),
      vector: Array.isArray(point.vector) ? point.vector : Array.from(point.vector),
      payload,
    };
  }

  // ========================================================================
  // Collection utilities
  // ========================================================================

  async isEmpty(collectionName: string): Promise<boolean> {
    this.ensureInitialized();

    const collection = this.collections.get(collectionName);
    if (!collection) {
      throw new NotFoundError(`Collection '${collectionName}'`);
    }

    return collection.store.is_empty();
  }

  async flush(collectionName: string): Promise<void> {
    this.ensureInitialized();

    const collection = this.collections.get(collectionName);
    if (!collection) {
      throw new NotFoundError(`Collection '${collectionName}'`);
    }

    // WASM runs in-memory, flush is a no-op
  }

  // ========================================================================
  // Search & Query — delegates to wasm-search.ts
  // ========================================================================

  async search(
    collectionName: string, query: number[] | Float32Array, options?: SearchOptions
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    return wasmSearch(this.context(), collectionName, query, options);
  }

  async searchBatch(
    collectionName: string,
    searches: Array<{ vector: number[] | Float32Array; k?: number; filter?: FilterInput }>
  ): Promise<SearchResult[][]> {
    this.ensureInitialized();
    return wasmSearchBatch(this.context(), collectionName, searches);
  }

  async textSearch(
    collection: string, query: string, options?: { k?: number; filter?: FilterInput }
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    return wasmTextSearch(this.context(), collection, query, options);
  }

  async hybridSearch(
    collection: string, vector: number[] | Float32Array, textQuery: string,
    options?: { k?: number; vectorWeight?: number; filter?: FilterInput }
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    return wasmHybridSearch(this.context(), collection, vector, textQuery, options);
  }

  async query(
    collection: string, queryString: string,
    params?: Record<string, unknown>, options?: QueryOptions
  ): Promise<QueryApiResponse> {
    this.ensureInitialized();
    return wasmQuery(this.context(), collection, queryString, params, options);
  }

  async multiQuerySearch(
    collection: string, vectors: Array<number[] | Float32Array>,
    options?: MultiQuerySearchOptions
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    return wasmMultiQuerySearch(this.context(), collection, vectors, options);
  }

  // ========================================================================
  // Stubs — delegates to wasm-stubs.ts
  // ========================================================================

  async queryExplain(q: string, p?: Record<string, unknown>, o?: { analyze?: boolean }): Promise<ExplainResponse> {
    this.ensureInitialized();
    return wasmQueryExplain(q, p, o);
  }

  async collectionSanity(collection: string): Promise<CollectionSanityResponse> {
    this.ensureInitialized();
    return wasmCollectionSanity(collection);
  }

  async scroll(collection: string, request?: ScrollRequest): Promise<ScrollResponse> {
    this.ensureInitialized();
    return wasmScroll(collection, request);
  }

  async createIndex(collection: string, options: CreateIndexOptions): Promise<void> {
    this.ensureInitialized();
    return wasmCreateIndex(collection, options);
  }

  async listIndexes(collection: string): Promise<IndexInfo[]> {
    this.ensureInitialized();
    return wasmListIndexes(collection);
  }

  async hasIndex(collection: string, label: string, property: string): Promise<boolean> {
    this.ensureInitialized();
    return wasmHasIndex(collection, label, property);
  }

  async dropIndex(collection: string, label: string, property: string): Promise<boolean> {
    this.ensureInitialized();
    return wasmDropIndex(collection, label, property);
  }

  async addEdge(collection: string, edge: AddEdgeRequest): Promise<void> {
    this.ensureInitialized();
    return wasmAddEdge(collection, edge);
  }

  async getEdges(collection: string, options?: GetEdgesOptions): Promise<GraphEdge[]> {
    this.ensureInitialized();
    return wasmGetEdges(collection, options);
  }

  async traverseGraph(collection: string, request: TraverseRequest): Promise<TraverseResponse> {
    this.ensureInitialized();
    return wasmTraverseGraph(collection, request);
  }

  async traverseParallel(collection: string, request: TraverseParallelRequest): Promise<TraverseResponse> {
    this.ensureInitialized();
    return wasmTraverseParallel(collection, request);
  }

  async getNodeDegree(collection: string, nodeId: number): Promise<DegreeResponse> {
    this.ensureInitialized();
    return wasmGetNodeDegree(collection, nodeId);
  }

  async trainPq(collection: string, options?: PqTrainOptions): Promise<string> {
    this.ensureInitialized();
    return wasmTrainPq(collection, options);
  }

  async streamInsert(collection: string, docs: VectorDocument[]): Promise<void> {
    this.ensureInitialized();
    return wasmStreamInsert(collection, docs);
  }

  async createGraphCollection(name: string, config?: GraphCollectionConfig): Promise<void> {
    this.ensureInitialized();
    return wasmCreateGraphCollection(name, config);
  }

  async getCollectionStats(collection: string): Promise<CollectionStatsResponse | null> {
    this.ensureInitialized();
    return wasmGetCollectionStats(collection);
  }

  async analyzeCollection(collection: string): Promise<CollectionStatsResponse> {
    this.ensureInitialized();
    return wasmAnalyzeCollection(collection);
  }

  async getCollectionConfig(collection: string): Promise<CollectionConfigResponse> {
    this.ensureInitialized();
    return wasmGetCollectionConfig(collection);
  }

  async searchIds(
    collection: string, query: number[] | Float32Array, options?: SearchOptions
  ): Promise<Array<{ id: number; score: number }>> {
    this.ensureInitialized();
    return wasmSearchIds(collection, query, options);
  }

  async storeSemanticFact(collection: string, entry: SemanticEntry): Promise<void> {
    this.ensureInitialized();
    return wasmStoreSemanticFact(collection, entry);
  }

  async searchSemanticMemory(
    collection: string, embedding: number[], k?: number
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    return wasmSearchSemanticMemory(collection, embedding, k);
  }

  async recordEpisodicEvent(collection: string, event: EpisodicEvent): Promise<void> {
    this.ensureInitialized();
    return wasmRecordEpisodicEvent(collection, event);
  }

  async recallEpisodicEvents(
    collection: string, embedding: number[], k?: number
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    return wasmRecallEpisodicEvents(collection, embedding, k);
  }

  async storeProceduralPattern(collection: string, pattern: ProceduralPattern): Promise<void> {
    this.ensureInitialized();
    return wasmStoreProceduralPattern(collection, pattern);
  }

  async matchProceduralPatterns(
    collection: string, embedding: number[], k?: number
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    return wasmMatchProceduralPatterns(collection, embedding, k);
  }
}
