/**
 * WASM Backend for VelesDB
 *
 * Uses velesdb-wasm for in-browser/Node.js vector operations.
 * Search/query logic lives in wasm-search.ts; unsupported-feature
 * stubs live in wasm-stubs.ts and wasm-wave4-stubs.ts.
 */

import type {
  IVelesDBBackend,
  CollectionConfig,
  Collection,
  VectorDocument,
  SearchOptions,
  SearchQuality,
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
import type { CapabilityMap } from '../capabilities';
import { WASM_CAPABILITIES } from '../capabilities';
import { ConnectionError, NotFoundError, VelesDBError } from '../types';
import type { WasmModule, CollectionData } from './wasm-types';

// Internal helpers
import {
  toNumericId,
  canonicalPayloadKey,
  buildWasmContext,
  buildCollectionInfo,
} from './wasm-helpers';

// Search & query delegates
import {
  wasmSearch,
  wasmSearchBatch,
  wasmTextSearch,
  wasmHybridSearch,
  wasmMultiQuerySearch,
  wasmQuery,
} from './wasm-search';

// Unsupported-feature stubs (pre-Wave 4)
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
  wasmStreamUpsertPoints,
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

// Wave 4 unsupported stubs
import {
  wasmRebuildIndex,
  wasmGetGuardrails,
  wasmUpdateGuardrails,
  wasmAggregate,
  wasmMatchQuery,
  wasmRemoveEdge,
  wasmGetEdgeCount,
  wasmListNodes,
  wasmGetNodeEdges,
  wasmGetNodePayload,
  wasmUpsertNodePayload,
  wasmGraphSearch,
} from './wasm-wave4-stubs';

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
  // Memoized single-shot init promise. Subsequent concurrent calls to init()
  // await the same in-flight initialization instead of racing into duplicate
  // wasm-bindgen `default()` invocations. Cleared on close() so a fresh
  // backend instance can re-initialize if needed.
  private _initInFlight: Promise<void> | null = null;
  // Generation token bumped by close(). runInit() captures the token at
  // entry and refuses to publish its result if close() advanced the
  // generation while the async work was in flight. Without this, calling
  // close() during an in-flight init() would let the racy completion of
  // runInit() flip _initialized back to true after close() set it false.
  private _initGen = 0;

  // ========================================================================
  // Lifecycle
  // ========================================================================

  async init(): Promise<void> {
    if (this._initialized) { return; }
    if (this._initInFlight) { return this._initInFlight; }
    const gen = this._initGen;
    this._initInFlight = this.runInit(gen).finally(() => {
      // Only clear the in-flight slot if this run is still the active one.
      // close() may have already cleared it and advanced the generation.
      if (this._initGen === gen) {
        this._initInFlight = null;
      }
    });
    return this._initInFlight;
  }

  private async runInit(gen: number): Promise<void> {
    try {
      const mod = (await import('@wiscale/velesdb-wasm')) as unknown as WasmModule;
      // The wasm-pack default init() does `fetch(wasmUrl)` to locate the .wasm
      // binary. That works in the browser but Node's undici has no scheme
      // handler for `file://`, so the import explodes with
      // "not implemented... yet..." (see #379 honesty notes).
      // Detect Node and pass an explicit Buffer so init never relies on fetch.
      if (isNodeRuntime()) {
        await mod.default(await loadWasmBytesNode());
      } else {
        await mod.default();
      }
      // Publish the result only if close() did not advance the generation
      // while the async work above was running. Otherwise the run is stale
      // and must not flip _initialized back to true after close() reset it.
      if (this._initGen !== gen) { return; }
      this.wasmModule = mod;
      this._initialized = true;
    } catch (error) {
      throw new ConnectionError(
        'Failed to initialize WASM module',
        error instanceof Error ? error : undefined
      );
    }
  }

  isInitialized(): boolean { return this._initialized; }

  async close(): Promise<void> {
    for (const [, data] of this.collections) { data.store.free(); }
    this.collections.clear();
    this._initialized = false;
    this._initInFlight = null;
    // Drop the WASM module reference so a future init() picks up a fresh
    // import rather than reusing a possibly-stale handle, and bump the
    // generation so any concurrently in-flight runInit() refuses to
    // publish its result over our cleared state.
    this.wasmModule = null;
    this._initGen += 1;
  }

  capabilities(): Readonly<CapabilityMap> { return WASM_CAPABILITIES; }

  private ensureInitialized(): void {
    if (!this._initialized || !this.wasmModule) {
      throw new ConnectionError('WASM backend not initialized');
    }
  }

  // ========================================================================
  // Collection management
  // ========================================================================

  async createCollection(name: string, config: CollectionConfig): Promise<void> {
    this.ensureInitialized();
    if (this.collections.has(name)) {
      throw new VelesDBError(`Collection '${name}' already exists`, 'COLLECTION_EXISTS');
    }
    const dimension = config.dimension ?? 0;
    const metric = config.metric ?? 'cosine';
    const store = new this.wasmModule!.VectorStore(dimension, metric);
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
    if (!collection) { throw new NotFoundError(`Collection '${name}'`); }
    collection.store.free();
    this.collections.delete(name);
  }

  async getCollection(name: string): Promise<Collection | null> {
    this.ensureInitialized();
    const data = this.collections.get(name);
    return data ? buildCollectionInfo(name, data) : null;
  }

  async listCollections(): Promise<Collection[]> {
    this.ensureInitialized();
    const result: Collection[] = [];
    for (const [name, data] of this.collections) {
      result.push(buildCollectionInfo(name, data));
    }
    return result;
  }

  // ========================================================================
  // Point CRUD
  // ========================================================================

  async upsert(collectionName: string, doc: VectorDocument): Promise<void> {
    this.ensureInitialized();
    const collection = this.collections.get(collectionName);
    if (!collection) { throw new NotFoundError(`Collection '${collectionName}'`); }

    const id = toNumericId(doc.id);
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
      collection.payloads.set(canonicalPayloadKey(doc.id), doc.payload);
    }
  }

  async upsertBatch(collectionName: string, docs: VectorDocument[]): Promise<void> {
    this.ensureInitialized();
    const collection = this.collections.get(collectionName);
    if (!collection) { throw new NotFoundError(`Collection '${collectionName}'`); }

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
      const id = BigInt(toNumericId(doc.id));
      const vector = doc.vector instanceof Float32Array
        ? doc.vector
        : new Float32Array(doc.vector);
      if (doc.payload) {
        collection.store.insert_with_payload(id, vector, doc.payload);
      } else {
        batch.push([id, Array.from(vector)]);
      }
    }
    if (batch.length > 0) { collection.store.insert_batch(batch); }

    for (const doc of docs) {
      if (doc.payload) {
        collection.payloads.set(canonicalPayloadKey(doc.id), doc.payload);
      }
    }
  }

  async delete(collectionName: string, id: string | number): Promise<boolean> {
    this.ensureInitialized();
    const collection = this.collections.get(collectionName);
    if (!collection) { throw new NotFoundError(`Collection '${collectionName}'`); }
    const numericId = toNumericId(id);
    const removed = collection.store.remove(BigInt(numericId));
    if (removed) { collection.payloads.delete(canonicalPayloadKey(id)); }
    return removed;
  }

  async get(collectionName: string, id: string | number): Promise<VectorDocument | null> {
    this.ensureInitialized();
    const collection = this.collections.get(collectionName);
    if (!collection) { throw new NotFoundError(`Collection '${collectionName}'`); }
    const numericId = toNumericId(id);
    const point = collection.store.get(BigInt(numericId));
    if (!point) { return null; }
    const payload = point.payload ?? collection.payloads.get(canonicalPayloadKey(numericId));
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
    if (!collection) { throw new NotFoundError(`Collection '${collectionName}'`); }
    return collection.store.is_empty;
  }

  async flush(collectionName: string): Promise<void> {
    this.ensureInitialized();
    const collection = this.collections.get(collectionName);
    if (!collection) { throw new NotFoundError(`Collection '${collectionName}'`); }
    // WASM runs in-memory, flush is a no-op
  }

  // ========================================================================
  // Search & Query -- delegates to wasm-search.ts
  // ========================================================================

  async search(c: string, q: number[] | Float32Array, o?: SearchOptions): Promise<SearchResult[]> {
    this.ensureInitialized();
    return wasmSearch(buildWasmContext(this.wasmModule!, this.collections), c, q, o);
  }

  async searchBatch(c: string, s: Array<{ vector: number[] | Float32Array; k?: number; filter?: FilterInput; quality?: SearchQuality }>): Promise<SearchResult[][]> {
    this.ensureInitialized();
    return wasmSearchBatch(buildWasmContext(this.wasmModule!, this.collections), c, s);
  }

  async textSearch(c: string, q: string, o?: { k?: number; filter?: FilterInput }): Promise<SearchResult[]> {
    this.ensureInitialized();
    return wasmTextSearch(buildWasmContext(this.wasmModule!, this.collections), c, q, o);
  }

  async hybridSearch(c: string, v: number[] | Float32Array, t: string, o?: { k?: number; vectorWeight?: number; filter?: FilterInput }): Promise<SearchResult[]> {
    this.ensureInitialized();
    return wasmHybridSearch(buildWasmContext(this.wasmModule!, this.collections), c, v, t, o);
  }

  async query(c: string, q: string, p?: Record<string, unknown>, o?: QueryOptions): Promise<QueryApiResponse> {
    this.ensureInitialized();
    return wasmQuery(buildWasmContext(this.wasmModule!, this.collections), c, q, p, o);
  }

  async multiQuerySearch(c: string, v: Array<number[] | Float32Array>, o?: MultiQuerySearchOptions): Promise<SearchResult[]> {
    this.ensureInitialized();
    return wasmMultiQuerySearch(buildWasmContext(this.wasmModule!, this.collections), c, v, o);
  }

  // ========================================================================
  // Stubs -- delegates to wasm-stubs.ts & wasm-wave4-stubs.ts
  // ========================================================================

  async queryExplain(q: string, p?: Record<string, unknown>, o?: { analyze?: boolean }): Promise<ExplainResponse> { this.ensureInitialized(); return wasmQueryExplain(q, p, o); }
  async collectionSanity(c: string): Promise<CollectionSanityResponse> { this.ensureInitialized(); return wasmCollectionSanity(c); }
  async scroll(c: string, r?: ScrollRequest): Promise<ScrollResponse> { this.ensureInitialized(); return wasmScroll(c, r); }
  async createIndex(c: string, o: CreateIndexOptions): Promise<void> { this.ensureInitialized(); return wasmCreateIndex(c, o); }
  async listIndexes(c: string): Promise<IndexInfo[]> { this.ensureInitialized(); return wasmListIndexes(c); }
  async hasIndex(c: string, l: string, p: string): Promise<boolean> { this.ensureInitialized(); return wasmHasIndex(c, l, p); }
  async dropIndex(c: string, l: string, p: string): Promise<boolean> { this.ensureInitialized(); return wasmDropIndex(c, l, p); }
  async addEdge(c: string, e: AddEdgeRequest): Promise<void> { this.ensureInitialized(); return wasmAddEdge(c, e); }
  async getEdges(c: string, o?: GetEdgesOptions): Promise<GraphEdge[]> { this.ensureInitialized(); return wasmGetEdges(c, o); }
  async traverseGraph(c: string, r: TraverseRequest): Promise<TraverseResponse> { this.ensureInitialized(); return wasmTraverseGraph(c, r); }
  async traverseParallel(c: string, r: TraverseParallelRequest): Promise<TraverseResponse> { this.ensureInitialized(); return wasmTraverseParallel(c, r); }
  async getNodeDegree(c: string, n: number): Promise<DegreeResponse> { this.ensureInitialized(); return wasmGetNodeDegree(c, n); }
  async trainPq(c: string, o?: PqTrainOptions): Promise<string> { this.ensureInitialized(); return wasmTrainPq(c, o); }
  async streamInsert(c: string, d: VectorDocument[]): Promise<void> { this.ensureInitialized(); return wasmStreamInsert(c, d); }
  async streamUpsertPoints(c: string, d: VectorDocument[]): Promise<import('../types').StreamUpsertResponse> { this.ensureInitialized(); return wasmStreamUpsertPoints(c, d); }
  async createGraphCollection(n: string, c?: GraphCollectionConfig): Promise<void> { this.ensureInitialized(); return wasmCreateGraphCollection(n, c); }
  async getCollectionStats(c: string): Promise<CollectionStatsResponse | null> { this.ensureInitialized(); return wasmGetCollectionStats(c); }
  async analyzeCollection(c: string): Promise<CollectionStatsResponse> { this.ensureInitialized(); return wasmAnalyzeCollection(c); }
  async getCollectionConfig(c: string): Promise<CollectionConfigResponse> { this.ensureInitialized(); return wasmGetCollectionConfig(c); }
  async searchIds(c: string, q: number[] | Float32Array, o?: SearchOptions): Promise<Array<{ id: number; score: number }>> { this.ensureInitialized(); return wasmSearchIds(c, q, o); }
  async storeSemanticFact(c: string, e: SemanticEntry): Promise<void> { this.ensureInitialized(); return wasmStoreSemanticFact(c, e); }
  async searchSemanticMemory(c: string, e: number[], k?: number): Promise<SearchResult[]> { this.ensureInitialized(); return wasmSearchSemanticMemory(c, e, k); }
  async recordEpisodicEvent(c: string, e: EpisodicEvent): Promise<void> { this.ensureInitialized(); return wasmRecordEpisodicEvent(c, e); }
  async recallEpisodicEvents(c: string, e: number[], k?: number): Promise<SearchResult[]> { this.ensureInitialized(); return wasmRecallEpisodicEvents(c, e, k); }
  async storeProceduralPattern(c: string, p: ProceduralPattern): Promise<void> { this.ensureInitialized(); return wasmStoreProceduralPattern(c, p); }
  async matchProceduralPatterns(c: string, e: number[], k?: number): Promise<SearchResult[]> { this.ensureInitialized(); return wasmMatchProceduralPatterns(c, e, k); }

  // Wave 4 stubs
  async rebuildIndex(c: string): Promise<import('../types').RebuildIndexResponse> { this.ensureInitialized(); return wasmRebuildIndex(c); }
  async getGuardrails(): Promise<import('../types').GuardRailsConfigResponse> { this.ensureInitialized(); return wasmGetGuardrails(); }
  async updateGuardrails(r: import('../types').GuardRailsUpdateRequest): Promise<import('../types').GuardRailsConfigResponse> { this.ensureInitialized(); return wasmUpdateGuardrails(r); }
  async aggregate(_q: string, _p?: Record<string, unknown>, _o?: import('../types').AggregateQueryOptions): Promise<import('../types').AggregateResponse> { this.ensureInitialized(); return wasmAggregate(_q, _p, _o); }
  async matchQuery(c: string, q: string, p?: Record<string, unknown>, o?: import('../types').MatchQueryOptions): Promise<import('../types').MatchQueryResponse> { this.ensureInitialized(); return wasmMatchQuery(c, q, p, o); }
  async removeEdge(c: string, id: number): Promise<boolean> { this.ensureInitialized(); return wasmRemoveEdge(c, id); }
  async getEdgeCount(c: string): Promise<number> { this.ensureInitialized(); return wasmGetEdgeCount(c); }
  async listNodes(c: string): Promise<import('../types').ListNodesResponse> { this.ensureInitialized(); return wasmListNodes(c); }
  async getNodeEdges(c: string, id: number, o?: import('../types').GetNodeEdgesOptions): Promise<GraphEdge[]> { this.ensureInitialized(); return wasmGetNodeEdges(c, id, o); }
  async getNodePayload(c: string, id: number): Promise<import('../types').NodePayloadResponse> { this.ensureInitialized(); return wasmGetNodePayload(c, id); }
  async upsertNodePayload(c: string, id: number, p: Record<string, unknown>): Promise<void> { this.ensureInitialized(); return wasmUpsertNodePayload(c, id, p); }
  async graphSearch(c: string, r: import('../types').GraphSearchRequest): Promise<import('../types').GraphSearchResponse> { this.ensureInitialized(); return wasmGraphSearch(c, r); }
}

/**
 * True iff we're running under a Node.js runtime. Centralized so both the
 * init() branch decision and the bytes-loader use the same signal.
 */
function isNodeRuntime(): boolean {
  return (
    typeof process !== 'undefined' &&
    Boolean((process as { versions?: { node?: string } }).versions?.node)
  );
}

/**
 * Read the wasm binary from disk in Node.js, returning a Uint8Array that
 * the wasm-pack `default()` initializer accepts as a `BufferSource`.
 *
 * Robustness against the bundled-package layout:
 *   - The WASM filename is **discovered** at runtime by reading
 *     `@wiscale/velesdb-wasm/package.json#files` for any `*.wasm` entry, so
 *     this code does not break if wasm-pack ever changes its default
 *     `<crate>_bg.wasm` naming or if `--out-name` is customized.
 *   - `createRequire(import.meta.url)` is the canonical pattern for module
 *     resolution under both ESM and CJS Node consumers. The CJS bundle
 *     produced by tsup rewrites `import.meta.url` to a `__filename`-based
 *     URL automatically, so this single line works under both module
 *     systems without per-format branching.
 *
 * Browser callers never hit this path — `init()` only invokes it when
 * {@link isNodeRuntime} is true.
 */
// Ambient declaration so the TypeScript source can reference Node's CJS
// `__filename` global without dragging in `@types/node`. The runtime check
// `typeof __filename !== 'undefined'` keeps ESM bundles safe — the variable
// only resolves under a CJS module wrapper.
declare const __filename: string | undefined;

async function loadWasmBytesNode(): Promise<Uint8Array> {
  const [{ createRequire }, { readFile, readdir }, path] = await Promise.all([
    import('node:module'),
    import('node:fs/promises'),
    import('node:path'),
  ]);
  // Pick whichever module identifier is actually defined in this runtime:
  //   - CJS: `__filename` is wrapped in by Node's CommonJS module loader.
  //   - ESM: `import.meta.url` is the spec-mandated locator.
  // We prefer `__filename` first because tsup's CJS output leaves
  // `import.meta.url` as `undefined`, which would make `createRequire`
  // throw ERR_INVALID_ARG_VALUE under `require('@wiscale/velesdb-sdk')`.
  const cjsFilename =
    typeof __filename !== 'undefined' ? __filename : undefined;
  const moduleId =
    typeof cjsFilename === 'string' && cjsFilename.length > 0
      ? cjsFilename
      : import.meta.url;
  const require = createRequire(moduleId);
  const pkgJsonPath = require.resolve('@wiscale/velesdb-wasm/package.json');
  const pkgDir = path.dirname(pkgJsonPath);

  // Discover the WASM binary by listing files actually present on disk.
  // We deliberately do NOT inspect package.json#files because that field is
  // an npm publish whitelist, not a general manifest — if the publisher
  // ever switches to .npmignore (or simply forgets to include the wasm in
  // `files`), the manifest-based lookup would fail even though the binary
  // is present in the installed package. Reading the directory matches
  // what wasm-pack actually ships and is resilient to packaging convention
  // changes upstream.
  const entries = await readdir(pkgDir);
  const wasmFile = entries.find((name) => name.endsWith('.wasm'));
  if (!wasmFile) {
    throw new Error(
      `Cannot locate a *.wasm binary in @wiscale/velesdb-wasm at ${pkgDir}. ` +
        'The Node.js path expects wasm-pack output (e.g. velesdb_wasm_bg.wasm) ' +
        'to be present alongside package.json.'
    );
  }
  return readFile(path.join(pkgDir, wasmFile));
}
