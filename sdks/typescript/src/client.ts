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
  CreateIndexOptions,
  IndexInfo,
  AddEdgeRequest,
  GetEdgesOptions,
  GraphEdge,
  TraverseRequest,
  TraverseResponse,
  DegreeResponse,
  QueryOptions,
  QueryResponse,
  MatchQueryOptions,
  MatchQueryResponse,
  ExplainResponse,
  HealthResponse,
  StreamTraverseOptions,
  StreamTraverseCallbacks,
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
   * Check server health without reinitializing.
   * Works even before init() for REST backends.
   */
  async health(): Promise<HealthResponse> {
    return this.backend.health();
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
   * Execute a VelesQL multi-model query (EPIC-031 US-011)
   * 
   * Supports hybrid vector + graph queries with VelesQL syntax.
   * 
   * @param collection - Collection name
   * @param queryString - VelesQL query string
   * @param params - Query parameters (vectors, scalars)
   * @param options - Query options (timeout, streaming)
   * @returns Query response with results and execution stats
   * 
   * @example
   * ```typescript
   * const response = await db.query('docs', `
   *   MATCH (d:Doc) WHERE vector NEAR $q LIMIT 20
   * `, { q: queryVector });
   * 
   * for (const r of response.results) {
   *   console.log(`Node ${r.nodeId}: ${r.fusedScore}`);
   * }
   * ```
   */
  async query(
    collection: string,
    queryString: string,
    params?: Record<string, unknown>,
    options?: QueryOptions
  ): Promise<QueryResponse> {
    this.ensureInitialized();

    if (!collection || typeof collection !== 'string') {
      throw new ValidationError('Collection name must be a non-empty string');
    }

    if (!queryString || typeof queryString !== 'string') {
      throw new ValidationError('Query string must be a non-empty string');
    }

    return this.backend.query(collection, queryString, params, options);
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

  // ========================================================================
  // Index Management (EPIC-009)
  // ========================================================================

  /**
   * Create a property index for O(1) equality lookups or O(log n) range queries
   * 
   * @param collection - Collection name
   * @param options - Index configuration (label, property, indexType)
   * 
   * @example
   * ```typescript
   * // Create hash index for fast email lookups
   * await db.createIndex('users', { label: 'Person', property: 'email' });
   * 
   * // Create range index for timestamp queries
   * await db.createIndex('events', { label: 'Event', property: 'timestamp', indexType: 'range' });
   * ```
   */
  async createIndex(collection: string, options: CreateIndexOptions): Promise<void> {
    this.ensureInitialized();
    
    if (!options.label || !options.property) {
      throw new ValidationError('Index requires label and property');
    }

    await this.backend.createIndex(collection, options);
  }

  /**
   * List all indexes on a collection
   * 
   * @param collection - Collection name
   * @returns Array of index information
   */
  async listIndexes(collection: string): Promise<IndexInfo[]> {
    this.ensureInitialized();
    return this.backend.listIndexes(collection);
  }

  /**
   * Check if an index exists
   * 
   * @param collection - Collection name
   * @param label - Node label
   * @param property - Property name
   * @returns true if index exists
   */
  async hasIndex(collection: string, label: string, property: string): Promise<boolean> {
    this.ensureInitialized();
    return this.backend.hasIndex(collection, label, property);
  }

  /**
   * Drop an index
   * 
   * @param collection - Collection name
   * @param label - Node label
   * @param property - Property name
   * @returns true if index was dropped, false if it didn't exist
   */
  async dropIndex(collection: string, label: string, property: string): Promise<boolean> {
    this.ensureInitialized();
    return this.backend.dropIndex(collection, label, property);
  }

  // ========================================================================
  // Knowledge Graph (EPIC-016 US-041)
  // ========================================================================

  /**
   * Add an edge to the collection's knowledge graph
   * 
   * @param collection - Collection name
   * @param edge - Edge to add (id, source, target, label, properties)
   * 
   * @example
   * ```typescript
   * await db.addEdge('social', {
   *   id: 1,
   *   source: 100,
   *   target: 200,
   *   label: 'FOLLOWS',
   *   properties: { since: '2024-01-01' }
   * });
   * ```
   */
  async addEdge(collection: string, edge: AddEdgeRequest): Promise<void> {
    this.ensureInitialized();
    
    if (!edge.label || typeof edge.label !== 'string') {
      throw new ValidationError('Edge label is required and must be a string');
    }
    
    if (typeof edge.source !== 'number' || typeof edge.target !== 'number') {
      throw new ValidationError('Edge source and target must be numbers');
    }

    await this.backend.addEdge(collection, edge);
  }

  /**
   * Get edges from the collection's knowledge graph
   * 
   * @param collection - Collection name
   * @param options - Query options (filter by label)
   * @returns Array of edges
   * 
   * @example
   * ```typescript
   * // Get all edges with label "FOLLOWS"
   * const edges = await db.getEdges('social', { label: 'FOLLOWS' });
   * ```
   */
  async getEdges(collection: string, options?: GetEdgesOptions): Promise<GraphEdge[]> {
    this.ensureInitialized();
    return this.backend.getEdges(collection, options);
  }

  // ========================================================================
  // Graph Traversal (EPIC-016 US-050)
  // ========================================================================

  /**
   * Traverse the graph using BFS or DFS from a source node
   * 
   * @param collection - Collection name
   * @param request - Traversal request options
   * @returns Traversal response with results and stats
   * 
   * @example
   * ```typescript
   * // BFS traversal from node 100
   * const result = await db.traverseGraph('social', {
   *   source: 100,
   *   strategy: 'bfs',
   *   maxDepth: 3,
   *   limit: 100,
   *   relTypes: ['FOLLOWS', 'KNOWS']
   * });
   * 
   * for (const node of result.results) {
   *   console.log(`Reached node ${node.targetId} at depth ${node.depth}`);
   * }
   * ```
   */
  async traverseGraph(collection: string, request: TraverseRequest): Promise<TraverseResponse> {
    this.ensureInitialized();
    
    if (typeof request.source !== 'number') {
      throw new ValidationError('Source node ID must be a number');
    }

    if (request.strategy && !['bfs', 'dfs'].includes(request.strategy)) {
      throw new ValidationError("Strategy must be 'bfs' or 'dfs'");
    }

    return this.backend.traverseGraph(collection, request);
  }

  /**
   * Get the in-degree and out-degree of a node
   * 
   * @param collection - Collection name
   * @param nodeId - Node ID
   * @returns Degree response with inDegree and outDegree
   * 
   * @example
   * ```typescript
   * const degree = await db.getNodeDegree('social', 100);
   * console.log(`In: ${degree.inDegree}, Out: ${degree.outDegree}`);
   * ```
   */
  async getNodeDegree(collection: string, nodeId: number): Promise<DegreeResponse> {
    this.ensureInitialized();
    
    if (typeof nodeId !== 'number') {
      throw new ValidationError('Node ID must be a number');
    }

    return this.backend.getNodeDegree(collection, nodeId);
  }

  // ========================================================================
  // MATCH Query (Graph Traversal via VelesQL)
  // ========================================================================

  /**
   * Execute a MATCH graph traversal query
   * 
   * Calls the server's MATCH endpoint for Cypher-like graph pattern matching.
   * Only available with the REST backend.
   * 
   * @param collection - Collection name (used in endpoint URL)
   * @param queryString - VelesQL MATCH query string
   * @param params - Query parameters (e.g., vector bindings)
   * @param options - Optional vector and threshold for similarity matching
   * @returns MATCH query response with results, timing, and count
   * 
   * @example
   * ```typescript
   * const result = await db.matchQuery('docs',
   *   'MATCH (a:Person)-[:KNOWS]->(b) WHERE similarity(a.embedding, $v) > 0.8 RETURN a.name',
   *   { v: queryVector },
   *   { vector: queryVector, threshold: 0.8 }
   * );
   * 
   * for (const r of result.results) {
   *   console.log(`${r.bindings.a} → score: ${r.score}`);
   * }
   * ```
   */
  async matchQuery(
    collection: string,
    queryString: string,
    params?: Record<string, unknown>,
    options?: MatchQueryOptions
  ): Promise<MatchQueryResponse> {
    this.ensureInitialized();

    if (!collection || typeof collection !== 'string') {
      throw new ValidationError('Collection name must be a non-empty string');
    }

    if (!queryString || typeof queryString !== 'string') {
      throw new ValidationError('Query string must be a non-empty string');
    }

    return this.backend.matchQuery(collection, queryString, params, options);
  }

  // ========================================================================
  // EXPLAIN Query (EPIC-058 US-002)
  // ========================================================================

  /**
   * Explain a VelesQL query without executing it.
   * 
   * Returns the query plan, estimated costs, and detected features.
   * Useful for understanding query performance before execution.
   * 
   * @param queryString - VelesQL query to explain
   * @param params - Named parameters for the query (optional)
   * @returns Query plan with steps, cost estimation, and feature detection
   * 
   * @example
   * ```typescript
   * const plan = await db.explain(
   *   'SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8 LIMIT 10'
   * );
   * console.log(plan.queryType);              // 'SELECT'
   * console.log(plan.estimatedCost.complexity); // 'O(log n)'
   * console.log(plan.features.hasVectorSearch); // true
   * ```
   */
  async explain(
    queryString: string,
    params?: Record<string, unknown>
  ): Promise<ExplainResponse> {
    this.ensureInitialized();

    if (!queryString || typeof queryString !== 'string') {
      throw new ValidationError('Query string must be a non-empty string');
    }

    return this.backend.explain(queryString, params);
  }

  // ========================================================================
  // Streaming Graph Traversal (SSE)
  // ========================================================================

  /**
   * Stream graph traversal results via Server-Sent Events.
   * 
   * Connects to the server's SSE endpoint and invokes callbacks as nodes
   * are discovered during traversal. Only available with the REST backend.
   * 
   * @param collection - Collection name
   * @param options - Traversal options (source, strategy, maxDepth, limit, relTypes)
   * @param callbacks - Event callbacks (onNode, onStats, onDone, onError)
   * 
   * @example
   * ```typescript
   * await db.streamTraverseGraph('social', { source: 1, strategy: 'bfs', maxDepth: 3 }, {
   *   onNode: (node) => console.log(`Node ${node.id} at depth ${node.depth}`),
   *   onDone: (done) => console.log(`Traversal complete: ${done.totalNodes} nodes`),
   * });
   * ```
   */
  async streamTraverseGraph(
    collection: string,
    options: StreamTraverseOptions,
    callbacks: StreamTraverseCallbacks
  ): Promise<void> {
    this.ensureInitialized();

    if (!collection || typeof collection !== 'string') {
      throw new ValidationError('Collection name must be a non-empty string');
    }

    if (typeof options.source !== 'number') {
      throw new ValidationError('Source node ID must be a number');
    }

    if (options.strategy && options.strategy !== 'bfs' && options.strategy !== 'dfs') {
      throw new ValidationError("Strategy must be 'bfs' or 'dfs'");
    }

    return this.backend.streamTraverseGraph(collection, options, callbacks);
  }
}
