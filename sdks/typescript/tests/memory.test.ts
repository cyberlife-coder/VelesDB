/**
 * MemoryService Unit Tests
 *
 * Tests the MemoryService class against a mocked `@wiscale/velesdb-wasm`
 * module, mirroring wasm-backend.test.ts's convention (a real-wasm smoke
 * run was used to verify the class against the actual compiled artifact
 * during development; this suite is the permanent, CI-safe one).
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { MemoryService } from '../src/memory';
import type { CompileContextFragment, MemoryMetadata } from '../src/memory';
import { ConnectionError, NotFoundError, ValidationError } from '../src/types';

// Captures the most recently constructed mock instance so a test can
// override one of its methods for that specific instance. Overriding
// `MockWasmMemoryService.prototype.X` would NOT work here: each method is a
// class-field (`remember = vi.fn(...)`), which vitest/TS compiles into a
// per-instance own-property assigned in the constructor — an own-property
// always shadows a same-named prototype property, so a prototype patch
// applied after construction has no effect on an already-built instance.
let lastMockInstance: MockWasmMemoryService | null = null;

class MockWasmMemoryService {
  remember = vi.fn(() => '1');
  recall = vi.fn(() => [{ id: '1', score: 0.9, content: 'we chose parking_lot' }]);
  recallWhere = vi.fn(() => [{ id: '1', score: 0.9, content: 'we chose parking_lot' }]);
  recallFused = vi.fn(() => [{ id: '2', score: 0.0, content: 'EPIC-317' }]);
  recallFusedDated = vi.fn(() => ({
    memories: [{ id: '1', score: 0.9, content: 'we chose parking_lot' }],
    datedContext: '- [2026-01-03] we chose parking_lot',
    now: '2026-01-03',
  }));
  relate = vi.fn(() => '5');
  unrelate = vi.fn(() => ({ found: true, removed: 1 }));
  // The camelCase wire shape the wasm binding actually serves: `relationsIn`
  // beside `targetId`, never a snake_case key next to a camelCase one. A mock
  // spelling it `relations_in` would keep this suite green over a binding it
  // no longer describes.
  entity = vi.fn(() => ({
    found: true,
    id: '12297829382473034410',
    name: 'theo durand',
    attributes: { age: 15 },
    relations: [],
    relationsIn: [
      { predicate: 'sister of', targetId: '43', target: 'Entity: camille durand' },
    ],
    // Asymmetric on purpose: a mock repeating one value for both would keep
    // a SDK that mirrored the two flags green.
    relationsTruncated: true,
    relationsInTruncated: false,
  }));
  rememberExtracted = vi.fn(() => ({ ids: ['1'], skippedOverCap: 0 }));
  forget = vi.fn(() => true);
  why = vi.fn(() => ({
    nodes: [{ id: '1', content: 'we chose parking_lot', hop: 0 }],
    edges: [],
    truncated: false,
  }));
  compileContext = vi.fn(() => ({
    content: 'compiled',
    sections: [],
    decisions: [{ fragment_id: '18446744073709551615', rule_id: 'preserve.default' }],
    sources: [],
    retrieval_handles: [],
    insights: { tokens_saved: 0 },
    risk: 'low',
  }));
  compileTranscript = vi.fn(() => ({
    context: {
      content: 'compiled from transcript',
      sections: [],
      decisions: [{ fragment_id: '18446744073709551615', rule_id: 'preserve.default' }],
      sources: [],
      retrieval_handles: [],
      insights: { tokens_saved: 0 },
      risk: 'low',
    },
    segmentation: {
      format_detected: 'plain',
      segments: [
        {
          index: 0,
          turn: 0,
          role: 'User',
          kind: 'body',
          byte_start: 0,
          byte_end: 12,
          fragment_id: '18446744073709551615',
        },
      ],
      merged_segments: 0,
    },
  }));
  explainCompilation = vi.fn(() => ({
    fragment_id: '18446744073709551615',
    content_hash: '18446744073709551615',
    action: 'preserve',
    rule_id: 'preserve.default',
    relevance: 0.9,
    risk: 'low',
    reason: 'high relevance to query',
  }));
  contextSavings = vi.fn(() => ({
    events: 1,
    tokens_in: 100,
    tokens_out: 0,
    tokens_saved: 20,
    cost_saved_micros_by_currency: {},
    truncated: false,
  }));
  suggestBudget = vi.fn(() => ({
    window: 200000,
    suggested_budget: 199000,
    source: 'static table as of 2026-01-01',
  }));
  retrieveContextSource = vi.fn(() => ({
    handle: 'ctx://source/123',
    content: 'the original fragment text',
  }));
  saveWorkingContext = vi.fn(() => '42');
  // The mock returns the THREE-field envelope the wasm binding now serves,
  // not the bare working context — a mock that keeps the old shape would let
  // this suite stay green over a binding it no longer describes.
  loadWorkingContext = vi.fn(() => ({
    found: true,
    working: {
      goal: 'ship the canary fix',
      decisions: [{ fragment_id: '18446744073709551615', rule_id: 'media.atomic' }],
      pending_actions: ['roll back if error rate spikes'],
    },
    other_sessions: ['session-b'],
  }));
  listWorkingContexts = vi.fn(() => ({
    sessions: [{ session: 'session-a', saved_at: 1731000000 }],
  }));
  free = vi.fn();

  constructor(public dimension: number) {
    lastMockInstance = this;
  }
}

const mockWasmModule = {
  default: vi.fn(() => Promise.resolve()),
  MemoryService: MockWasmMemoryService,
};

// Mock the dynamic import - must match the import path in memory.ts
vi.mock('@wiscale/velesdb-wasm', () => mockWasmModule);

// Stub the Node-only loader so the unit suite does not touch the real
// filesystem (see wasm-backend.test.ts for the same rationale).
vi.mock('../src/backends/wasm-node-loader', () => ({
  isNodeRuntime: () => false,
  loadWasmBytesNode: vi.fn(() => Promise.resolve(new Uint8Array(0))),
}));

describe('MemoryService', () => {
  let memory: MemoryService;

  beforeEach(() => {
    vi.clearAllMocks();
    lastMockInstance = null;
    memory = new MemoryService({ dimension: 4 });
  });

  describe('lifecycle', () => {
    it('initializes successfully', async () => {
      await memory.init();
      expect(memory.isInitialized()).toBe(true);
    });

    it('is idempotent', async () => {
      await memory.init();
      await memory.init();
      expect(memory.isInitialized()).toBe(true);
      expect(mockWasmModule.default).toHaveBeenCalledTimes(1);
    });

    it('coalesces concurrent init() calls into one wasm-bindgen invocation', async () => {
      await Promise.all([memory.init(), memory.init(), memory.init()]);
      expect(mockWasmModule.default).toHaveBeenCalledTimes(1);
      expect(memory.isInitialized()).toBe(true);
    });

    it('supports re-init after close()', async () => {
      await memory.init();
      await memory.close();
      expect(memory.isInitialized()).toBe(false);
      await memory.init();
      expect(memory.isInitialized()).toBe(true);
    });

    it('throws ConnectionError from any wedge method before init()', async () => {
      await expect(memory.recall('query')).rejects.toThrow(ConnectionError);
    });

    it('wraps a wasm-bindgen default() failure in ConnectionError', async () => {
      mockWasmModule.default.mockRejectedValueOnce(new Error('boom'));
      await expect(memory.init()).rejects.toThrow(ConnectionError);
      expect(memory.isInitialized()).toBe(false);
    });

    it('init() names the wasm version floor when the resolved build lacks MemoryService', async () => {
      // Simulates a stale lockfile resolving a @wiscale/velesdb-wasm build
      // older than the ^3.8.0 floor the SDK's full memory surface needs.
      const saved = mockWasmModule.MemoryService;
      (mockWasmModule as { MemoryService?: unknown }).MemoryService = undefined;
      try {
        const stale = new MemoryService();
        const rejection = stale.init();
        await expect(rejection).rejects.toBeInstanceOf(ConnectionError);
        await expect(rejection).rejects.toThrow(/>= 3\.8\.0/);
        // A retry after the failed init runs a fresh load (the memoized
        // in-flight promise is cleared on settle) and must fail the same
        // way — never spuriously resolve with a null inner store.
        await expect(stale.init()).rejects.toThrow(/>= 3\.8\.0/);
        expect(stale.isInitialized()).toBe(false);
      } finally {
        mockWasmModule.MemoryService = saved;
      }
    });
  });

  describe('wedge operations', () => {
    beforeEach(async () => {
      await memory.init();
    });

    it('remember() passes fact/links/metadata/ttl through and returns the id', async () => {
      const id = await memory.remember('we chose parking_lot', {
        links: [{ target: '1', relation: 'decided_in' }],
        metadata: { project: 'veles' },
        ttlSeconds: 60,
      });
      expect(id).toBe('1');
      expect(lastMockInstance!.remember).toHaveBeenCalledWith(
        'we chose parking_lot',
        [{ target: '1', relation: 'decided_in' }],
        { project: 'veles' },
        60n
      );
    });

    it('remember() passes empty links and undefined metadata/ttl when not provided', async () => {
      await memory.remember('a fact');
      expect(lastMockInstance!.remember).toHaveBeenCalledWith('a fact', [], undefined, undefined);
    });

    it.each([1.5, -1, Number.NaN, 2 ** 64, Number.POSITIVE_INFINITY])(
      'remember() rejects ttlSeconds %p with ValidationError, not a raw RangeError',
      async (ttlSeconds) => {
        // Regression: BigInt(1.5) throws a codeless RangeError, a negative
        // value dies as an opaque wasm-bindgen u64 conversion, and 2**64
        // silently wraps to 0 ("permanent") at the wasm boundary — all must
        // surface through the typed-error contract as ValidationError.
        await expect(memory.remember('a fact', { ttlSeconds })).rejects.toBeInstanceOf(
          ValidationError
        );
        expect(lastMockInstance!.remember).not.toHaveBeenCalled();
      }
    );

    it('recall() returns the mocked recollections', async () => {
      const hits = await memory.recall('parking_lot', 5, { project: 'veles' });
      expect(hits).toEqual([{ id: '1', score: 0.9, content: 'we chose parking_lot' }]);
      expect(lastMockInstance!.recall).toHaveBeenCalledWith('parking_lot', 5, { project: 'veles' });
    });

    it('recallWhere() returns the mocked recollections', async () => {
      const hits = await memory.recallWhere('parking_lot', [
        { field: 'project', op: 'eq', value: 'veles' },
      ]);
      expect(hits).toEqual([{ id: '1', score: 0.9, content: 'we chose parking_lot' }]);
    });

    it('recallFused() returns the mocked recollections', async () => {
      const hits = await memory.recallFused('parking_lot', 3, undefined, { hops: 2 });
      expect(hits).toEqual([{ id: '2', score: 0.0, content: 'EPIC-317' }]);
      expect(lastMockInstance!.recallFused).toHaveBeenCalledWith('parking_lot', 3, undefined, {
        hops: 2,
      });
    });

    it('recallFusedDated() returns the memories plus a dated timeline', async () => {
      const res = await memory.recallFusedDated('parking_lot', 'ts', 3, undefined, { hops: 2 });
      expect(res.now).toBe('2026-01-03');
      expect(res.datedContext).toContain('- [2026-01-03] we chose parking_lot');
      expect(res.memories).toHaveLength(1);
      expect(lastMockInstance!.recallFusedDated).toHaveBeenCalledWith(
        'parking_lot',
        'ts',
        3,
        undefined,
        { hops: 2 }
      );
    });

    it('relate() returns the edge id', async () => {
      const edgeId = await memory.relate('1', '2', 'decided_in');
      expect(edgeId).toBe('5');
      expect(lastMockInstance!.relate).toHaveBeenCalledWith('1', '2', 'decided_in');
    });

    it('forget() resolves to whether the id existed', async () => {
      await expect(memory.forget('1')).resolves.toBe(true);
      lastMockInstance!.forget.mockReturnValueOnce(false);
      await expect(memory.forget('999')).resolves.toBe(false);
    });

    // The three below were absent from this SDK for its whole life while the
    // wasm binding exposed them (issue #1721). Nothing caught it because the
    // parity guard's declared perimeter held three Rust crates and no
    // TypeScript surface at all.
    it('unrelate() returns what was actually removed', async () => {
      await expect(memory.unrelate('1', '2', 'decided_in')).resolves.toEqual({
        found: true,
        removed: 1,
      });
      expect(lastMockInstance!.unrelate).toHaveBeenCalledWith('1', '2', 'decided_in');
    });

    it('entity() relays BOTH edge directions under their camelCase names', async () => {
      const profile = await memory.entity('Theo Durand');
      expect(profile.found).toBe(true);
      // The assertion that matters: an edge LEAVING camille is invisible from
      // theo's outgoing list and reachable only here. Reading `relations`
      // alone is the exact question this field exists to answer.
      expect(profile.relationsIn).toEqual([
        { predicate: 'sister of', targetId: '43', target: 'Entity: camille durand' },
      ]);
      expect(profile.relations).toEqual([]);
      // A budget cut must reach the SDK caller: an empty `relations` that was
      // TRUNCATED means "more exist", and reads identically to a genuinely
      // empty one without this flag.
      expect(profile.relationsTruncated).toBe(true);
      expect(profile.relationsInTruncated).toBe(false);
      expect(lastMockInstance!.entity).toHaveBeenCalledWith('Theo Durand');
    });

    it('rememberExtracted() returns the envelope, not a bare id array', async () => {
      await expect(memory.rememberExtracted('edge: Camille | sister of | Theo')).resolves.toEqual({
        ids: ['1'],
        skippedOverCap: 0,
      });
      // `extractor` reaches the binding as `undefined`, which the binding
      // reads as its documented default — the SDK does not second-guess it.
      expect(lastMockInstance!.rememberExtracted).toHaveBeenCalledWith(
        'edge: Camille | sister of | Theo',
        undefined,
        undefined
      );
    });

    it('a wasm build too old to carry a method is named, not left to TypeError', async () => {
      // The capability guard's whole reason for existing: without it this
      // surfaces as `x is not a function` from inside wrapWasmCall.
      delete (lastMockInstance as unknown as Record<string, unknown>).entity;
      await expect(memory.entity('Theo Durand')).rejects.toThrow(/does not implement entity\(\)/);
    });

    it('compileContext() delegates the request and returns the wire shape', async () => {
      const request = {
        query: 'state of the canary deploy',
        token_budget: 500,
        fragments: [{ content: 'The canary is green.' }],
      };
      const compiled = await memory.compileContext(request);
      expect(lastMockInstance!.compileContext).toHaveBeenCalledWith(request);
      expect(compiled.risk).toBe('low');
      expect(compiled.content).toBe('compiled');
      const decisions = compiled.decisions as Array<{ fragment_id: string }>;
      expect(decisions[0].fragment_id).toBe('18446744073709551615');
    });

    it('compileContext() passes a media fragment through untouched (US-009)', async () => {
      // Regression this attrapes: a future refactor of compileContext() that
      // reconstructs the request object field-by-field (instead of a plain
      // passthrough) would silently drop an unlisted key like `media` —
      // this fails the moment that happens, without needing a real wasm
      // build to observe it.
      const request = {
        query: 'a screenshot of the failing build',
        token_budget: 4000,
        fragments: [
          {
            content: 'the failing build, before the fix',
            kind: 'screenshot',
            metadata: { target: 'deploy-status-page' },
            media: { mime: 'image/png', bytes_b64: 'aGVsbG8=' },
          },
        ],
      };
      await memory.compileContext(request);
      expect(lastMockInstance!.compileContext).toHaveBeenCalledWith(request);
      // Type-level check: CompileContextFragment must accept `media` without
      // a cast — this line fails to *compile* (not just run) if the field is
      // ever removed from the interface.
      const fragment: CompileContextFragment = request.fragments[0];
      expect(fragment.media?.bytes_b64).toBe('aGVsbG8=');
    });

    it('compileTranscript() delegates the request and returns context + segmentation', async () => {
      const request = {
        query: 'what broke the deploy',
        transcript: 'User: what broke the deploy?\nAssistant: clippy failed on main.\n',
        token_budget: 5000,
      };
      const result = await memory.compileTranscript(request);
      expect(lastMockInstance!.compileTranscript).toHaveBeenCalledWith(request);
      expect(result.context.content).toBe('compiled from transcript');
      expect(result.segmentation.format_detected).toBe('plain');
      expect(result.segmentation.segments).toHaveLength(1);
      expect(result.segmentation.segments[0].fragment_id).toBe('18446744073709551615');
    });

    it('explainCompilation() delegates request/fragmentId/fragmentIndex and returns the decision', async () => {
      const request = {
        query: 'deploy',
        token_budget: 5000,
        fragments: [{ content: 'a fact' }],
      };
      const decision = await memory.explainCompilation(request, '18446744073709551615', 0);
      expect(lastMockInstance!.explainCompilation).toHaveBeenCalledWith(
        request,
        '18446744073709551615',
        0
      );
      expect(decision.action).toBe('preserve');
      expect(decision.fragment_id).toBe('18446744073709551615');
    });

    it('explainCompilation() omits fragmentIndex when not provided', async () => {
      const request = { query: 'q', token_budget: 1000, fragments: [{ content: 'x' }] };
      await memory.explainCompilation(request, '1');
      expect(lastMockInstance!.explainCompilation).toHaveBeenCalledWith(request, '1', undefined);
    });

    it('contextSavings() delegates the project and returns the aggregate', async () => {
      const savings = await memory.contextSavings('veles');
      expect(lastMockInstance!.contextSavings).toHaveBeenCalledWith('veles');
      expect(savings.events).toBe(1);
      expect(savings.tokens_saved).toBe(20);
      expect(savings.truncated).toBe(false);
    });

    it('contextSavings() works with no project filter', async () => {
      await memory.contextSavings();
      expect(lastMockInstance!.contextSavings).toHaveBeenCalledWith(undefined);
    });

    it('suggestBudget() passes reserveTokens as a BigInt and returns the suggestion', async () => {
      const budget = await memory.suggestBudget('claude-sonnet-4-5', 500);
      expect(lastMockInstance!.suggestBudget).toHaveBeenCalledWith('claude-sonnet-4-5', 500n);
      expect(budget.window).toBe(200000);
      expect(budget.suggested_budget).toBe(199000);
    });

    it('suggestBudget() passes undefined reserveTokens when omitted', async () => {
      await memory.suggestBudget('claude-sonnet-4-5');
      expect(lastMockInstance!.suggestBudget).toHaveBeenCalledWith(
        'claude-sonnet-4-5',
        undefined
      );
    });

    it.each([1.5, -1, Number.NaN, 2 ** 64, Number.POSITIVE_INFINITY])(
      'suggestBudget() rejects reserveTokens %p with ValidationError, not a raw RangeError',
      async (reserveTokens) => {
        await expect(
          memory.suggestBudget('claude-sonnet-4-5', reserveTokens)
        ).rejects.toBeInstanceOf(ValidationError);
        expect(lastMockInstance!.suggestBudget).not.toHaveBeenCalled();
      }
    );

    it.each([
      [
        'compileTranscript',
        () => memory.compileTranscript({ query: 'q', transcript: 't', token_budget: 100 }),
      ],
      [
        'explainCompilation',
        () =>
          memory.explainCompilation(
            { query: 'q', token_budget: 100, fragments: [{ content: 'x' }] },
            '1'
          ),
      ],
      ['contextSavings', () => memory.contextSavings()],
      ['suggestBudget', () => memory.suggestBudget('claude-sonnet-4-5')],
    ] as const)(
      '%s() throws an actionable ConnectionError when the resolved wasm build lacks it',
      async (method, call) => {
        // Simulates a resolved @wiscale/velesdb-wasm build that HAS the
        // MemoryService class (so init() already succeeded, per the outer
        // beforeEach) but predates this specific method — e.g. a lockfile
        // pinned between the 3.8.0 base floor and whichever release this
        // method first ships in. Without ensureCapability's guard this call
        // would fail with a raw, unhelpful `TypeError: x is not a function`
        // from deep inside wrapWasmCall instead of a typed, actionable error.
        delete (lastMockInstance as unknown as Record<string, unknown>)[method];
        await expect(call()).rejects.toSatisfy((e: unknown) => {
          expect(e).toBeInstanceOf(ConnectionError);
          expect((e as Error).message).toContain(method);
          expect((e as Error).message).toMatch(/3\.12\.0/);
          return true;
        });
      }
    );

    it('MemoryMetadata types _veles_date as an optional YYYYMMDD number', () => {
      // Type-level check: this must accept `_veles_date` without a cast —
      // fails to *compile* (not just run) if the field is ever removed.
      const metadata: MemoryMetadata = { _veles_date: 20260723, project: 'veles' };
      expect(metadata._veles_date).toBe(20260723);
    });

    it('retrieveContextSource() delegates the handle and returns the resolved source', async () => {
      const resolved = await memory.retrieveContextSource('ctx://source/123');
      expect(lastMockInstance!.retrieveContextSource).toHaveBeenCalledWith('ctx://source/123');
      expect(resolved.handle).toBe('ctx://source/123');
      expect(resolved.content).toBe('the original fragment text');
    });

    it('retrieveContextSource() passes a media source through untouched', async () => {
      lastMockInstance!.retrieveContextSource.mockReturnValueOnce({
        handle: 'ctx://source/456',
        content: 'a screenshot caption',
        media: { mime: 'image/png', bytes_b64: 'aGVsbG8=' },
      });
      const resolved = await memory.retrieveContextSource('ctx://source/456');
      expect(resolved.media?.bytes_b64).toBe('aGVsbG8=');
      expect(resolved.media?.mime).toBe('image/png');
    });

    it('saveWorkingContext() delegates project/session/working and returns the fact id', async () => {
      const working = { goal: 'ship the canary fix', pending_actions: ['watch error rate'] };
      const id = await memory.saveWorkingContext('veles', 'session-a', working);
      expect(lastMockInstance!.saveWorkingContext).toHaveBeenCalledWith(
        'veles',
        'session-a',
        working
      );
      expect(id).toBe('42');
    });

    it('loadWorkingContext() returns the envelope, decimal ids intact under working', async () => {
      const loaded = await memory.loadWorkingContext('veles', 'session-a');
      expect(lastMockInstance!.loadWorkingContext).toHaveBeenCalledWith('veles', 'session-a');
      expect(loaded.found).toBe(true);
      expect(loaded.working?.goal).toBe('ship the canary fix');
      const decisions = loaded.working?.decisions as Array<{ fragment_id: string }>;
      expect(decisions[0].fragment_id).toBe('18446744073709551615');
    });

    it('loadWorkingContext() surfaces other_sessions on a hit, so a typo is detectable', async () => {
      // Populated on a HIT too: a typo landing on another REAL session
      // returns found:true, the case a caller can least detect on its own.
      const loaded = await memory.loadWorkingContext('veles', 'session-a');
      expect(loaded.other_sessions).toEqual(['session-b']);
    });

    it('loadWorkingContext() reports found:false with a null working when nothing was saved', async () => {
      lastMockInstance!.loadWorkingContext.mockReturnValueOnce({
        found: false,
        working: null,
        other_sessions: ['task-1234'],
      });
      const loaded = await memory.loadWorkingContext('veles', 'never-saved-session');
      expect(loaded.found).toBe(false);
      expect(loaded.working).toBeNull();
      // A miss and a TYPO are indistinguishable without this — the whole
      // reason the bare `WorkingContext | null` return was replaced.
      expect(loaded.other_sessions).toEqual(['task-1234']);
    });

    // The SDK's dependency floor on `@wiscale/velesdb-wasm` admits published
    // builds that predate the envelope. On those, `loadWorkingContext` EXISTS
    // — so `ensureCapability`, which only checks presence, passes — and hands
    // back the bare working context. A blind `as LoadedWorkingContext` cast
    // would then give the caller `found: undefined`, falsy, and the agent
    // starts over on top of work that was sitting right there: precisely the
    // silent loss this envelope exists to prevent, reintroduced by a version
    // skew rather than by a missing field. Presence is not shape.
    it('loadWorkingContext() rejects when the resolved wasm build returns the BARE working context', async () => {
      const bare = { goal: 'ship the canary fix', decisions: [] };
      lastMockInstance!.loadWorkingContext.mockReturnValueOnce(bare);
      await expect(memory.loadWorkingContext('veles', 'session-a')).rejects.toThrow(
        ConnectionError
      );
      // The message must name the CAUSE — a wasm build older than this SDK —
      // because the caller's only remedy is to upgrade that dependency.
      lastMockInstance!.loadWorkingContext.mockReturnValueOnce(bare);
      await expect(memory.loadWorkingContext('veles', 'session-a')).rejects.toThrow(
        /predates the .*envelope/
      );
    });

    it('loadWorkingContext() rejects when the resolved wasm build returns bare null', async () => {
      // The pre-envelope MISS: `null`, which a cast turns into a crash on the
      // caller's first property read instead of an actionable error.
      lastMockInstance!.loadWorkingContext.mockReturnValueOnce(null);
      await expect(memory.loadWorkingContext('veles', 'never-saved-session')).rejects.toThrow(
        ConnectionError
      );
    });

    it('listWorkingContexts() delegates the project and returns the sessions', async () => {
      const result = await memory.listWorkingContexts('veles');
      expect(lastMockInstance!.listWorkingContexts).toHaveBeenCalledWith('veles');
      expect(result.sessions).toHaveLength(1);
      expect(result.sessions[0].session).toBe('session-a');
    });

    it('why() returns the explanation subgraph', async () => {
      const explanation = await memory.why('parking_lot', 2);
      expect(explanation.nodes).toHaveLength(1);
      expect(explanation.edges).toEqual([]);
    });

    it('close() frees the underlying wasm instance', async () => {
      const inner = lastMockInstance!;
      await memory.close();
      expect(inner.free).toHaveBeenCalledTimes(1);
    });
  });

  describe('error translation', () => {
    beforeEach(async () => {
      await memory.init();
    });

    it('translates a NOT_FOUND wasm error into NotFoundError, preserving the original message', async () => {
      const err = new Error('memory 999 does not exist');
      (err as Error & { code: string }).code = 'NOT_FOUND';
      lastMockInstance!.relate.mockImplementationOnce(() => {
        throw err;
      });

      await expect(memory.relate('999', '1', 'x')).rejects.toSatisfy((e: unknown) => {
        expect(e).toBeInstanceOf(NotFoundError);
        expect((e as Error).message).toBe('memory 999 does not exist');
        return true;
      });
    });

    it('translates an INVALID_INPUT wasm error into ValidationError', async () => {
      const err = new Error('fact text must not be empty');
      (err as Error & { code: string }).code = 'INVALID_INPUT';
      lastMockInstance!.remember.mockImplementationOnce(() => {
        throw err;
      });

      await expect(memory.remember('')).rejects.toSatisfy((e: unknown) => {
        expect(e).toBeInstanceOf(ValidationError);
        expect((e as Error).message).toBe('fact text must not be empty');
        return true;
      });
    });

    it('rethrows an uncoded error unchanged', async () => {
      const err = new Error('unstructured failure');
      lastMockInstance!.forget.mockImplementationOnce(() => {
        throw err;
      });

      await expect(memory.forget('1')).rejects.toBe(err);
    });

    it('wraps a thrown non-Error value in VelesDBError', async () => {
      lastMockInstance!.forget.mockImplementationOnce(() => {
        // eslint-disable-next-line @typescript-eslint/no-throw-literal
        throw 'a raw string throw';
      });

      await expect(memory.forget('1')).rejects.toThrow('a raw string throw');
    });
  });
});
