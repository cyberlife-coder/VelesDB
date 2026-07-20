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

/**
 * An inline media payload on a {@link CompileContextFragment} (US-009).
 * `content` on the fragment stays the caption — often empty for a bare
 * screenshot — while the pixels live here, base64-encoded. The fragment
 * packs atomically (never chunked) and its token cost comes from the
 * image itself (dimensions sniffed from the PNG/JPEG header), not its
 * base64 text.
 */
export interface CompileContextMedia {
  /** Declared MIME type, e.g. `"image/png"` or `"image/jpeg"`. */
  mime: string;
  /** The raw media bytes, base64-encoded (standard alphabet, padded). */
  bytes_b64: string;
}

/** One input fragment of {@link MemoryService.compileContext}. */
export interface CompileContextFragment {
  /** Optional caller id as a decimal string (content-derived when absent). */
  id?: string;
  /** The fragment text (the caption, when {@link media} is set). */
  content: string;
  /** Classification hint (`"code"`, `"log"`, `"screenshot"`, …). */
  kind?: string;
  /** Fragment flags, e.g. `{ verbatim: true }` or `{ cache: true }`. */
  metadata?: Record<string, unknown>;
  /**
   * Inline media payload (US-009). `undefined` keeps every pre-existing
   * request wire-compatible. Fetch it back later — inline or externalized
   * by budget, it makes no difference — through
   * {@link MemoryService.retrieveContextSource} over the resulting
   * `ctx://source/<hash>` handle.
   */
  media?: CompileContextMedia;
}

/**
 * Input of {@link MemoryService.compileContext} — the MCP `compile_context`
 * wire shape (snake_case keys, ids as decimal strings).
 */
export interface CompileContextRequest {
  /** The current task — relevance ordering anchors on it. */
  query: string;
  /** Hard budget for the compiled context, in estimated tokens. */
  token_budget: number;
  /** The fragments to compile. */
  fragments: CompileContextFragment[];
  /** Pull stored memories into the compile (tri-engine recall). */
  memory_scope?: Record<string, unknown>;
  /** Compile policy overrides (importance weights, pricing, …). */
  policy?: Record<string, unknown>;
  /** Project facet for savings aggregation. */
  project?: string;
  [key: string]: unknown;
}

/**
 * Output of {@link MemoryService.compileContext} — the MCP wire shape
 * (snake_case keys; every id field is a decimal string).
 */
export interface CompiledContext {
  /** The assembled context, ready to inject into a prompt. */
  content: string;
  /** Ordered output blocks (cache prefix first). */
  sections: unknown;
  /** One auditable decision per input fragment. */
  decisions: unknown;
  /** One source pointer per distinct fragment. */
  sources: unknown;
  /** Handles of externalized fragments (`ctx://source/…`). */
  retrieval_handles: unknown;
  /** Token/cost savings of this compilation. */
  insights: unknown;
  /** Overall fidelity risk. */
  risk: 'low' | 'medium' | 'high';
  [key: string]: unknown;
}

/**
 * Output of {@link MemoryService.retrieveContextSource} — the exact original
 * content (and media, when the fragment carried one) behind a
 * `ctx://source/<hash>` handle from a {@link MemoryService.compileContext}
 * result. Same wire shape as the Node binding's own `retrieveContextSource`.
 */
export interface ContextSource {
  /** The handle this source was resolved from (echoed back). */
  handle: string;
  /** The exact original fragment text. */
  content: string;
  /** Present only when the fragment carried an inline media payload. */
  media?: CompileContextMedia;
  [key: string]: unknown;
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

/** Result of {@link MemoryService.recallFusedDated}: the recalled memories plus a dated timeline. */
export interface MemoryDatedRecall {
  /** Recalled memories, most relevant first. */
  memories: MemoryRecollection[];
  /**
   * Chronological, date-prefixed rendering of {@link memories}
   * (`- [YYYY-MM-DD] content` per line, oldest first, undated facts last).
   */
  datedContext: string;
  /** The most recent date across {@link memories} (`YYYY-MM-DD`), or `null` when none is dated. */
  now: string | null;
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
  recallFusedDated(
    query: string,
    dateField: string,
    k: number | null | undefined,
    filter: unknown,
    opts: unknown
  ): unknown;
  relate(from: string, to: string, relation: string): string;
  forget(id: string): boolean;
  why(decision: string, maxHops: number | null | undefined, filter: unknown): unknown;
  compileContext(request: unknown): unknown;
  retrieveContextSource(handle: string): unknown;
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
    let mod: MemoryWasmModule;
    try {
      mod = (await import('@wiscale/velesdb-wasm')) as unknown as MemoryWasmModule;
    } catch (error) {
      throw new ConnectionError(
        'Failed to load @wiscale/velesdb-wasm',
        error instanceof Error ? error : undefined
      );
    }
    // Capability floor, checked at runtime because the declared dependency
    // range (^3.0.0) admits older builds: the memory wedge shipped in wasm
    // 3.6.0, and an existing project's lockfile may still resolve an
    // earlier version. Fail with the actionable cause, not a generic
    // load error.
    if (typeof mod.MemoryService !== 'function') {
      throw new ConnectionError(
        'The resolved @wiscale/velesdb-wasm build does not ship MemoryService — ' +
          'the memory wedge requires @wiscale/velesdb-wasm >= 3.6.0 ' +
          '(update the dependency in your lockfile)'
      );
    }
    try {
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

  /**
   * Fused recall plus a dated timeline: like {@link recallFused}, but reads each
   * fact's date from the `dateField` metadata key (a `YYYYMMDD` integer) and
   * resolves to `{ memories, datedContext, now }` — the memories, a chronological
   * date-prefixed timeline, and a "now" anchor for temporal reasoning.
   */
  recallFusedDated(
    query: string,
    dateField: string,
    k?: number,
    filter?: Record<string, unknown>,
    opts?: MemoryFusionOptions
  ): Promise<MemoryDatedRecall> {
    return wrapWasmCall(
      () =>
        this.ensureInitialized().recallFusedDated(
          query,
          dateField,
          k,
          filter,
          opts
        ) as MemoryDatedRecall
    );
  }

  /** Create a typed edge `from -> to`. Resolves to the edge's decimal-string id. */
  relate(from: string, to: string, relation: string): Promise<string> {
    return wrapWasmCall(() => this.ensureInitialized().relate(from, to, relation));
  }

  /**
   * Delete a memory by id. Resolves to whether a memory actually existed
   * under that id and was deleted — `false` means nothing was stored there
   * (a stale id or a typo), not a second successful deletion.
   */
  forget(id: string): Promise<boolean> {
    return wrapWasmCall(() => this.ensureInitialized().forget(id));
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

  /**
   * Compile context fragments into a token-budgeted, provenance-audited
   * prompt context — deterministic, no LLM, running the same compiler as the
   * MCP server and the Node binding, in the browser. Request and result use
   * the MCP `compile_context` wire shape; every id field crosses as a
   * decimal string.
   *
   * In-memory semantics: externalized sources and savings events live in
   * this session's store — `ctx://source/` handles resolve only within the
   * current browser session.
   */
  compileContext(request: CompileContextRequest): Promise<CompiledContext> {
    return wrapWasmCall(
      () => this.ensureInitialized().compileContext(request) as CompiledContext
    );
  }

  /**
   * Fetch back the exact original content — and media, when the fragment
   * carried one — behind a `ctx://source/<hash>` handle from a
   * {@link compileContext} result: what was externalized or partially
   * packed is recoverable, not lost. Same wire shape as the Node binding's
   * own `retrieveContextSource`.
   *
   * In-memory semantics: the handle resolves only within this session's
   * store — see {@link compileContext}'s doc comment.
   */
  retrieveContextSource(handle: string): Promise<ContextSource> {
    return wrapWasmCall(
      () => this.ensureInitialized().retrieveContextSource(handle) as ContextSource
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
    // `toTypedError` is total (every property read and coercion inside it
    // is guarded), so this single reject can never itself throw — the
    // every-failure-is-a-rejection contract needs no second safety net.
    return Promise.reject(toTypedError(error));
  }
}

/**
 * Translate a value thrown across the wasm boundary into the SDK's typed
 * hierarchy. **Total by construction**: every property read and string
 * coercion is individually guarded, so this never throws even for exotic
 * values (poisoned getters, revoked proxies, prototype-less objects) —
 * `wrapWasmCall`'s rejection contract depends on that. Degradation is
 * per-property: a readable `.code` still classifies the error as
 * NotFound/Validation even when `.message` is unreadable, and the original
 * error object is passed through verbatim only when its whole inspected
 * surface proved safe — otherwise the caller gets a synthetic error whose
 * own `.code`/`.message` reads can never detonate in their catch handler.
 */
function toTypedError(error: unknown): Error {
  const isError = tryRead(() => error instanceof Error); // a revoked proxy throws on the prototype walk
  const rawMessage = tryRead(() => (error as WasmErrorLike).message);
  const message = salvageMessage(rawMessage);
  if (isError.value !== true) {
    return coerceNonError(error, message);
  }
  const code = tryRead(() => (error as WasmErrorLike).code);
  switch (code.value) {
    // NotFoundError's constructor takes a bare resource name and builds its
    // own "X not found" message — but the wasm error already carries a
    // specific, well-formed message from Rust (e.g. "memory 42 does not
    // exist"). Construct it for `instanceof` narrowing, then overwrite its
    // message with the original rather than wrapping it a second time.
    // ValidationError has no such mangling — its constructor takes a raw
    // message directly. The "(original message unavailable)" fallbacks make
    // a lost message visible without claiming why it was lost (empty,
    // non-string, or a throwing getter alike).
    case 'NOT_FOUND':
      return withMessage(
        new NotFoundError('memory'),
        messageOr(message, 'memory not found (original message unavailable)')
      );
    case 'INVALID_INPUT':
      return new ValidationError(
        messageOr(message, 'invalid input (original message unavailable)')
      );
    default:
      return code.ok && rawMessage.ok
        ? (error as Error)
        : degradedError(message, 'wasm error (message unavailable)');
  }
}

/**
 * The `INTERNAL` translation for a thrown non-Error: its string coercion
 * when that works, else a degraded error salvaging the already-read
 * `.message` (a prototype-less `{code, message}` object has no `toString`,
 * yet its message is the one diagnostic worth keeping).
 */
function coerceNonError(error: unknown, message: string | undefined): VelesDBError {
  const text = tryRead(() => String(error));
  if (typeof text.value === 'string') {
    return new VelesDBError(text.value, 'INTERNAL');
  }
  return degradedError(message, 'non-coercible value thrown across the wasm boundary');
}

/** The salvaged message, or `fallback` when nothing survived. */
function messageOr(message: string | undefined, fallback: string): string {
  return message ?? fallback;
}

/** A synthetic error carrying whatever message survived salvage, else `fallback`. */
function degradedError(message: string | undefined, fallback: string): VelesDBError {
  return new VelesDBError(
    message !== undefined ? `wasm error (translation failed): ${message}` : fallback,
    'INTERNAL'
  );
}

/** A guarded read: captures the result, or the fact that reading threw. */
function tryRead<T>(read: () => T): { ok: boolean; value: T | undefined } {
  try {
    return { ok: true, value: read() };
  } catch {
    return { ok: false, value: undefined };
  }
}

/**
 * A usable message text out of a guarded `.message` read, or `undefined`:
 * non-empty strings pass through, non-string values are coerced when their
 * coercion doesn't itself throw; empty, absent, and unreadable all yield
 * nothing (the caller's fallback text stays accurate for every case).
 */
function salvageMessage(read: { ok: boolean; value: unknown }): string | undefined {
  if (!read.ok || read.value == null) {
    return undefined;
  }
  if (typeof read.value === 'string') {
    return read.value.length > 0 ? read.value : undefined;
  }
  const coerced = tryRead(() => String(read.value));
  return coerced.value ? coerced.value : undefined;
}

function withMessage<E extends Error>(error: E, message: string): E {
  Object.defineProperty(error, 'message', { value: message, writable: true, configurable: true });
  return error;
}
