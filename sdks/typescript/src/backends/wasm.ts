/**
 * WASM Backend for VelesDB
 * 
 * Uses velesdb-wasm for in-browser/Node.js vector operations
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
  GraphCollectionConfig,
  CollectionStatsResponse,
  CollectionConfigResponse,
  SemanticEntry,
  EpisodicEvent,
  ProceduralPattern,
} from '../types';
import { ConnectionError, NotFoundError, VelesDBError } from '../types';
import type { SparseVector } from '../types';
import { wasmNotSupported } from './shared';

// Type definitions are loose to avoid strict type checking issues with dynamic WASM imports
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type WasmModule = any;

/** In-memory collection storage */
interface CollectionData {
  config: CollectionConfig;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  store: any;
  payloads: Map<string, Record<string, unknown>>;
  createdAt: Date;
}

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

  async init(): Promise<void> {
    if (this._initialized) {
      return;
    }

    try {
      // Dynamic import for WASM module
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

    // Validate all documents first
    for (const doc of docs) {
      const vectorLen = doc.vector.length;
      if (vectorLen !== collection.config.dimension) {
        throw new VelesDBError(
          `Vector dimension mismatch for doc ${doc.id}: expected ${collection.config.dimension}, got ${vectorLen}`,
          'DIMENSION_MISMATCH'
        );
      }
    }

    // Reserve capacity
    collection.store.reserve(docs.length);

    // Batch insert docs without payload; payload-bearing docs use insert_with_payload.
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

    // Store payloads
    for (const doc of docs) {
      if (doc.payload) {
        collection.payloads.set(this.canonicalPayloadKey(doc.id), doc.payload);
      }
    }
  }

  async search(
    collectionName: string,
    query: number[] | Float32Array,
    options?: SearchOptions
  ): Promise<SearchResult[]> {
    this.ensureInitialized();

    const collection = this.collections.get(collectionName);
    if (!collection) {
      throw new NotFoundError(`Collection '${collectionName}'`);
    }

    const queryVector = query instanceof Float32Array 
      ? query 
      : new Float32Array(query);

    if (queryVector.length !== collection.config.dimension) {
      throw new VelesDBError(
        `Query dimension mismatch: expected ${collection.config.dimension}, got ${queryVector.length}`,
        'DIMENSION_MISMATCH'
      );
    }

    const k = options?.k ?? 10;

    // Sparse/hybrid search handling
    if (options?.sparseVector) {
      const { indices, values } = this.sparseVectorToArrays(options.sparseVector);

      if (queryVector.length > 0 && collection.config.dimension && collection.config.dimension > 0) {
        // Hybrid: dense + sparse → use RRF fusion
        const denseResults = collection.store.search(queryVector, k) as Array<[bigint, number]>;
        const sparseResults = collection.store.sparse_search(
          new Uint32Array(indices),
          new Float32Array(values),
          k
        );

        // Parse sparse results
        const sparseArray = (sparseResults as Array<{ doc_id: bigint | number; score: number }>);

        // Format for hybrid_search_fuse: arrays of [doc_id, score]
        const denseForFuse = denseResults.map(([id, score]: [bigint, number]) => [Number(id), score]);
        const sparseForFuse = sparseArray.map((r: { doc_id: bigint | number; score: number }) => [Number(r.doc_id), r.score]);

        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const fused = (this.wasmModule as any).hybrid_search_fuse(denseForFuse, sparseForFuse, 60) as Array<{ doc_id: bigint | number; score: number }>;

        return fused.slice(0, k).map(r => ({
          id: String(r.doc_id),
          score: r.score,
          payload: collection.payloads.get(this.canonicalPayloadKeyFromResultId(r.doc_id)),
        }));
      } else {
        // Sparse-only search
        const sparseResults = collection.store.sparse_search(
          new Uint32Array(indices),
          new Float32Array(values),
          k
        ) as Array<{ doc_id: bigint | number; score: number }>;

        return sparseResults.map(r => ({
          id: String(r.doc_id),
          score: r.score,
          payload: collection.payloads.get(this.canonicalPayloadKeyFromResultId(r.doc_id)),
        }));
      }
    }

    if (options?.filter) {
      // Use the new search_with_filter method
      const results = collection.store.search_with_filter(queryVector, k, options.filter) as Array<{
        id: bigint;
        score: number;
        payload: any;
      }>;

      return results.map(r => ({
        id: String(r.id),
        score: r.score,
        payload: r.payload || collection.payloads.get(this.canonicalPayloadKeyFromResultId(r.id)),
      }));
    }

    const rawResults = collection.store.search(queryVector, k) as Array<[bigint, number]>;

    return rawResults.map(([id, score]: [bigint, number]) => {
      const stringId = String(id);
      const result: SearchResult = {
        id: stringId,
        score,
      };

      const payload = collection.payloads.get(this.canonicalPayloadKeyFromResultId(id));
      if (payload) {
        result.payload = payload;
      }

      return result;
    });
  }

  async searchBatch(
    collectionName: string,
    searches: Array<{
      vector: number[] | Float32Array;
      k?: number;
      filter?: Record<string, unknown>;
    }>
  ): Promise<SearchResult[][]> {
    this.ensureInitialized();

    const results: SearchResult[][] = [];
    for (const s of searches) {
      results.push(await this.search(collectionName, s.vector, { k: s.k, filter: s.filter }));
    }
    return results;
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

  async textSearch(
    _collection: string,
    _query: string,
    _options?: { k?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    const collection = this.collections.get(_collection);
    if (!collection) {
      throw new NotFoundError(`Collection '${_collection}'`);
    }
    const k = _options?.k ?? 10;
    const field = undefined;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const raw = collection.store.text_search(_query, k, field) as Array<any>;
    return raw.map((r: [bigint | number, number] | { id: bigint | number; score: number; payload?: Record<string, unknown> }) => {
      if (Array.isArray(r)) {
        const key = this.canonicalPayloadKeyFromResultId(r[0]);
        return { id: String(r[0]), score: r[1], payload: collection.payloads.get(key) };
      }
      const key = this.canonicalPayloadKeyFromResultId(r.id);
      return { id: String(r.id), score: r.score, payload: r.payload ?? collection.payloads.get(key) };
    });
  }

  async hybridSearch(
    _collection: string,
    _vector: number[] | Float32Array,
    _textQuery: string,
    _options?: { k?: number; vectorWeight?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    const collection = this.collections.get(_collection);
    if (!collection) {
      throw new NotFoundError(`Collection '${_collection}'`);
    }
    const queryVector = _vector instanceof Float32Array ? _vector : new Float32Array(_vector);
    const k = _options?.k ?? 10;
    const vectorWeight = _options?.vectorWeight ?? 0.5;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const raw = collection.store.hybrid_search(queryVector, _textQuery, k, vectorWeight) as Array<any>;
    return raw.map((r: { id: bigint | number; score: number; payload?: Record<string, unknown> }) => {
      const key = this.canonicalPayloadKeyFromResultId(r.id);
      return {
        id: String(r.id),
        score: r.score,
        payload: r.payload ?? collection.payloads.get(key),
      };
    });
  }

  async query(
    _collection: string,
    _queryString: string,
    _params?: Record<string, unknown>,
    _options?: QueryOptions
  ): Promise<QueryApiResponse> {
    this.ensureInitialized();
    const collection = this.collections.get(_collection);
    if (!collection) {
      throw new NotFoundError(`Collection '${_collection}'`);
    }
    const paramsVector = _params?.q;
    if (!Array.isArray(paramsVector) && !(paramsVector instanceof Float32Array)) {
      throw new VelesDBError(
        'WASM query() expects params.q to contain the query embedding vector.',
        'BAD_REQUEST'
      );
    }
    const requestedK = _params?.k;
    const k =
      typeof requestedK === 'number' && Number.isInteger(requestedK) && requestedK > 0
        ? requestedK
        : 10;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const raw = collection.store.query(
      paramsVector instanceof Float32Array ? paramsVector : new Float32Array(paramsVector),
      k
    ) as Array<any>;

    // v3.0.0: WASM query returns projected rows directly.
    return {
      results: raw as Record<string, unknown>[],
      stats: {
        executionTimeMs: 0,
        strategy: 'wasm-query',
        scannedNodes: raw.length,
      },
    };
  }

  async multiQuerySearch(
    _collection: string,
    _vectors: Array<number[] | Float32Array>,
    _options?: MultiQuerySearchOptions
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    const collection = this.collections.get(_collection);
    if (!collection) {
      throw new NotFoundError(`Collection '${_collection}'`);
    }
    if (_vectors.length === 0) {
      return [];
    }

    const numVectors = _vectors.length;
    const dimension = collection.config.dimension ?? 0;
    const flat = new Float32Array(numVectors * dimension);
    _vectors.forEach((vector, idx) => {
      const src = vector instanceof Float32Array ? vector : new Float32Array(vector);
      flat.set(src, idx * dimension);
    });

    const strategy = _options?.fusion ?? 'rrf';
    if (strategy === 'weighted') {
      throw new VelesDBError(
        "Fusion strategy 'weighted' is not supported in WASM backend.",
        'NOT_SUPPORTED'
      );
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const raw = collection.store.multi_query_search(
      flat,
      numVectors,
      _options?.k ?? 10,
      strategy,
      _options?.fusionParams?.k ?? 60
    ) as Array<any>;

    return raw.map((r: [bigint | number, number] | { id: bigint | number; score: number; payload?: Record<string, unknown> }) => {
      if (Array.isArray(r)) {
        const key = this.canonicalPayloadKeyFromResultId(r[0]);
        return { id: String(r[0]), score: r[1], payload: collection.payloads.get(key) };
      }
      const key = this.canonicalPayloadKeyFromResultId(r.id);
      return { id: String(r.id), score: r.score, payload: r.payload ?? collection.payloads.get(key) };
    });
  }


  async queryExplain(_queryString: string, _params?: Record<string, unknown>): Promise<ExplainResponse> {
    this.ensureInitialized();
    wasmNotSupported('Query explain');
  }

  async collectionSanity(_collection: string): Promise<CollectionSanityResponse> {
    this.ensureInitialized();
    wasmNotSupported('Collection sanity endpoint');
  }

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
    // Real persistence would require IndexedDB or similar
  }

  async close(): Promise<void> {
    for (const [, data] of this.collections) {
      data.store.free();
    }
    this.collections.clear();
    this._initialized = false;
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
    // Parse only canonical numeric strings, avoid parseInt partial parsing ("123abc" -> 123)
    const normalized = this.normalizeIdString(id);
    if (normalized !== null) {
      const parsed = Number(normalized);
      if (Number.isSafeInteger(parsed)) {
        return parsed;
      }
    }
    // Simple string hash for non-numeric IDs
    let hash = 0;
    for (let i = 0; i < id.length; i++) {
      const char = id.charCodeAt(i);
      hash = ((hash << 5) - hash) + char;
      hash = hash & hash; // Convert to 32bit integer
    }
    return Math.abs(hash);
  }

  // ========================================================================
  // Index Management (EPIC-009) - Stubs for WASM backend
  // Note: Full implementation requires velesdb-wasm support
  // ========================================================================

  async createIndex(_collection: string, _options: CreateIndexOptions): Promise<void> {
    this.ensureInitialized();
    // FIX: Throw error instead of silent warning for fail-fast behavior
    // This prevents confusion when switching between REST and WASM backends
    throw new Error(
      'WasmBackend: createIndex is not yet supported. ' +
      'Index operations require the REST backend with velesdb-server.'
    );
  }

  async listIndexes(_collection: string): Promise<IndexInfo[]> {
    this.ensureInitialized();
    // Return empty list - WASM backend has no indexes
    // This is acceptable since an empty list is semantically correct (no indexes exist)
    return [];
  }

  async hasIndex(_collection: string, _label: string, _property: string): Promise<boolean> {
    this.ensureInitialized();
    // Return false - WASM backend has no indexes
    // This is semantically correct (no index exists)
    return false;
  }

  async dropIndex(_collection: string, _label: string, _property: string): Promise<boolean> {
    this.ensureInitialized();
    // Return false - nothing to drop since no indexes exist in WASM backend
    return false;
  }

  // ========================================================================
  // Knowledge Graph (EPIC-016 US-041) - Stubs for WASM backend
  // Note: Graph operations require server-side EdgeStore
  // ========================================================================

  async addEdge(_collection: string, _edge: AddEdgeRequest): Promise<void> {
    this.ensureInitialized();
    wasmNotSupported('Knowledge Graph operations');
  }

  async getEdges(_collection: string, _options?: GetEdgesOptions): Promise<GraphEdge[]> {
    this.ensureInitialized();
    wasmNotSupported('Knowledge Graph operations');
  }

  async traverseGraph(_collection: string, _request: TraverseRequest): Promise<TraverseResponse> {
    this.ensureInitialized();
    wasmNotSupported('Graph traversal');
  }

  async getNodeDegree(_collection: string, _nodeId: number): Promise<DegreeResponse> {
    this.ensureInitialized();
    wasmNotSupported('Graph degree query');
  }

  // ========================================================================
  // Sparse / PQ / Streaming (v1.5)
  // ========================================================================

  async trainPq(_collection: string, _options?: PqTrainOptions): Promise<string> {
    this.ensureInitialized();
    wasmNotSupported('PQ training');
  }

  async streamInsert(_collection: string, _docs: VectorDocument[]): Promise<void> {
    this.ensureInitialized();
    wasmNotSupported('Streaming insert');
  }

  // ========================================================================
  // Graph Collection / Stats / Agent Memory (Phase 8) - WASM stubs
  // ========================================================================

  async createGraphCollection(_name: string, _config?: GraphCollectionConfig): Promise<void> {
    this.ensureInitialized();
    wasmNotSupported('Graph collections');
  }

  async getCollectionStats(_collection: string): Promise<CollectionStatsResponse | null> {
    this.ensureInitialized();
    wasmNotSupported('Collection stats');
  }

  async analyzeCollection(_collection: string): Promise<CollectionStatsResponse> {
    this.ensureInitialized();
    wasmNotSupported('Collection analyze');
  }

  async getCollectionConfig(_collection: string): Promise<CollectionConfigResponse> {
    this.ensureInitialized();
    wasmNotSupported('Collection config');
  }

  async searchIds(
    _collection: string,
    _query: number[] | Float32Array,
    _options?: SearchOptions
  ): Promise<Array<{ id: number; score: number }>> {
    this.ensureInitialized();
    wasmNotSupported('searchIds');
  }

  async storeSemanticFact(_collection: string, _entry: SemanticEntry): Promise<void> {
    this.ensureInitialized();
    wasmNotSupported('Agent memory');
  }

  async searchSemanticMemory(
    _collection: string,
    _embedding: number[],
    _k?: number
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    wasmNotSupported('Agent memory');
  }

  async recordEpisodicEvent(_collection: string, _event: EpisodicEvent): Promise<void> {
    this.ensureInitialized();
    wasmNotSupported('Agent memory');
  }

  async recallEpisodicEvents(
    _collection: string,
    _embedding: number[],
    _k?: number
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    wasmNotSupported('Agent memory');
  }

  async storeProceduralPattern(
    _collection: string,
    _pattern: ProceduralPattern
  ): Promise<void> {
    this.ensureInitialized();
    wasmNotSupported('Agent memory');
  }

  async matchProceduralPatterns(
    _collection: string,
    _embedding: number[],
    _k?: number
  ): Promise<SearchResult[]> {
    this.ensureInitialized();
    wasmNotSupported('Agent memory');
  }
}
