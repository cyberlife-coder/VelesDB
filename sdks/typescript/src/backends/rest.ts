/**
 * REST Backend for VelesDB
 * 
 * Connects to VelesDB server via REST API
 */

import type {
  IVelesDBBackend,
  CollectionConfig,
  Collection,
  DistanceMetric,
  StorageMode,
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
  QueryResponse,
  MatchQueryOptions,
  MatchQueryResponse,
} from '../types';
import { ConnectionError, NotFoundError, VelesDBError } from '../types';

/** REST API response wrapper */
interface ApiResponse<T> {
  data?: T;
  error?: {
    code: string;
    message: string;
  };
}

/** Batch search response structure */
interface BatchSearchResponse {
  results: Array<{ results: SearchResult[] }>;
}

/** Server-side MATCH response (snake_case contract — internal) */
interface ServerMatchQueryResponse {
  results: Array<{
    bindings: Record<string, unknown>;
    score?: number | null;
    depth: number;
    projected?: Record<string, unknown>;
  }>;
  took_ms: number;
  count: number;
}

/** Server SELECT /query response */
interface ServerSelectQueryResponse {
  results: Array<{
    id: number;
    score: number;
    payload: Record<string, unknown> | null;
  }>;
  timing_ms: number;
  rows_returned: number;
}

/** Server aggregation /query response */
interface ServerAggregationResponse {
  result: unknown;
  timing_ms: number;
}

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
  private _initPromise: Promise<void> | null = null;

  constructor(url: string, apiKey?: string, timeout = 30000) {
    this.baseUrl = url.replace(/\/$/, ''); // Remove trailing slash
    this.apiKey = apiKey;
    this.timeout = timeout;
  }

  async init(): Promise<void> {
    if (this._initialized) return;
    if (this._initPromise) return this._initPromise;

    this._initPromise = this._performInit();
    try {
      await this._initPromise;
    } finally {
      this._initPromise = null;
    }
  }

  private async _performInit(): Promise<void> {
    try {
      // Health check
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
      case 400:
        return 'BAD_REQUEST';
      case 401:
        return 'UNAUTHORIZED';
      case 403:
        return 'FORBIDDEN';
      case 404:
        return 'NOT_FOUND';
      case 409:
        return 'CONFLICT';
      case 429:
        return 'RATE_LIMITED';
      case 500:
        return 'INTERNAL_ERROR';
      case 503:
        return 'SERVICE_UNAVAILABLE';
      default:
        return 'UNKNOWN_ERROR';
    }
  }

  private extractErrorPayload(data: unknown): { code?: string; message?: string } {
    if (!data || typeof data !== 'object') {
      return {};
    }

    const payload = data as Record<string, unknown>;
    const code = typeof payload.code === 'string' ? payload.code : undefined;
    const messageField = payload.message ?? payload.error;
    const message = typeof messageField === 'string' ? messageField : undefined;
    return { code, message };
  }

  /**
   * Parse node ID safely to handle u64 values above Number.MAX_SAFE_INTEGER.
   * Returns bigint for large values, number for safe values.
   */
  private parseNodeId(value: unknown): bigint | number {
    if (value === null || value === undefined) {
      return 0;
    }
    
    // If already a bigint, return as-is
    if (typeof value === 'bigint') {
      return value;
    }
    
    // If string (JSON may serialize large numbers as strings), parse as BigInt
    if (typeof value === 'string') {
      const num = Number(value);
      if (num > Number.MAX_SAFE_INTEGER) {
        return BigInt(value);
      }
      return num;
    }
    
    // If number, check if precision is at risk
    if (typeof value === 'number') {
      if (value > Number.MAX_SAFE_INTEGER) {
        // Precision already lost, but return as-is (best effort)
        // Note: This case indicates the API should return strings for large IDs
        return value;
      }
      return value;
    }
    
    return 0;
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown
  ): Promise<ApiResponse<T>> {
    const url = `${this.baseUrl}${path}`;
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };

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

  async createCollection(name: string, config: CollectionConfig): Promise<void> {
    this.ensureInitialized();

    const response = await this.request('POST', '/collections', {
      name,
      dimension: config.dimension,
      metric: config.metric ?? 'cosine',
      storage_mode: config.storageMode ?? 'full',
      collection_type: config.collectionType ?? 'vector',
      description: config.description,
    });

    if (response.error) {
      throw new VelesDBError(response.error.message, response.error.code);
    }
  }

  async deleteCollection(name: string): Promise<void> {
    this.ensureInitialized();

    const response = await this.request('DELETE', `/collections/${encodeURIComponent(name)}`);

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${name}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }
  }

  async getCollection(name: string): Promise<Collection | null> {
    this.ensureInitialized();

    const response = await this.request<Collection>(
      'GET',
      `/collections/${encodeURIComponent(name)}`
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        return null;
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return response.data ?? null;
  }

  async listCollections(): Promise<Collection[]> {
    this.ensureInitialized();

    interface ServerCollectionResponse {
      name: string;
      dimension: number;
      metric: string;
      point_count: number;
      storage_mode: string;
    }

    const response = await this.request<{ collections: ServerCollectionResponse[] }>('GET', '/collections');

    if (response.error) {
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return (response.data?.collections ?? []).map(c => ({
      name: c.name,
      dimension: c.dimension,
      metric: c.metric as DistanceMetric,
      count: c.point_count,
      storageMode: c.storage_mode as StorageMode,
    }));
  }

  async insert(collection: string, doc: VectorDocument): Promise<void> {
    this.ensureInitialized();

    const vector = doc.vector instanceof Float32Array 
      ? Array.from(doc.vector) 
      : doc.vector;

    const response = await this.request(
      'POST',
      `/collections/${encodeURIComponent(collection)}/points`,
      {
        points: [{
          id: doc.id,
          vector,
          payload: doc.payload,
        }],
      }
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }
  }

  async insertBatch(collection: string, docs: VectorDocument[]): Promise<void> {
    this.ensureInitialized();

    const vectors = docs.map(doc => ({
      id: doc.id,
      vector: doc.vector instanceof Float32Array ? Array.from(doc.vector) : doc.vector,
      payload: doc.payload,
    }));

    const response = await this.request(
      'POST',
      `/collections/${encodeURIComponent(collection)}/points`,
      { points: vectors }
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }
  }

  async search(
    collection: string,
    query: number[] | Float32Array,
    options?: SearchOptions
  ): Promise<SearchResult[]> {
    this.ensureInitialized();

    const queryVector = query instanceof Float32Array ? Array.from(query) : query;

    const response = await this.request<{ results: SearchResult[] }>(
      'POST',
      `/collections/${encodeURIComponent(collection)}/search`,
      {
        vector: queryVector,
        k: options?.k ?? 10,
        filter: options?.filter,
        include_vectors: options?.includeVectors ?? false,
      }
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return response.data?.results ?? [];
  }

  async searchBatch(
    collection: string,
    searches: Array<{
      vector: number[] | Float32Array;
      k?: number;
      filter?: Record<string, unknown>;
    }>
  ): Promise<SearchResult[][]> {
    this.ensureInitialized();

    const formattedSearches = searches.map(s => ({
      vector: s.vector instanceof Float32Array ? Array.from(s.vector) : s.vector,
      top_k: s.k ?? 10,
      filter: s.filter,
    }));

    const response = await this.request<BatchSearchResponse>(
      'POST',
      `/collections/${encodeURIComponent(collection)}/search/batch`,
      { searches: formattedSearches }
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return response.data?.results.map(r => r.results) ?? [];
  }

  async delete(collection: string, id: string | number): Promise<boolean> {
    this.ensureInitialized();

    const response = await this.request<{ deleted: boolean }>(
      'DELETE',
      `/collections/${encodeURIComponent(collection)}/points/${encodeURIComponent(String(id))}`
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        return false;
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return response.data?.deleted ?? true;
  }

  async get(collection: string, id: string | number): Promise<VectorDocument | null> {
    this.ensureInitialized();

    const response = await this.request<VectorDocument>(
      'GET',
      `/collections/${encodeURIComponent(collection)}/points/${encodeURIComponent(String(id))}`
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        return null;
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return response.data ?? null;
  }

  async textSearch(
    collection: string,
    query: string,
    options?: { k?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]> {
    this.ensureInitialized();

    const response = await this.request<{ results: SearchResult[] }>(
      'POST',
      `/collections/${encodeURIComponent(collection)}/search/text`,
      {
        query,
        top_k: options?.k ?? 10,
        filter: options?.filter,
      }
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return response.data?.results ?? [];
  }

  async hybridSearch(
    collection: string,
    vector: number[] | Float32Array,
    textQuery: string,
    options?: { k?: number; vectorWeight?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]> {
    this.ensureInitialized();

    const queryVector = vector instanceof Float32Array ? Array.from(vector) : vector;

    const response = await this.request<{ results: SearchResult[] }>(
      'POST',
      `/collections/${encodeURIComponent(collection)}/search/hybrid`,
      {
        vector: queryVector,
        query: textQuery,
        top_k: options?.k ?? 10,
        vector_weight: options?.vectorWeight ?? 0.5,
        filter: options?.filter,
      }
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return response.data?.results ?? [];
  }

  /**
   * Execute a VelesQL SELECT query.
   * 
   * For MATCH queries, use `matchQuery()` or pass a MATCH query here
   * for automatic routing to the correct endpoint.
   * 
   * @param collection - Collection name. For SELECT: error context only
   *   (server reads FROM clause). For MATCH: used in the endpoint URL.
   * @param queryString - VelesQL query string
   * @param params - Query parameters
   * @param options - Query options (supports vector/threshold for MATCH pass-through)
   */
  async query(
    collection: string,
    queryString: string,
    params?: Record<string, unknown>,
    options?: QueryOptions
  ): Promise<QueryResponse> {
    this.ensureInitialized();

    // Smart routing: detect MATCH queries and delegate to matchQuery()
    const trimmed = queryString.trim();
    if (trimmed.toUpperCase().startsWith('MATCH')) {
      const matchResult = await this.matchQuery(collection, queryString, params, {
        vector: options?.vector,
        threshold: options?.threshold,
      });
      // Adapt MatchQueryResponse → QueryResponse for unified interface
      return {
        results: matchResult.results.map(r => ({
          nodeId: Object.values(r.bindings)[0] as bigint | number ?? 0,
          vectorScore: r.score,
          graphScore: null,
          fusedScore: r.score ?? 0,
          bindings: { ...r.bindings, ...r.projected },
          columnData: null,
        })),
        stats: {
          executionTimeMs: matchResult.tookMs,
          strategy: 'match',
          scannedNodes: matchResult.count,
        },
      };
    }

    // SELECT query → POST /query
    const response = await this.request<ServerSelectQueryResponse | ServerAggregationResponse>(
      'POST', '/query',
      { query: queryString, params: params ?? {} }
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    const rawData = response.data;

    // Detect aggregation response: has `result` (singular) instead of `results` (plural)
    if (rawData && 'result' in rawData && !('results' in rawData)) {
      const aggData = rawData as ServerAggregationResponse;
      return {
        results: [{
          nodeId: 0,
          vectorScore: null,
          graphScore: null,
          fusedScore: 0,
          bindings: typeof aggData.result === 'object' && aggData.result !== null
            ? aggData.result as Record<string, unknown>
            : { value: aggData.result },
          columnData: null,
        }],
        stats: {
          executionTimeMs: aggData.timing_ms ?? 0,
          strategy: 'aggregation',
          scannedNodes: 0,
        },
      };
    }

    // Standard SELECT response
    const selectData = rawData as ServerSelectQueryResponse;
    return {
      results: (selectData?.results ?? []).map((r) => ({
        nodeId: this.parseNodeId(r.id),
        vectorScore: r.score ?? null,
        graphScore: null,
        fusedScore: r.score ?? 0,
        bindings: r.payload ?? {},
        columnData: null,
      })),
      stats: {
        executionTimeMs: selectData?.timing_ms ?? 0,
        strategy: 'select',
        scannedNodes: selectData?.rows_returned ?? 0,
      },
    };
  }

  async multiQuerySearch(
    collection: string,
    vectors: Array<number[] | Float32Array>,
    options?: MultiQuerySearchOptions
  ): Promise<SearchResult[]> {
    this.ensureInitialized();

    const formattedVectors = vectors.map(v => 
      v instanceof Float32Array ? Array.from(v) : v
    );

    const response = await this.request<{ results: SearchResult[] }>(
      'POST',
      `/collections/${encodeURIComponent(collection)}/search/multi`,
      {
        vectors: formattedVectors,
        top_k: options?.k ?? 10,
        strategy: options?.fusion ?? 'rrf',
        rrf_k: options?.fusionParams?.k ?? 60,
        filter: options?.filter,
      }
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return response.data?.results ?? [];
  }

  /**
   * Execute a MATCH graph traversal query.
   * 
   * Calls `POST /collections/{name}/match` on the server.
   * 
   * @param collection - Collection name (used in endpoint URL)
   * @param queryString - VelesQL MATCH query string
   * @param params - Query parameters (e.g., vector bindings)
   * @param options - Optional vector and threshold for similarity matching
   */
  async matchQuery(
    collection: string,
    queryString: string,
    params?: Record<string, unknown>,
    options?: MatchQueryOptions
  ): Promise<MatchQueryResponse> {
    this.ensureInitialized();

    const body: Record<string, unknown> = {
      query: queryString,
      params: params ?? {},
    };

    if (options?.vector) {
      body.vector = options.vector instanceof Float32Array
        ? Array.from(options.vector)
        : options.vector;
    }
    if (options?.threshold !== undefined) {
      body.threshold = options.threshold;
    }

    const response = await this.request<ServerMatchQueryResponse>(
      'POST',
      `/collections/${encodeURIComponent(collection)}/match`,
      body
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    const data = response.data;
    return {
      results: (data?.results ?? []).map(r => ({
        bindings: r.bindings,
        score: r.score ?? null,
        depth: r.depth,
        projected: r.projected ?? {},
      })),
      tookMs: data?.took_ms ?? 0,
      count: data?.count ?? 0,
    };
  }

  async isEmpty(collection: string): Promise<boolean> {
    this.ensureInitialized();

    const response = await this.request<{ is_empty: boolean }>(
      'GET',
      `/collections/${encodeURIComponent(collection)}/empty`
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return response.data?.is_empty ?? true;
  }

  async flush(collection: string): Promise<void> {
    this.ensureInitialized();

    const response = await this.request(
      'POST',
      `/collections/${encodeURIComponent(collection)}/flush`
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }
  }

  async close(): Promise<void> {
    this._initialized = false;
  }

  // ========================================================================
  // Index Management (EPIC-009)
  // ========================================================================

  async createIndex(collection: string, options: CreateIndexOptions): Promise<void> {
    this.ensureInitialized();

    const response = await this.request(
      'POST',
      `/collections/${encodeURIComponent(collection)}/indexes`,
      {
        label: options.label,
        property: options.property,
        index_type: options.indexType ?? 'hash',
      }
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }
  }

  async listIndexes(collection: string): Promise<IndexInfo[]> {
    this.ensureInitialized();

    const response = await this.request<{ indexes: Array<{
      label: string;
      property: string;
      index_type: string;
      cardinality: number;
      memory_bytes: number;
    }>; total: number }>(
      'GET',
      `/collections/${encodeURIComponent(collection)}/indexes`
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return (response.data?.indexes ?? []).map(idx => ({
      label: idx.label,
      property: idx.property,
      indexType: idx.index_type as 'hash' | 'range',
      cardinality: idx.cardinality,
      memoryBytes: idx.memory_bytes,
    }));
  }

  async hasIndex(collection: string, label: string, property: string): Promise<boolean> {
    const indexes = await this.listIndexes(collection);
    return indexes.some(idx => idx.label === label && idx.property === property);
  }

  async dropIndex(collection: string, label: string, property: string): Promise<boolean> {
    this.ensureInitialized();

    const response = await this.request<{ dropped: boolean }>(
      'DELETE',
      `/collections/${encodeURIComponent(collection)}/indexes/${encodeURIComponent(label)}/${encodeURIComponent(property)}`
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        return false;  // Index didn't exist
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    // BUG-2 FIX: Success without error = index was dropped
    // API may return 200/204 without body, so default to true on success
    return response.data?.dropped ?? true;
  }

  // ========================================================================
  // Knowledge Graph (EPIC-016 US-041)
  // ========================================================================

  async addEdge(collection: string, edge: AddEdgeRequest): Promise<void> {
    this.ensureInitialized();

    const response = await this.request(
      'POST',
      `/collections/${encodeURIComponent(collection)}/graph/edges`,
      {
        id: edge.id,
        source: edge.source,
        target: edge.target,
        label: edge.label,
        properties: edge.properties ?? {},
      }
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }
  }

  async getEdges(collection: string, options?: GetEdgesOptions): Promise<GraphEdge[]> {
    this.ensureInitialized();

    const queryParams = options?.label ? `?label=${encodeURIComponent(options.label)}` : '';

    const response = await this.request<{ edges: GraphEdge[]; count: number }>(
      'GET',
      `/collections/${encodeURIComponent(collection)}/graph/edges${queryParams}`
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return response.data?.edges ?? [];
  }

  // ========================================================================
  // Graph Traversal (EPIC-016 US-050)
  // ========================================================================

  async traverseGraph(collection: string, request: TraverseRequest): Promise<TraverseResponse> {
    this.ensureInitialized();

    const response = await this.request<{
      results: Array<{ target_id: number; depth: number; path: number[] }>;
      next_cursor: string | null;
      has_more: boolean;
      stats: { visited: number; depth_reached: number };
    }>(
      'POST',
      `/collections/${encodeURIComponent(collection)}/graph/traverse`,
      {
        source: request.source,
        strategy: request.strategy ?? 'bfs',
        max_depth: request.maxDepth ?? 3,
        limit: request.limit ?? 100,
        cursor: request.cursor,
        rel_types: request.relTypes ?? [],
      }
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    const data = response.data;
    return {
      results: (data?.results ?? []).map(r => ({
        targetId: r.target_id,
        depth: r.depth,
        path: r.path,
      })),
      nextCursor: data?.next_cursor ?? undefined,
      hasMore: data?.has_more ?? false,
      stats: {
        visited: data?.stats?.visited ?? 0,
        depthReached: data?.stats?.depth_reached ?? 0,
      },
    };
  }

  async getNodeDegree(collection: string, nodeId: number): Promise<DegreeResponse> {
    this.ensureInitialized();

    const response = await this.request<{ in_degree: number; out_degree: number }>(
      'GET',
      `/collections/${encodeURIComponent(collection)}/graph/nodes/${nodeId}/degree`
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        throw new NotFoundError(`Collection '${collection}'`);
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return {
      inDegree: response.data?.in_degree ?? 0,
      outDegree: response.data?.out_degree ?? 0,
    };
  }
}
