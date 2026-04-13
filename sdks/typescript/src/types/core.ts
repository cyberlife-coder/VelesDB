/**
 * VelesDB TypeScript SDK - Core Type Definitions
 *
 * Collection, configuration, and basic document types.
 * @packageDocumentation
 */

/** Supported distance metrics for vector similarity */
export type DistanceMetric = 'cosine' | 'euclidean' | 'dot' | 'hamming' | 'jaccard';

/** Storage mode for vector quantization */
export type StorageMode = 'full' | 'sq8' | 'binary' | 'pq' | 'rabitq';

/** Search quality preset controlling recall vs speed tradeoff. */
export type SearchQuality = 'fast' | 'balanced' | 'accurate' | 'perfect' | 'autotune' | `custom:${number}` | `adaptive:${number}:${number}`;

/** Backend type for VelesDB connection */
export type BackendType = 'wasm' | 'rest';

/** Numeric point ID required by velesdb-server REST API (`u64`). */
export type RestPointId = number;

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

/** Collection type */
export type CollectionType = 'vector' | 'metadata_only' | 'graph';

/** HNSW index parameters for collection creation */
export interface HnswParams {
  /** Number of bi-directional links per node (M parameter) */
  m?: number;
  /** Size of dynamic candidate list during construction */
  efConstruction?: number;
  /** Maximum number of elements in the index */
  maxElements?: number;
  /** Storage mode for vector quantization */
  storageMode?: StorageMode;
  /** Alpha parameter for HNSW construction */
  alpha?: number;
}

/**
 * Deferred indexing configuration (`velesdb_core::collection::streaming::DeferredIndexerConfig`).
 *
 * When enabled, inserts are buffered in memory and batch-merged into the
 * HNSW index once the buffer reaches `mergeThreshold` or once the oldest
 * buffered vector is older than `maxBufferAgeMs`. Trades insert latency
 * for throughput.
 */
export interface DeferredIndexerOptions {
  /** Whether deferred indexing is enabled (default: false). */
  enabled?: boolean;
  /** Number of buffered vectors that triggers a merge into HNSW. */
  mergeThreshold?: number;
  /** Max age (ms) of the oldest buffered vector before a time-based merge. */
  maxBufferAgeMs?: number;
}

/**
 * Async index builder configuration (`velesdb_core::collection::streaming::AsyncIndexBuilderConfig`).
 *
 * Enables the parallel segment-based `AsyncIndexBuilder` for bulk inserts
 * (Issue #488 -- Bulk Insert V2). Used when the collection is known to
 * receive large bulk loads where the extra segment coordination cost is
 * amortised over millions of inserts.
 */
export interface AsyncIndexBuilderOptions {
  /** Buffered vector count that triggers a build (default: 10_000). */
  mergeThreshold?: number;
  /** Number of segments for parallel construction (default: num_cpus). */
  segmentCount?: number;
}

/** Collection configuration */
export interface CollectionConfig {
  /** Vector dimension (e.g., 768 for BERT, 1536 for GPT). Required for vector collections. */
  dimension?: number;
  /** Distance metric (default: 'cosine') */
  metric?: DistanceMetric;
  /** Storage mode for vector quantization (default: 'full')
   * - 'full': Full f32 precision (3 KB/vector for 768D)
   * - 'sq8': 8-bit scalar quantization, 4x memory reduction (~1% recall loss)
   * - 'binary': 1-bit binary quantization, 32x memory reduction (edge/IoT)
   * - 'pq': Product quantization (requires training via `trainPq`)
   * - 'rabitq': RaBitQ quantization (binary + rescoring)
   */
  storageMode?: StorageMode;
  /** Collection type: 'vector' (default) or 'metadata_only' */
  collectionType?: CollectionType;
  /** Optional collection description */
  description?: string;
  /** Optional HNSW parameters for index tuning */
  hnsw?: HnswParams;
  /**
   * PQ rescore oversampling factor (quantised storage modes only).
   *
   * The search pipeline fetches `max(k * factor, k + 32)` candidates from
   * HNSW and rescores them with full-precision ADC. Default is `4`.
   * Setting `0` disables rescoring (fastest, lowest recall).
   */
  pqRescoreOversampling?: number;
  /** Deferred indexing buffer configuration (US-366). */
  deferredIndexing?: DeferredIndexerOptions;
  /** Parallel async index builder configuration (Issue #488). */
  asyncIndexBuilder?: AsyncIndexBuilderOptions;
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

/** Sparse vector: mapping from term/dimension index to weight */
export type SparseVector = Record<number, number>;

/** Vector document to upsert */
export interface VectorDocument {
  /** Unique identifier */
  id: string | number;
  /** Vector data */
  vector: number[] | Float32Array;
  /** Optional payload/metadata */
  payload?: Record<string, unknown>;
  /** Optional sparse vector for hybrid search */
  sparseVector?: SparseVector;
}

/** PQ (Product Quantization) training options */
export interface PqTrainOptions {
  /** Number of subquantizers (default: 8) */
  m?: number;
  /** Number of centroids per subquantizer (default: 256) */
  k?: number;
  /** Enable Optimized Product Quantization (default: false) */
  opq?: boolean;
}
