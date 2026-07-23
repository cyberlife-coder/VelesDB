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

/**
 * Metadata shape every `remember`-d fact carries, extended by whatever
 * caller-supplied fields were passed. `_veles_date` is a RESERVED key:
 * `remember` auto-stamps it with today's date (a `YYYYMMDD` integer, e.g.
 * `20260723`) unless the caller already set it — see `velesdb-memory`'s
 * README "Automatic dating (`_veles_date`)" section, and this SDK's own
 * README. Pass `"_veles_date"` as {@link MemoryService.recallFusedDated}'s
 * `dateField` to get a chronological `datedContext` timeline with zero
 * setup; set `_veles_date` explicitly in `remember`'s `metadata` only to
 * override the auto-stamp (e.g. dating a fact by when it actually
 * happened, not when it was stored).
 */
export interface MemoryMetadata extends Record<string, unknown> {
  /** `YYYYMMDD` integer (e.g. `20260723`) — auto-stamped by `remember` unless already set. */
  _veles_date?: number;
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
  metadata?: MemoryMetadata;
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
 * Input of {@link MemoryService.compileTranscript} — the same fields as the
 * MCP `compile_transcript` tool's request MINUS `path`: the wedge runs
 * entirely in-memory with no filesystem, so only an inline `transcript` is
 * accepted (there is nothing for a `path` to resolve against).
 */
export interface CompileTranscriptRequest {
  /** What the agent is working on — drives relevance scoring, like {@link CompileContextRequest.query}. */
  query: string;
  /**
   * The raw transcript text: plain (marker-based —
   * `System:`/`User:`/`Human:`/`Assistant:`/`AI:`/`Tool:`/`### User`/`### Assistant`)
   * or JSONL (one `{role, content}` object per line).
   */
  transcript: string;
  /** Hard budget for the compiled context, in estimated tokens. */
  token_budget: number;
  /** Project facet for savings aggregation. */
  project?: string;
  /** Target model name, for cost insights. */
  target_model?: string;
  /** Compile policy overrides (importance weights, pricing, …). */
  policy?: Record<string, unknown>;
  /**
   * Tuning knobs for the segmentation step itself (format, merge threshold,
   * system-turn caching) — omitted uses the engine's documented defaults
   * (auto-detect format, 256-byte merge threshold, cache the system turn).
   */
  segmentation?: {
    /** Force `"plain"` or `"jsonl"` instead of auto-detecting. */
    format?: 'auto' | 'plain' | 'jsonl';
    /** Segments under this many bytes merge into an adjacent same-kind segment. */
    min_segment_bytes?: number;
    /** Tag the first turn cache-eligible when it looks like a system prompt. */
    cache_system_turn?: boolean;
  };
  [key: string]: unknown;
}

/** One entry of {@link TranscriptSegmentationReport.segments} — the audit trail of how a transcript was cut. */
export interface TranscriptSegmentInfo {
  /** Position of this segment in the segmentation, in transcript order. */
  index: number;
  /** Which turn (0-based) this segment belongs to. */
  turn: number;
  /** The turn's role, when one was determined; absent for a `plain` transcript with no matching marker. */
  role?: string;
  /** `"body"`, `"code"`, or `"log"`. */
  kind: 'body' | 'code' | 'log';
  /** Start byte offset (inclusive) in the original transcript. */
  byte_start: number;
  /** End byte offset (exclusive) in the original transcript. */
  byte_end: number;
  /** The decimal-string id this segment's fragment carries into `context.decisions`. */
  fragment_id: string;
}

/** How {@link MemoryService.compileTranscript} cut the transcript into fragments before compiling. */
export interface TranscriptSegmentationReport {
  /** `"plain"` or `"jsonl"` — the format actually used, never `"auto"` even when requested. */
  format_detected: 'plain' | 'jsonl';
  /** Every segment, in transcript order. */
  segments: TranscriptSegmentInfo[];
  /** How many segments the merge step eliminated. */
  merged_segments: number;
}

/** Output of {@link MemoryService.compileTranscript}. */
export interface CompileTranscriptResult {
  /** The compiled context — byte-compatible with {@link MemoryService.compileContext}'s own output. */
  context: CompiledContext;
  /** How the transcript was cut into fragments before compilation. */
  segmentation: TranscriptSegmentationReport;
}

/**
 * One decision of a {@link MemoryService.compileContext} /
 * {@link MemoryService.compileTranscript} request, as returned by
 * {@link MemoryService.explainCompilation}: why one fragment was preserved,
 * abstracted, externalized, dropped, or cached.
 */
export interface ContextDecision {
  /** The fragment this decision is about (decimal-string id). */
  fragment_id: string;
  /** Content hash of the original fragment text (decimal-string id). */
  content_hash: string;
  /** What was done: `"preserve"`, `"abstract"`, `"externalize"`, `"drop"`, or `"cache"`. */
  action: string;
  /** The stable id of the rule that decided (e.g. `"preserve.code_fence"`). */
  rule_id: string;
  /** Lexical relevance of the fragment to the request query, in `[0, 1]`. */
  relevance: number;
  /** Fidelity risk this single decision contributes. */
  risk: 'low' | 'medium' | 'high';
  /** Human-readable explanation of the decision. */
  reason: string;
  /** The backing memory's decimal-string id, present only for a memory-scope-pulled fragment. */
  memory_id?: string;
  /** A `ctx://source/<hash>` retrieval handle, present only for an externalized/partially-packed fragment. */
  handle?: string;
  [key: string]: unknown;
}

/**
 * Output of {@link MemoryService.contextSavings}: aggregated token (and
 * cost) savings of past {@link MemoryService.compileContext} /
 * {@link MemoryService.compileTranscript} calls.
 */
export interface ContextSavings {
  /** Number of compilation events aggregated. */
  events: number;
  /** Sum of estimated input tokens across events. */
  tokens_in: number;
  /** Sum of estimated output tokens across events. */
  tokens_out: number;
  /** Sum of estimated tokens saved across events. */
  tokens_saved: number;
  /** Estimated cost avoided, in micro-units, keyed by currency. */
  cost_saved_micros_by_currency: Record<string, number>;
  /** `true` when the aggregation hit the recall cap — older events beyond it were not folded in. */
  truncated: boolean;
  [key: string]: unknown;
}

/** Output of {@link MemoryService.suggestBudget}. */
export interface SuggestedBudget {
  /** The model's context window, in tokens — `null` when the model is not in the static table. */
  window: number | null;
  /** `window - reserveTokens` (saturating at 0) — `null` when `window` is `null`. */
  suggested_budget: number | null;
  /** Provenance of the static table, dated — never "measured" or "fetched". */
  source: string;
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

/**
 * The distilled working state of an agent session, persisted and reloaded
 * via {@link MemoryService.saveWorkingContext} /
 * {@link MemoryService.loadWorkingContext} (#1517). Same wire shape as the
 * Node binding's `WorkingContext` (snake_case keys); nested fact/decision
 * shapes are kept as `unknown` here, matching {@link CompiledContext}'s own
 * convention for wire-shaped sub-objects this SDK does not otherwise need
 * to inspect.
 */
export interface WorkingContext {
  /** What the session is trying to achieve. */
  goal?: string;
  /** Constraints currently in force (never compressed away). */
  active_constraints?: unknown[];
  /** Facts that were verified, with their sources. */
  verified_facts?: unknown[];
  /** Hypotheses still open. */
  open_hypotheses?: unknown[];
  /** Decisions taken so far (`{fragment_id, rule_id}`, `fragment_id` a decimal string). */
  decisions?: unknown[];
  /** Exact evidence the session relies on (verbatim, addressable). */
  exact_evidence?: unknown[];
  /** Actions still to do. */
  pending_actions?: string[];
  [key: string]: unknown;
}

/** One session recorded in a project's working-context index (output of {@link MemoryService.listWorkingContexts}). */
export interface WorkingContextSession {
  /** The session id, as passed to {@link MemoryService.saveWorkingContext}. */
  session: string;
  /** Unix seconds this session was last saved. */
  saved_at: number;
}

/** Result of {@link MemoryService.listWorkingContexts}. */
export interface ListWorkingContextsResult {
  /** Every session saved under this project, most-recently-saved first. */
  sessions: WorkingContextSession[];
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
  compileTranscript(request: unknown): unknown;
  explainCompilation(request: unknown, fragmentId: string, fragmentIndex?: number | null): unknown;
  contextSavings(project?: string | null): unknown;
  suggestBudget(targetModel: string, reserveTokens?: bigint | null): unknown;
  retrieveContextSource(handle: string): unknown;
  saveWorkingContext(project: string, session: string, working: unknown): string;
  loadWorkingContext(project: string, session: string): unknown;
  listWorkingContexts(project: string): unknown;
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
 * Two methods available on the Node (`@wiscale/velesdb-memory-node`) and
 * Python bindings are deliberately absent here (issue #1547's audit):
 *
 * - `feedback` (RL Memory re-ranking): the underlying
 *   `MemoryService::feedback` lives behind `velesdb-memory`'s
 *   `persistence` feature, which the WASM build does not enable — a
 *   durable learned confidence is meaningless for a store that disappears
 *   on page reload (see `crates/velesdb-wasm/src/memory_service.rs`'s
 *   module doc). Not a missing binding; an intentional boundary.
 * - `rememberExtracted`: it needs a generative model (the only
 *   `Extractor` implementation in `velesdb-memory` calls out to Ollama),
 *   which would pull a network dependency into the WASM bundle by
 *   default. A JS-provided extractor callback is a natural v2 addition.
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
    // Capability floor, checked at runtime because a stale lockfile can
    // resolve a wasm build older than the declared range (^3.8.0, the
    // floor this SDK's full memory surface — media fragments,
    // retrieveContextSource — needs; the wedge itself first shipped in
    // 3.6.0). Fail with the actionable cause, not a generic load error.
    if (typeof mod.MemoryService !== 'function') {
      throw new ConnectionError(
        'The resolved @wiscale/velesdb-wasm build does not ship MemoryService — ' +
          'the memory wedge requires @wiscale/velesdb-wasm >= 3.8.0 ' +
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
   * Runtime capability guard for a `WasmMemoryService` method that shipped
   * AFTER the class's own base floor (`runInit()`'s `MemoryService`
   * presence check, >= 3.8.0): `compileTranscript`, `explainCompilation`,
   * `contextSavings`, and `suggestBudget` need a @wiscale/velesdb-wasm
   * release newer than 3.12.0. A resolved build that has the `MemoryService`
   * class but not yet this specific method would otherwise fail with a raw,
   * unhelpful `TypeError: x is not a function` from deep inside
   * `wrapWasmCall` — this throws with the same actionable-cause contract
   * `runInit()`'s own capability check uses instead, so every version-floor
   * failure in this file reads the same way regardless of which method hit
   * it.
   */
  private ensureCapability(method: keyof WasmMemoryServiceInstance): WasmMemoryServiceInstance {
    const svc = this.ensureInitialized();
    if (typeof svc[method] !== 'function') {
      throw new ConnectionError(
        `The resolved @wiscale/velesdb-wasm build does not implement ${method}() — ` +
          'this method needs a @wiscale/velesdb-wasm release newer than 3.12.0 ' +
          '(the compileTranscript/explainCompilation/contextSavings/suggestBudget ' +
          'surface ships in the next @wiscale/velesdb-wasm release after 3.12.0; ' +
          'update the dependency once it is available)'
      );
    }
    return svc;
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
      metadata?: MemoryMetadata;
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
   *
   * Pass `"_veles_date"` as `dateField` for zero-setup dating: {@link remember}
   * auto-stamps every fact's metadata with {@link MemoryMetadata._veles_date} —
   * today's date, as a `YYYYMMDD` integer — unless the caller already set it,
   * so this works without pre-tagging facts yourself.
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
   * One-call shortcut over {@link compileContext} for a raw agent-session
   * transcript: deterministically segments it into turns (plain
   * marker-based — `System:`/`User:`/`Human:`/`Assistant:`/`AI:`/`Tool:`/
   * `### User`/`### Assistant` — or JSONL, one line per turn) and, within
   * each turn, into code/log/body sub-segments (fenced code blocks stay
   * atomic; runs of 8+ log-like lines collapse), then compiles the result
   * exactly like {@link compileContext}. Only an inline `transcript` is
   * accepted — the wedge has no filesystem, so there is no `path` variant.
   * Resolves to `{ context, segmentation }`: `context` is byte-compatible
   * with {@link compileContext}'s own output; `segmentation` is the
   * detected format plus one audit entry (turn, role, kind, byte range,
   * `fragment_id` — a decimal string) per segment.
   *
   * In-memory semantics: same as {@link compileContext} — externalized
   * sources and savings events live only in this session's store.
   */
  compileTranscript(request: CompileTranscriptRequest): Promise<CompileTranscriptResult> {
    return wrapWasmCall(
      () =>
        this.ensureCapability('compileTranscript').compileTranscript(
          request
        ) as CompileTranscriptResult
    );
  }

  /**
   * Explain why one fragment of a {@link compileContext} /
   * {@link compileTranscript} request was preserved, abstracted,
   * externalized, dropped, or cached. Compilation is deterministic, so
   * `request` is re-compiled (event/source recording forced off) and the
   * matching decision is returned — no server-side state needed.
   * `fragmentIndex` (0-based position in `request.fragments`), when given,
   * TAKES PRIORITY over `fragmentId` for locating the decision: a shared
   * content-addressed id (byte-identical fragments) otherwise always
   * resolves to the deduplication survivor's decision.
   */
  explainCompilation(
    request: CompileContextRequest,
    fragmentId: string,
    fragmentIndex?: number
  ): Promise<ContextDecision> {
    return wrapWasmCall(
      () =>
        this.ensureCapability('explainCompilation').explainCompilation(
          request,
          fragmentId,
          fragmentIndex
        ) as ContextDecision
    );
  }

  /**
   * Aggregate the token (and cost) savings of past {@link compileContext} /
   * {@link compileTranscript} calls, optionally narrowed to one `project`.
   *
   * In-memory semantics: same as {@link compileContext} — the aggregated
   * events live only in this session's store.
   */
  contextSavings(project?: string): Promise<ContextSavings> {
    return wrapWasmCall(
      () => this.ensureCapability('contextSavings').contextSavings(project) as ContextSavings
    );
  }

  /**
   * Suggest a starting `token_budget` for {@link compileContext} /
   * {@link compileTranscript}, for a named target model — looked up in a
   * static, committed model-name to context-window table (dated "as of",
   * NEVER a network call). Pass `reserveTokens` (default 0) to reserve room
   * for the response. `window`/`suggested_budget` come back `null` when the
   * model is not in the table — an honest "unknown", never a guess.
   */
  suggestBudget(targetModel: string, reserveTokens?: number): Promise<SuggestedBudget> {
    return wrapWasmCall(() => {
      const svc = this.ensureCapability('suggestBudget');
      // Same validation as remember()'s ttlSeconds (see that method's
      // comment for the full rationale): BigInt(1.5) throws a raw
      // RangeError, a negative value dies as an opaque wasm-bindgen u64
      // conversion, and a value past MAX_SAFE_INTEGER wraps modulo 2^64 at
      // the wasm boundary — all must surface as ValidationError here too.
      if (
        reserveTokens !== undefined &&
        (!Number.isInteger(reserveTokens) ||
          reserveTokens < 0 ||
          reserveTokens > Number.MAX_SAFE_INTEGER)
      ) {
        throw new ValidationError(
          `reserveTokens must be an integer between 0 and ${Number.MAX_SAFE_INTEGER}, got ${reserveTokens}`
        );
      }
      return svc.suggestBudget(
        targetModel,
        reserveTokens !== undefined ? BigInt(reserveTokens) : undefined
      ) as SuggestedBudget;
    });
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

  /**
   * Persist the agent's distilled working state under `project` + `session`
   * (idempotent upsert: saving again replaces the previous state), for
   * later resumption (#1517, option 2). Same wire shape as the Node
   * binding's `saveWorkingContext`. Resolves to the stored fact id as a
   * decimal string.
   *
   * **In-memory semantics**: like {@link compileContext}, this is backed
   * entirely by this session's in-memory wasm store — there is no
   * filesystem or IndexedDB persistence behind this binding. A "saved"
   * working context disappears the moment this `MemoryService` instance
   * (and the page/worker that created it) is gone. This is useful to carry
   * state between two calls made within the SAME page load (e.g. across two
   * {@link compileContext} calls), not to resume a session after a reload —
   * that would need a real browser-storage backend, which does not exist
   * yet.
   */
  saveWorkingContext(
    project: string,
    session: string,
    working: WorkingContext
  ): Promise<string> {
    return wrapWasmCall(
      () => this.ensureInitialized().saveWorkingContext(project, session, working) as string
    );
  }

  /**
   * The working context previously saved under `project` + `session` —
   * `null` when there is none, the start-of-session mirror of
   * {@link saveWorkingContext} (#1517, option 2).
   *
   * **In-memory semantics**: see {@link saveWorkingContext}'s doc comment —
   * this only ever resolves what THIS session's in-memory store still
   * holds; nothing persists across a page reload.
   */
  loadWorkingContext(project: string, session: string): Promise<WorkingContext | null> {
    return wrapWasmCall(
      () =>
        this.ensureInitialized().loadWorkingContext(project, session) as WorkingContext | null
    );
  }

  /**
   * Every session ever saved under `project`'s working-context index,
   * most-recently-saved first — empty (never an error) when the project
   * never saved anything (#1517, option 2).
   *
   * **In-memory semantics**: see {@link saveWorkingContext}'s doc comment —
   * reflects only what this session's in-memory store currently holds,
   * never a cross-session/browser-restart view.
   */
  listWorkingContexts(project: string): Promise<ListWorkingContextsResult> {
    return wrapWasmCall(
      () => this.ensureInitialized().listWorkingContexts(project) as ListWorkingContextsResult
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
