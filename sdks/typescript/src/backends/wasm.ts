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
} from '../types';
import { ConnectionError, NotFoundError, VelesDBError } from '../types';

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
      dimension: collection.config.dimension,
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
        dimension: data.config.dimension,
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

    collection.store.insert(BigInt(id), vector);

    if (doc.payload) {
      collection.payloads.set(String(doc.id), doc.payload);
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
      const vectorLen = Array.isArray(doc.vector) ? doc.vector.length : doc.vector.length;
      if (vectorLen !== collection.config.dimension) {
        throw new VelesDBError(
          `Vector dimension mismatch for doc ${doc.id}: expected ${collection.config.dimension}, got ${vectorLen}`,
          'DIMENSION_MISMATCH'
        );
      }
    }

    // Reserve capacity
    collection.store.reserve(docs.length);

    // Batch insert
    const batch: Array<[bigint, number[]]> = docs.map(doc => [
      BigInt(this.toNumericId(doc.id)),
      Array.isArray(doc.vector) ? doc.vector : Array.from(doc.vector),
    ]);

    collection.store.insert_batch(batch);

    // Store payloads
    for (const doc of docs) {
      if (doc.payload) {
        collection.payloads.set(String(doc.id), doc.payload);
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
        payload: r.payload || collection.payloads.get(String(r.id)),
      }));
    }

    const rawResults = collection.store.search(queryVector, k) as Array<[bigint, number]>;

    return rawResults.map(([id, score]: [bigint, number]) => {
      const stringId = String(id);
      const result: SearchResult = {
        id: stringId,
        score,
      };

      const payload = collection.payloads.get(stringId);
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
      collection.payloads.delete(String(id));
    }

    return removed;
  }

  async get(collectionName: string, id: string | number): Promise<VectorDocument | null> {
    this.ensureInitialized();

    const collection = this.collections.get(collectionName);
    if (!collection) {
      throw new NotFoundError(`Collection '${collectionName}'`);
    }

    // WASM backend doesn't support direct get by ID
    // This is a limitation - would need to implement in Rust
    const payload = collection.payloads.get(String(id));
    if (!payload) {
      return null;
    }

    return {
      id,
      vector: [], // Not available in current WASM impl
      payload,
    };
  }

  async textSearch(
    _collection: string,
    _query: string,
    _options?: { k?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]> {
    // WASM backend doesn't support BM25 text search
    // Use REST backend for full-text search capabilities
    throw new VelesDBError(
      'Text search is not supported in WASM backend. Use REST backend for BM25 search.',
      'NOT_SUPPORTED'
    );
  }

  async hybridSearch(
    _collection: string,
    _vector: number[] | Float32Array,
    _textQuery: string,
    _options?: { k?: number; vectorWeight?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]> {
    // WASM backend doesn't support hybrid search (requires BM25)
    // Use REST backend for hybrid search capabilities
    throw new VelesDBError(
      'Hybrid search is not supported in WASM backend. Use REST backend for hybrid search.',
      'NOT_SUPPORTED'
    );
  }

  async query(
    _queryString: string,
    _params?: Record<string, unknown>
  ): Promise<SearchResult[]> {
    // WASM backend doesn't support VelesQL
    // Use REST backend for VelesQL queries
    throw new VelesDBError(
      'VelesQL queries are not supported in WASM backend. Use REST backend for query support.',
      'NOT_SUPPORTED'
    );
  }

  async close(): Promise<void> {
    for (const [, data] of this.collections) {
      data.store.free();
    }
    this.collections.clear();
    this._initialized = false;
  }

  private toNumericId(id: string | number): number {
    if (typeof id === 'number') {
      return id;
    }
    // Try to parse as number, otherwise use hash
    const parsed = parseInt(id, 10);
    if (!isNaN(parsed)) {
      return parsed;
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
}
