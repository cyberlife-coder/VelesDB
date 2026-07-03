/**
 * VelesDB Memory Wedge — local-first agent memory (WASM-backed).
 *
 * A standalone client, not a facade over {@link IVelesDBBackend}: the wedge
 * is a single in-memory store (no `collection` parameter, no REST
 * counterpart), architecturally distinct from the collection-scoped vector
 * API the rest of the SDK wraps. Mirrors the Node (`@wiscale/velesdb-memory-node`)
 * and Python bindings' own standalone `MemoryService` class rather than
 * bolting onto the generic backend interface — which also sidesteps a real
 * naming collision (`IVelesDBBackend.relate` is the graph-edge API, a
 * different shape than the memory wedge's `relate(from, to, relation)`).
 *
 * @packageDocumentation
 */

import { ConnectionError, NotFoundError, ValidationError, VelesDBError } from './types';

// ---------------------------------------------------------------------------
// Public types — mirror crates/velesdb-node/index.d.ts's Js DTOs
// ---------------------------------------------------------------------------

/** A typed link to an existing memory (input to {@link MemoryService.remember}). */
export interface MemoryLink {
  /** Decimal-string id of the memory being linked to. */
  target: string;
  /** Relationship label, e.g. `"decided_in"`. */
  relation: string;
}

/** One recalled memory (output of `recall` / `recallWhere` / `recallFused`). */
export interface MemoryRecollection {
  /** Decimal-string id of the memory. */
  id: string;
  /** Similarity score (higher is closer). */
  score: number;
  /** Stored fact content. */
  content: string;
  /** Caller-supplied structured metadata, or `undefined` when the fact carries none. */
  metadata?: Record<string, unknown>;
}

/** A structured predicate for {@link MemoryService.recallWhere}. */
export interface MemoryColumnFilter {
  /** Metadata field name (alphanumeric/underscore). */
  field: string;
  /** Comparison operator. */
  op: 'eq' | 'ne' | 'lt' | 'le' | 'gt' | 'ge';
  /** Value to compare against (string, number, or boolean). */
  value: string | number | boolean;
}

/**
 * Tuning knobs for {@link MemoryService.recallFused}. Every field is
 * optional; an omitted field falls back to the proven default (`hops: 2`,
 * `graphBoost: 0.15`, an oversampled pool).
 */
export interface MemoryFusionOptions {
  /** Hops the graph traversal walks from the top vector seed. */
  hops?: number;
  /** Weight added to a graph-reached fact's normalised vector score. */
  graphBoost?: number;
  /** Depth of the oversampled vector pool fusion re-ranks. */
  pool?: number;
}

/** A node in a {@link MemoryService.why} explanation subgraph. */
export interface MemoryNode {
  /** Decimal-string id of the memory. */
  id: string;
  /** Stored fact content. */
  content: string;
  /** Distance in hops from the seed (seed is `0`). */
  hop: number;
}

/** A typed edge in a {@link MemoryService.why} explanation subgraph. */
export interface MemoryEdge {
  /** Source memory id (decimal string). */
  from: string;
  /** Target memory id (decimal string). */
  to: string;
  /** Relationship label. */
  relation: string;
}

/** The connected answer to a {@link MemoryService.why} question. */
export interface MemoryExplanation {
  /** Memories in the subgraph, seed first. */
  nodes: MemoryNode[];
  /** Typed edges connecting the nodes. */
  edges: MemoryEdge[];
}

// ---------------------------------------------------------------------------
// Raw wasm-bindgen surface — just the shape memory.ts needs, kept local
// rather than extending backends/wasm-types.ts (WasmBackend's own typed
// surface): this module does its own independent module load and has no
// other dependency on WasmBackend's collection-oriented types.
// ---------------------------------------------------------------------------

interface WasmMemoryServiceInstance {
  remember(fact: string, links: unknown, metadata: unknown, ttlSeconds?: bigint | null): string;
  recall(query: string, k: number | null | undefined, filter: unknown): unknown;
  recallWhere(query: string, filters: unknown, k?: number | null): unknown;
  recallFused(query: string, k: number | null | undefined, filter: unknown, opts: unknown): unknown;
  relate(from: string, to: string, relation: string): string;
  forget(id: string): void;
  why(decision: string, maxHops: number | null | undefined, filter: unknown): unknown;
  free(): void;
}

interface WasmMemoryServiceConstructor {
  new (dimension: number): WasmMemoryServiceInstance;
}

interface MemoryWasmModule {
  default(moduleOrPath?: Uint8Array | URL | string): Promise<void>;
  MemoryService: WasmMemoryServiceConstructor;
}

/** A JS error thrown across the wasm boundary carries a non-enumerable `.code`. */
interface WasmErrorLike {
  code?: string;
  message?: string;
}

// ---------------------------------------------------------------------------
// MemoryService
// ---------------------------------------------------------------------------

/**
 * Local-first agent memory: remember facts, recall them semantically,
 * relate them, forget them, and ask why a decision was made (a connected
 * subgraph). Runs entirely in-process (browser or Node) via WebAssembly —
 * no server, no network.
 *
 * @example
 * ```typescript
 * const memory = new MemoryService({ dimension: 384 });
 * await memory.init();
 * const id = await memory.remember('we chose parking_lot to avoid lock poisoning');
 * const hits = await memory.recall('lock poisoning');
 * ```
 */
export class MemoryService {
  private readonly dimension: number;
  private inner: WasmMemoryServiceInstance | null = null;
  private _initialized = false;
  // Memoized single-shot init promise — mirrors WasmBackend's own pattern
  // (backends/wasm.ts) so concurrent init() callers await the same
  // in-flight load instead of racing duplicate wasm-bindgen `default()`
  // invocations.
  private _initInFlight: Promise<void> | null = null;

  constructor(options: { dimension?: number } = {}) {
    this.dimension = options.dimension ?? 384;
  }

  /** Load the WASM module and create the underlying in-memory store. */
  init(): Promise<void> {
    if (this._initialized) {
      return Promise.resolve();
    }
    if (!this._initInFlight) {
      this._initInFlight = this.runInit().finally(() => {
        this._initInFlight = null;
      });
    }
    // A distinct derived promise per caller (adopting the shared in-flight
    // load's fate), not the memoized instance itself: one caller's .catch
    // must not mark the rejection handled for every other caller — a
    // fire-and-forget init() still surfaces its own unhandledrejection
    // carrying the WASM-load root cause, exactly as the previous async
    // wrapper guaranteed.
    return this._initInFlight.then();
  }

  private async runInit(): Promise<void> {
    try {
      const mod = (await import('@wiscale/velesdb-wasm')) as unknown as MemoryWasmModule;
      const nodeLoader = await import('./backends/wasm-node-loader');
      if (nodeLoader.isNodeRuntime()) {
        await mod.default(await nodeLoader.loadWasmBytesNode());
      } else {
        await mod.default();
      }
      this.inner = new mod.MemoryService(this.dimension);
      this._initialized = true;
    } catch (error) {
      throw new ConnectionError(
        'Failed to initialize the memory wedge WASM module',
        error instanceof Error ? error : undefined
      );
    }
  }

  isInitialized(): boolean {
    return this._initialized;
  }

  /** Release the underlying WASM store. */
  close(): Promise<void> {
    return wrapWasmCall(() => {
      this.inner?.free();
      this.inner = null;
      this._initialized = false;
    });
  }

  private ensureInitialized(): WasmMemoryServiceInstance {
    if (!this._initialized || !this.inner) {
      throw new ConnectionError('Memory wedge not initialized — call init() first');
    }
    return this.inner;
  }

  /**
   * Store a fact; resolves to its decimal-string id (idempotent on
   * identical content). `links` are edges to existing memories; `metadata`
   * is optional structured data for later filtering; `ttlSeconds` makes the
   * fact expire after that many seconds (omit, or `0`, for permanent).
   */
  remember(
    fact: string,
    options: {
      links?: MemoryLink[];
      metadata?: Record<string, unknown>;
      ttlSeconds?: number;
    } = {}
  ): Promise<string> {
    return wrapWasmCall(() => {
      const svc = this.ensureInitialized();
      const ttl = options.ttlSeconds;
      // Validate before BigInt(): a non-integer throws a raw RangeError, a
      // negative value dies as an opaque wasm-bindgen u64 conversion, and a
      // value past MAX_SAFE_INTEGER silently wraps modulo 2^64 at the wasm
      // boundary (2**64 wraps to 0 — "permanent" — the opposite of what the
      // caller asked). All must surface as the ValidationError this class
      // promises. MAX_SAFE_INTEGER seconds ≈ 285 million years, so the cap
      // rejects only corrupted upstream arithmetic, never a real TTL.
      if (
        ttl !== undefined &&
        (!Number.isInteger(ttl) || ttl < 0 || ttl > Number.MAX_SAFE_INTEGER)
      ) {
        throw new ValidationError(
          `ttlSeconds must be an integer between 0 and ${Number.MAX_SAFE_INTEGER}, got ${ttl}`
        );
      }
      return svc.remember(
        fact,
        options.links ?? [],
        options.metadata,
        ttl !== undefined ? BigInt(ttl) : undefined
      );
    });
  }

  /**
   * Recall up to `k` (default 10) memories similar to `query`, optionally
   * narrowed by an exact-match metadata `filter`.
   */
  recall(
    query: string,
    k?: number,
    filter?: Record<string, unknown>
  ): Promise<MemoryRecollection[]> {
    return wrapWasmCall(
      () => this.ensureInitialized().recall(query, k, filter) as MemoryRecollection[]
    );
  }

  /**
   * Fused vector + `ColumnStore` recall: like {@link recall} but `filters`
   * support ranges/comparisons (`gt`, `le`, …), so temporal/numeric facets
   * become queryable.
   */
  recallWhere(
    query: string,
    filters: MemoryColumnFilter[],
    k?: number
  ): Promise<MemoryRecollection[]> {
    return wrapWasmCall(
      () => this.ensureInitialized().recallWhere(query, filters, k) as MemoryRecollection[]
    );
  }

  /**
   * Fused vector + graph recall: like {@link recall}, but also walks the
   * graph from the top vector hit and promotes any fact it reaches into the
   * ranking — the tri-engine ranking measured on HotpotQA/TimeQA/LoCoMo.
   */
  recallFused(
    query: string,
    k?: number,
    filter?: Record<string, unknown>,
    opts?: MemoryFusionOptions
  ): Promise<MemoryRecollection[]> {
    return wrapWasmCall(
      () => this.ensureInitialized().recallFused(query, k, filter, opts) as MemoryRecollection[]
    );
  }

  /** Create a typed edge `from -> to`. Resolves to the edge's decimal-string id. */
  relate(from: string, to: string, relation: string): Promise<string> {
    return wrapWasmCall(() => this.ensureInitialized().relate(from, to, relation));
  }

  /** Delete a memory by id. */
  forget(id: string): Promise<void> {
    return wrapWasmCall(() => {
      this.ensureInitialized().forget(id);
    });
  }

  /**
   * Explain a decision: the best-matching memory plus its connected
   * subgraph. `maxHops` (default 2) is capped at 10.
   */
  why(
    decision: string,
    maxHops?: number,
    filter?: Record<string, unknown>
  ): Promise<MemoryExplanation> {
    return wrapWasmCall(
      () => this.ensureInitialized().why(decision, maxHops, filter) as MemoryExplanation
    );
  }
}

/**
 * Run a synchronous wasm-bindgen call (every `WasmMemoryService` method is
 * sync — errors surface as a thrown value, not a rejected promise), lift the
 * result into a Promise, and translate a structured `{code}` error into the
 * SDK's typed hierarchy, so callers can
 * `catch (e) { if (e instanceof NotFoundError) ... }` the same way
 * regardless of which backend raised it. Every failure — including a sync
 * throw from validation or the not-initialized guard inside `call` — becomes
 * a rejection, never a synchronous throw, matching what the public
 * Promise-returning signatures advertise.
 */
function wrapWasmCall<T>(call: () => T): Promise<T> {
  try {
    return Promise.resolve(call());
  } catch (error) {
    // toTypedError itself can throw on exotic thrown values (a
    // prototype-less object breaks String(); a poisoned getter breaks the
    // `.code` read). Without an enclosing `async` to lift it, that throw
    // would escape this Promise-returning function SYNCHRONOUSLY — the one
    // path violating the every-failure-is-a-rejection contract — so it too
    // is caught and folded into a rejection.
    try {
      return Promise.reject(toTypedError(error));
    } catch {
      // Prefer the original thrown value when it's a real Error (a poisoned
      // `.code` getter broke the translation, but the message and stack are
      // intact and are what the caller needs) — synthesize a generic error
      // only when even that isn't usable.
      return Promise.reject(
        error instanceof Error
          ? error
          : new VelesDBError('non-coercible value thrown across the wasm boundary', 'INTERNAL')
      );
    }
  }
}

function toTypedError(error: unknown): Error {
  if (!(error instanceof Error)) {
    return new VelesDBError(String(error), 'INTERNAL');
  }
  const code = (error as WasmErrorLike).code;
  switch (code) {
    // NotFoundError's constructor takes a bare resource name and builds its
    // own "X not found" message — but the wasm error already carries a
    // specific, well-formed message from Rust (e.g. "memory 42 does not
    // exist"). Construct it for `instanceof` narrowing, then overwrite its
    // message with the original rather than wrapping it a second time.
    // ValidationError has no such mangling — its constructor takes a raw
    // message directly, so no override is needed there.
    case 'NOT_FOUND':
      return withMessage(new NotFoundError('memory'), error.message);
    case 'INVALID_INPUT':
      return new ValidationError(error.message);
    default:
      return error;
  }
}

function withMessage<E extends Error>(error: E, message: string): E {
  Object.defineProperty(error, 'message', { value: message, writable: true, configurable: true });
  return error;
}
