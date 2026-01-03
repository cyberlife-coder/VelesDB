/**
 * VelesDB TypeScript SDK - Type Definitions
 * @packageDocumentation
 */

/** Supported distance metrics for vector similarity */
export type DistanceMetric = 'cosine' | 'euclidean' | 'dot' | 'hamming' | 'jaccard';

/** Storage mode for vector quantization */
export type StorageMode = 'full' | 'sq8' | 'binary';

/** Backend type for VelesDB connection */
export type BackendType = 'wasm' | 'rest';

/** Configuration options for VelesDB client */
export interface VelesDBConfig {
  /** Backend type: 'wasm' for browser/Node.js, 'rest' for server */
  backend: BackendType;
  /** REST API URL (required for 'rest' backend) */
  url?: string;
  /** API key for authentication (optional) */
  apiKey?: string;
  /** Request timeout in milliseconds (default: 30000) */
  timeout?: number;
}

/** Collection configuration */
export interface CollectionConfig {
  /** Vector dimension (e.g., 768 for BERT, 1536 for GPT) */
  dimension: number;
  /** Distance metric (default: 'cosine') */
  metric?: DistanceMetric;
  /** Storage mode for vector quantization (default: 'full')
   * - 'full': Full f32 precision (3 KB/vector for 768D)
   * - 'sq8': 8-bit scalar quantization, 4x memory reduction (~1% recall loss)
   * - 'binary': 1-bit binary quantization, 32x memory reduction (edge/IoT)
   */
  storageMode?: StorageMode;
  /** Optional collection description */
  description?: string;
}

/** Collection metadata */
export interface Collection {
  /** Collection name */
  name: string;
  /** Vector dimension */
  dimension: number;
  /** Distance metric */
  metric: DistanceMetric;
  /** Storage mode */
  storageMode?: StorageMode;
  /** Number of vectors */
  count: number;
  /** Creation timestamp */
  createdAt?: Date;
}

/** Vector document to insert */
export interface VectorDocument {
  /** Unique identifier */
  id: string | number;
  /** Vector data */
  vector: number[] | Float32Array;
  /** Optional payload/metadata */
  payload?: Record<string, unknown>;
}

/** Search options */
export interface SearchOptions {
  /** Number of results to return (default: 10) */
  k?: number;
  /** Filter expression (optional) */
  filter?: Record<string, unknown>;
  /** Include vectors in results (default: false) */
  includeVectors?: boolean;
}

/** Search result */
export interface SearchResult {
  /** Document ID */
  id: string | number;
  /** Similarity score */
  score: number;
  /** Document payload (if requested) */
  payload?: Record<string, unknown>;
  /** Vector data (if includeVectors is true) */
  vector?: number[];
}

/** Backend interface that all backends must implement */
export interface IVelesDBBackend {
  /** Initialize the backend */
  init(): Promise<void>;
  
  /** Check if backend is initialized */
  isInitialized(): boolean;
  
  /** Create a new collection */
  createCollection(name: string, config: CollectionConfig): Promise<void>;
  
  /** Delete a collection */
  deleteCollection(name: string): Promise<void>;
  
  /** Get collection info */
  getCollection(name: string): Promise<Collection | null>;
  
  /** List all collections */
  listCollections(): Promise<Collection[]>;
  
  /** Insert a single vector */
  insert(collection: string, doc: VectorDocument): Promise<void>;
  
  /** Insert multiple vectors */
  insertBatch(collection: string, docs: VectorDocument[]): Promise<void>;
  
  /** Search for similar vectors */
  search(
    collection: string,
    query: number[] | Float32Array,
    options?: SearchOptions
  ): Promise<SearchResult[]>;
  
  /** Delete a vector by ID */
  delete(collection: string, id: string | number): Promise<boolean>;
  
  /** Get a vector by ID */
  get(collection: string, id: string | number): Promise<VectorDocument | null>;

  /** Search for multiple vectors in batch */
  searchBatch(
    collection: string,
    searches: Array<{
      vector: number[] | Float32Array;
      k?: number;
      filter?: Record<string, unknown>;
    }>
  ): Promise<SearchResult[][]>;
  
  /** Full-text search using BM25 */
  textSearch(
    collection: string,
    query: string,
    options?: { k?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]>;
  
  /** Hybrid search combining vector and text */
  hybridSearch(
    collection: string,
    vector: number[] | Float32Array,
    textQuery: string,
    options?: { k?: number; vectorWeight?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]>;
  
  /** Execute VelesQL query */
  query(
    queryString: string,
    params?: Record<string, unknown>
  ): Promise<SearchResult[]>;
  
  /** Close/cleanup the backend */
  close(): Promise<void>;
}

/** Error types */
export class VelesDBError extends Error {
  constructor(
    message: string,
    public readonly code: string,
    public readonly cause?: Error
  ) {
    super(message);
    this.name = 'VelesDBError';
  }
}

export class ConnectionError extends VelesDBError {
  constructor(message: string, cause?: Error) {
    super(message, 'CONNECTION_ERROR', cause);
    this.name = 'ConnectionError';
  }
}

export class ValidationError extends VelesDBError {
  constructor(message: string) {
    super(message, 'VALIDATION_ERROR');
    this.name = 'ValidationError';
  }
}

export class NotFoundError extends VelesDBError {
  constructor(resource: string) {
    super(`${resource} not found`, 'NOT_FOUND');
    this.name = 'NotFoundError';
  }
}
