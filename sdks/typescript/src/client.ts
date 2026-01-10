/**
 * VelesDB Client - Unified interface for all backends
 */

import type {
  VelesDBConfig,
  CollectionConfig,
  Collection,
  VectorDocument,
  SearchOptions,
  SearchResult,
  IVelesDBBackend,
  MultiQuerySearchOptions,
} from './types';
import { ValidationError } from './types';
import { WasmBackend } from './backends/wasm';
import { RestBackend } from './backends/rest';

/**
 * VelesDB Client
 * 
 * Provides a unified interface for interacting with VelesDB
 * using either WASM (browser/Node.js) or REST API backends.
 * 
 * @example
 * ```typescript
 * const db = new VelesDB({ backend: 'wasm' });
 * await db.init();
 * 
 * await db.createCollection('embeddings', { dimension: 768, metric: 'cosine' });
 * await db.insert('embeddings', { id: 'doc1', vector: [...], payload: { title: 'Hello' } });
 * 
 * const results = await db.search('embeddings', queryVector, { k: 5 });
 * ```
 */
export class VelesDB {
  private readonly config: VelesDBConfig;
  private backend: IVelesDBBackend;
  private initialized = false;

  /**
   * Create a new VelesDB client
   * 
   * @param config - Client configuration
   * @throws {ValidationError} If configuration is invalid
   */
  constructor(config: VelesDBConfig) {
    this.validateConfig(config);
    this.config = config;
    this.backend = this.createBackend(config);
  }

  private validateConfig(config: VelesDBConfig): void {
    if (!config.backend) {
      throw new ValidationError('Backend type is required');
    }

    if (config.backend !== 'wasm' && config.backend !== 'rest') {
      throw new ValidationError(`Invalid backend type: ${config.backend}. Use 'wasm' or 'rest'`);
    }

    if (config.backend === 'rest' && !config.url) {
      throw new ValidationError('URL is required for REST backend');
    }
  }

  private createBackend(config: VelesDBConfig): IVelesDBBackend {
    switch (config.backend) {
      case 'wasm':
        return new WasmBackend();
      case 'rest':
        return new RestBackend(config.url!, config.apiKey, config.timeout);
      default:
        throw new ValidationError(`Unknown backend: ${config.backend}`);
    }
  }

  /**
   * Initialize the client
   * Must be called before any other operations
   */
  async init(): Promise<void> {
    if (this.initialized) {
      return;
    }
    await this.backend.init();
    this.initialized = true;
  }

  /**
   * Check if client is initialized
   */
  isInitialized(): boolean {
    return this.initialized;
  }

  private ensureInitialized(): void {
    if (!this.initialized) {
      throw new ValidationError('Client not initialized. Call init() first.');
    }
  }

  /**
   * Create a new collection
   * 
   * @param name - Collection name
   * @param config - Collection configuration
   */
  async createCollection(name: string, config: CollectionConfig): Promise<void> {
    this.ensureInitialized();
    
    if (!name || typeof name !== 'string') {
      throw new ValidationError('Collection name must be a non-empty string');
    }
    
    // Dimension is required for vector collections, not for metadata_only
    const isMetadataOnly = config.collectionType === 'metadata_only';
    if (!isMetadataOnly && (!config.dimension || config.dimension <= 0)) {
      throw new ValidationError('Dimension must be a positive integer for vector collections');
    }

    await this.backend.createCollection(name, config);
  }

  /**
   * Create a metadata-only collection (no vectors, just payload data)
   * 
   * Useful for storing reference data that can be JOINed with vector collections.
   * 
   * @param name - Collection name
   * 
   * @example
   * ```typescript
   * await db.createMetadataCollection('products');
   * await db.insertMetadata('products', { id: 'P001', name: 'Widget', price: 99 });
   * ```
   */
  async createMetadataCollection(name: string): Promise<void> {
    this.ensureInitialized();
    
    if (!name || typeof name !== 'string') {
      throw new ValidationError('Collection name must be a non-empty string');
    }

    await this.backend.createCollection(name, { collectionType: 'metadata_only' });
  }

  /**
   * Delete a collection
   * 
   * @param name - Collection name
   */
  async deleteCollection(name: string): Promise<void> {
    this.ensureInitialized();
    await this.backend.deleteCollection(name);
  }

  /**
   * Get collection information
   * 
   * @param name - Collection name
   * @returns Collection info or null if not found
   */
  async getCollection(name: string): Promise<Collection | null> {
    this.ensureInitialized();
    return this.backend.getCollection(name);
  }

  /**
   * List all collections
   * 
   * @returns Array of collections
   */
  async listCollections(): Promise<Collection[]> {
    this.ensureInitialized();
    return this.backend.listCollections();
  }

  /**
   * Insert a vector document
   * 
   * @param collection - Collection name
   * @param doc - Document to insert
   */
  async insert(collection: string, doc: VectorDocument): Promise<void> {
    this.ensureInitialized();
    this.validateDocument(doc);
    await this.backend.insert(collection, doc);
  }

  /**
   * Insert multiple vector documents
   * 
   * @param collection - Collection name
   * @param docs - Documents to insert
   */
  async insertBatch(collection: string, docs: VectorDocument[]): Promise<void> {
    this.ensureInitialized();
    
    if (!Array.isArray(docs)) {
      throw new ValidationError('Documents must be an array');
    }

    for (const doc of docs) {
      this.validateDocument(doc);
    }

    await this.backend.insertBatch(collection, docs);
  }

  private validateDocument(doc: VectorDocument): void {
    if (doc.id === undefined || doc.id === null) {
      throw new ValidationError('Document ID is required');
    }

    if (!doc.vector) {
      throw new ValidationError('Document vector is required');
    }

    if (!Array.isArray(doc.vector) && !(doc.vector instanceof Float32Array)) {
      throw new ValidationError('Vector must be an array or Float32Array');
    }
  }

  /**
   * Search for similar vectors
   * 
   * @param collection - Collection name
   * @param query - Query vector
   * @param options - Search options
   * @returns Search results sorted by relevance
   */
  async search(
    collection: string,
    query: number[] | Float32Array,
    options?: SearchOptions
  ): Promise<SearchResult[]> {
    this.ensureInitialized();

    if (!query || (!Array.isArray(query) && !(query instanceof Float32Array))) {
      throw new ValidationError('Query must be an array or Float32Array');
    }

    return this.backend.search(collection, query, options);
  }

  /**
   * Search for multiple vectors in parallel
   * 
   * @param collection - Collection name
   * @param searches - List of search queries
   * @returns List of search results for each query
   */
  async searchBatch(
    collection: string,
    searches: Array<{
      vector: number[] | Float32Array;
      k?: number;
      filter?: Record<string, unknown>;
    }>
  ): Promise<SearchResult[][]> {
    this.ensureInitialized();

    if (!Array.isArray(searches)) {
      throw new ValidationError('Searches must be an array');
    }

    for (const s of searches) {
      if (!s.vector || (!Array.isArray(s.vector) && !(s.vector instanceof Float32Array))) {
        throw new ValidationError('Each search must have a vector (array or Float32Array)');
      }
    }

    return this.backend.searchBatch(collection, searches);
  }

  /**
   * Delete a vector by ID
   * 
   * @param collection - Collection name
   * @param id - Document ID
   * @returns true if deleted, false if not found
   */
  async delete(collection: string, id: string | number): Promise<boolean> {
    this.ensureInitialized();
    return this.backend.delete(collection, id);
  }

  /**
   * Get a vector by ID
   * 
   * @param collection - Collection name
   * @param id - Document ID
   * @returns Document or null if not found
   */
  async get(collection: string, id: string | number): Promise<VectorDocument | null> {
    this.ensureInitialized();
    return this.backend.get(collection, id);
  }

  /**
   * Perform full-text search using BM25
   * 
   * @param collection - Collection name
   * @param query - Text query
   * @param options - Search options (k, filter)
   * @returns Search results sorted by BM25 score
   */
  async textSearch(
    collection: string,
    query: string,
    options?: { k?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]> {
    this.ensureInitialized();

    if (!query || typeof query !== 'string') {
      throw new ValidationError('Query must be a non-empty string');
    }

    return this.backend.textSearch(collection, query, options);
  }

  /**
   * Perform hybrid search combining vector similarity and BM25 text search
   * 
   * @param collection - Collection name
   * @param vector - Query vector
   * @param textQuery - Text query for BM25
   * @param options - Search options (k, vectorWeight, filter)
   * @returns Search results sorted by fused score
   */
  async hybridSearch(
    collection: string,
    vector: number[] | Float32Array,
    textQuery: string,
    options?: { k?: number; vectorWeight?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]> {
    this.ensureInitialized();

    if (!vector || (!Array.isArray(vector) && !(vector instanceof Float32Array))) {
      throw new ValidationError('Vector must be an array or Float32Array');
    }

    if (!textQuery || typeof textQuery !== 'string') {
      throw new ValidationError('Text query must be a non-empty string');
    }

    return this.backend.hybridSearch(collection, vector, textQuery, options);
  }

  /**
   * Execute a VelesQL query
   * 
   * @param queryString - VelesQL query string
   * @param params - Optional query parameters
   * @returns Query results
   */
  async query(
    queryString: string,
    params?: Record<string, unknown>
  ): Promise<SearchResult[]> {
    this.ensureInitialized();

    if (!queryString || typeof queryString !== 'string') {
      throw new ValidationError('Query string must be a non-empty string');
    }

    return this.backend.query(queryString, params);
  }

  /**
   * Multi-query fusion search combining results from multiple query vectors
   * 
   * Ideal for RAG pipelines using Multiple Query Generation (MQG).
   * 
   * @param collection - Collection name
   * @param vectors - Array of query vectors
   * @param options - Search options (k, fusion strategy, fusionParams, filter)
   * @returns Fused search results
   * 
   * @example
   * ```typescript
   * // RRF fusion (default)
   * const results = await db.multiQuerySearch('docs', [emb1, emb2, emb3], {
   *   k: 10,
   *   fusion: 'rrf',
   *   fusionParams: { k: 60 }
   * });
   * 
   * // Weighted fusion
   * const results = await db.multiQuerySearch('docs', [emb1, emb2], {
   *   k: 10,
   *   fusion: 'weighted',
   *   fusionParams: { avgWeight: 0.6, maxWeight: 0.3, hitWeight: 0.1 }
   * });
   * ```
   */
  async multiQuerySearch(
    collection: string,
    vectors: Array<number[] | Float32Array>,
    options?: MultiQuerySearchOptions
  ): Promise<SearchResult[]> {
    this.ensureInitialized();

    if (!Array.isArray(vectors) || vectors.length === 0) {
      throw new ValidationError('Vectors must be a non-empty array');
    }

    for (const v of vectors) {
      if (!Array.isArray(v) && !(v instanceof Float32Array)) {
        throw new ValidationError('Each vector must be an array or Float32Array');
      }
    }

    return this.backend.multiQuerySearch(collection, vectors, options);
  }

  /**
   * Check if a collection is empty
   * 
   * @param collection - Collection name
   * @returns true if empty, false otherwise
   */
  async isEmpty(collection: string): Promise<boolean> {
    this.ensureInitialized();
    return this.backend.isEmpty(collection);
  }

  /**
   * Flush pending changes to disk
   * 
   * @param collection - Collection name
   */
  async flush(collection: string): Promise<void> {
    this.ensureInitialized();
    await this.backend.flush(collection);
  }

  /**
   * Close the client and release resources
   */
  async close(): Promise<void> {
    if (this.initialized) {
      await this.backend.close();
      this.initialized = false;
    }
  }

  /**
   * Get the current backend type
   */
  get backendType(): string {
    return this.config.backend;
  }
}
