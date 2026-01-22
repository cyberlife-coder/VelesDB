/**
 * REST Backend for VelesDB
 * 
 * Connects to VelesDB server via REST API
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
    this.baseUrl = url.replace(/\/$/, ''); // Remove trailing slash
    this.apiKey = apiKey;
    this.timeout = timeout;
  }

  async init(): Promise<void> {
    if (this._initialized) {
      return;
    }

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

    const response = await this.request<Collection[]>('GET', '/collections');

    if (response.error) {
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return response.data ?? [];
  }

  async insert(collection: string, doc: VectorDocument): Promise<void> {
    this.ensureInitialized();

    const vector = doc.vector instanceof Float32Array 
      ? Array.from(doc.vector) 
      : doc.vector;

    const response = await this.request(
      'POST',
      `/collections/${encodeURIComponent(collection)}/vectors`,
      {
        id: doc.id,
        vector,
        payload: doc.payload,
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
      `/collections/${encodeURIComponent(collection)}/vectors/batch`,
      { vectors }
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

    const response = await this.request<SearchResult[]>(
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

    return response.data ?? [];
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
      `/collections/${encodeURIComponent(collection)}/vectors/${encodeURIComponent(String(id))}`
    );

    if (response.error) {
      if (response.error.code === 'NOT_FOUND') {
        return false;
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return response.data?.deleted ?? false;
  }

  async get(collection: string, id: string | number): Promise<VectorDocument | null> {
    this.ensureInitialized();

    const response = await this.request<VectorDocument>(
      'GET',
      `/collections/${encodeURIComponent(collection)}/vectors/${encodeURIComponent(String(id))}`
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

  async query(
    queryString: string,
    params?: Record<string, unknown>
  ): Promise<SearchResult[]> {
    this.ensureInitialized();

    const response = await this.request<{ results: SearchResult[] }>(
      'POST',
      '/query',
      {
        query: queryString,
        params: params ?? {},
      }
    );

    if (response.error) {
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return response.data?.results ?? [];
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
        fusion: options?.fusion ?? 'rrf',
        fusion_params: options?.fusionParams ?? { k: 60 },
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
        return false;
      }
      throw new VelesDBError(response.error.message, response.error.code);
    }

    return response.data?.dropped ?? false;
  }
}
