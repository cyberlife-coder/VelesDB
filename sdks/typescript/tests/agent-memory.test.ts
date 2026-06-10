/**
 * Agent Memory SDK Tests
 *
 * - #1039: storeProceduralPattern MUST send the pattern embedding as the
 *   point vector, otherwise matchProceduralPatterns (a vector search)
 *   can never recall it.
 * - #1047: recordEvent/learnProcedure return the generated point ID; the
 *   facade exposes a delete method; reserved payload keys cannot be
 *   clobbered by caller metadata.
 * - generateUniqueId() must produce unique IDs under rapid-fire calls.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { RestBackend, generateUniqueId, _resetIdState } from '../src/backends/rest';

// Mock global fetch
const mockFetch = vi.fn();
global.fetch = mockFetch;

describe('generateUniqueId', () => {
  afterEach(() => {
    _resetIdState();
    vi.restoreAllMocks();
  });

  it('should produce unique IDs when called rapidly in the same millisecond', () => {
    // Pin Date.now() to a fixed value so every call lands in the same ms
    const fixed = 1700000000000;
    vi.spyOn(Date, 'now').mockReturnValue(fixed);

    const ids = new Set<number>();
    for (let i = 0; i < 100; i++) {
      ids.add(generateUniqueId());
    }

    expect(ids.size).toBe(100);
  });

  it('should reset the counter when the timestamp advances', () => {
    let tick = 1700000000000;
    vi.spyOn(Date, 'now').mockImplementation(() => tick);

    const a = generateUniqueId();

    tick += 1; // advance 1 ms
    const b = generateUniqueId();

    // Both should end with sub-ms counter 0 (different ms buckets)
    expect(a % 1000).toBe(0);
    expect(b % 1000).toBe(0);
    expect(a).not.toBe(b);
  });

  it('should produce 2000 unique IDs when called 2000 times in the same millisecond', () => {
    const fixed = 1700000000000;
    vi.spyOn(Date, 'now').mockReturnValue(fixed);

    const ids = new Set<number>();
    for (let i = 0; i < 2000; i++) {
      ids.add(generateUniqueId());
    }

    expect(ids.size).toBe(2000);
  });

  it('should never exceed Number.MAX_SAFE_INTEGER for realistic timestamps', () => {
    // A timestamp ~year 2100 with 999 sub-ms calls
    const futureMs = 4_102_444_800_000; // 2100-01-01
    vi.spyOn(Date, 'now').mockReturnValue(futureMs);

    for (let i = 0; i < 999; i++) {
      generateUniqueId();
    }
    const id = generateUniqueId();
    expect(id).toBeLessThanOrEqual(Number.MAX_SAFE_INTEGER);
  });
});

describe('Agent Memory REST methods', () => {
  let backend: RestBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    _resetIdState();
    backend = new RestBackend('http://localhost:8080', 'test-key');

    // Init with health check
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ status: 'ok' }),
    });
    await backend.init();
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('storeProceduralPattern (Issue #1039)', () => {
    it('should store the pattern embedding as the point vector', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({}),
      });

      const embedding = [0.1, 0.2, 0.3];
      const id = await backend.storeProceduralPattern('patterns', {
        name: 'deploy',
        steps: ['build', 'test', 'push'],
        embedding,
        metadata: { env: 'prod' },
      });

      expect(mockFetch).toHaveBeenCalledTimes(1);
      const body = JSON.parse(mockFetch.mock.calls[0][1].body);
      const point = body.points[0];

      // vector must be the supplied embedding so vector search can recall it
      expect(point.vector).toEqual(embedding);
      // payload must still be present
      expect(point.payload._memory_type).toBe('procedural');
      expect(point.payload.name).toBe('deploy');
      expect(point.payload.steps).toEqual(['build', 'test', 'push']);
      expect(point.payload.env).toBe('prod');
      // store returns the generated point ID as a string (u64 boundary);
      // the wire id is the numeric form of that same value.
      expect(typeof id).toBe('string');
      expect(String(point.id)).toBe(id);
    });
  });

  describe('reserved payload keys (Issue #1047)', () => {
    it('should not let caller metadata clobber reserved keys', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({}),
      });

      await backend.storeProceduralPattern('patterns', {
        name: 'real-name',
        steps: ['a'],
        embedding: [0.1],
        metadata: { _memory_type: 'hacked', name: 'spoofed', steps: ['evil'] },
      });

      const point = JSON.parse(mockFetch.mock.calls[0][1].body).points[0];
      expect(point.payload._memory_type).toBe('procedural');
      expect(point.payload.name).toBe('real-name');
      expect(point.payload.steps).toEqual(['a']);
    });

    it('should not let caller metadata clobber reserved semantic keys', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({}),
      });

      await backend.storeSemanticFact('facts', {
        id: 1,
        text: 'real-fact',
        embedding: [0.1],
        metadata: { _memory_type: 'hacked', content: 'spoofed' },
      });

      const point = JSON.parse(mockFetch.mock.calls[0][1].body).points[0];
      expect(point.payload._memory_type).toBe('semantic');
      expect(point.payload.content).toBe('real-fact');
    });

    it('should not let caller data/metadata clobber reserved episodic keys', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({}),
      });

      await backend.recordEpisodicEvent('events', {
        eventType: 'real-event',
        embedding: [0.1],
        timestamp: 123,
        data: { _memory_type: 'hacked', event_type: 'spoofed', timestamp: 999 },
        metadata: { _memory_type: 'hacked2', event_type: 'spoofed2' },
      });

      const point = JSON.parse(mockFetch.mock.calls[0][1].body).points[0];
      expect(point.payload._memory_type).toBe('episodic');
      expect(point.payload.event_type).toBe('real-event');
      expect(point.payload.timestamp).toBe(123);
    });
  });

  describe('recordEpisodicEvent (Issue #7)', () => {
    it('should use generateUniqueId instead of Date.now for the point ID', async () => {
      const fixed = 1700000000000;
      vi.spyOn(Date, 'now').mockReturnValue(fixed);

      mockFetch
        .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({}) })
        .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({}) });

      await backend.recordEpisodicEvent('events', {
        eventType: 'click',
        embedding: [0.1, 0.2],
        data: {},
        metadata: {},
      });

      await backend.recordEpisodicEvent('events', {
        eventType: 'scroll',
        embedding: [0.3, 0.4],
        data: {},
        metadata: {},
      });

      const id1 = JSON.parse(mockFetch.mock.calls[0][1].body).points[0].id;
      const id2 = JSON.parse(mockFetch.mock.calls[1][1].body).points[0].id;

      // Same ms, but different IDs
      expect(id1).not.toBe(id2);
    });
  });

  describe('storeProceduralPattern (Issue #7)', () => {
    it('should use generateUniqueId for the point ID', async () => {
      const fixed = 1700000000000;
      vi.spyOn(Date, 'now').mockReturnValue(fixed);

      mockFetch
        .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({}) })
        .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({}) });

      await backend.storeProceduralPattern('patterns', {
        name: 'a',
        steps: ['1'],
        embedding: [0.1],
      });
      await backend.storeProceduralPattern('patterns', {
        name: 'b',
        steps: ['2'],
        embedding: [0.2],
      });

      const id1 = JSON.parse(mockFetch.mock.calls[0][1].body).points[0].id;
      const id2 = JSON.parse(mockFetch.mock.calls[1][1].body).points[0].id;

      expect(id1).not.toBe(id2);
    });
  });

  describe('learn -> recall round-trip (Issue #1039)', () => {
    it('should recall a stored procedural pattern via vector search', async () => {
      const embedding = [0.1, 0.2, 0.3];

      // 1. learnProcedure -> store point (with vector)
      mockFetch.mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({}) });
      const id = await backend.storeProceduralPattern('patterns', {
        name: 'deploy',
        steps: ['build', 'push'],
        embedding,
      });

      const stored = JSON.parse(mockFetch.mock.calls[0][1].body).points[0];
      expect(stored.vector).toEqual(embedding);

      // 2. matchProceduralPatterns -> vector search returns the stored point
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          results: [
            { id, score: 1.0, payload: { _memory_type: 'procedural', name: 'deploy' } },
          ],
        }),
      });

      const matches = await backend.matchProceduralPatterns('patterns', embedding, 5);

      // Before the fix the point had no vector and search returned [].
      expect(matches).toHaveLength(1);
      expect(matches[0].id).toBe(id);
      expect(matches[0].payload?.name).toBe('deploy');
    });
  });

  // ── numeric timestamp + content field alignment ───────────────────────────
  describe('episodic numeric timestamp + semantic content field', () => {
    it('writes content (not text) for a semantic fact', async () => {
      mockFetch.mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({}) });

      await backend.storeSemanticFact('facts', {
        id: 7,
        text: 'the sky is blue',
        embedding: [0.1, 0.2],
      });

      const point = JSON.parse(mockFetch.mock.calls[0][1].body).points[0];
      expect(point.payload.content).toBe('the sky is blue');
      expect(point.payload.text).toBeUndefined();
      expect(point.payload._memory_type).toBe('semantic');
    });

    it('defaults episodic timestamp to numeric unix-seconds (floor ms/1000)', async () => {
      // 1700000000123 ms -> 1700000000 s
      vi.spyOn(Date, 'now').mockReturnValue(1700000000123);
      mockFetch.mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({}) });

      await backend.recordEpisodicEvent('events', {
        eventType: 'click',
        embedding: [0.1, 0.2],
        data: {},
      });

      const point = JSON.parse(mockFetch.mock.calls[0][1].body).points[0];
      expect(point.payload.timestamp).toBe(1700000000);
      expect(typeof point.payload.timestamp).toBe('number');
    });

    it('round-trips an explicit numeric timestamp unchanged', async () => {
      mockFetch.mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({}) });

      await backend.recordEpisodicEvent('events', {
        eventType: 'login',
        embedding: [0.3, 0.4],
        data: {},
        timestamp: 1234567890,
      });

      const point = JSON.parse(mockFetch.mock.calls[0][1].body).points[0];
      expect(point.payload.timestamp).toBe(1234567890);
    });
  });

  // ── string-id integrity across the u64 boundary ───────────────────────────
  describe('string-id boundary (u64 precision)', () => {
    it('returns generated episodic/procedural ids as strings', async () => {
      mockFetch
        .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({}) })
        .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({}) });

      const eId = await backend.recordEpisodicEvent('events', {
        eventType: 'x', embedding: [0.1], data: {},
      });
      const pId = await backend.storeProceduralPattern('patterns', {
        name: 'p', steps: ['a'], embedding: [0.1],
      });

      expect(typeof eId).toBe('string');
      expect(typeof pId).toBe('string');
    });

    it('accepts a caller numeric-string id within the safe range and preserves it exactly', async () => {
      mockFetch.mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({}) });

      // 9007199254740991 == Number.MAX_SAFE_INTEGER, the largest exact u64 in JS.
      const big = String(Number.MAX_SAFE_INTEGER);
      const id = await backend.recordEpisodicEvent('events', {
        id: big, eventType: 'x', embedding: [0.1], data: {},
      });

      const point = JSON.parse(mockFetch.mock.calls[0][1].body).points[0];
      // The wire id is the exact integer; the returned id is its string form.
      expect(point.id).toBe(Number.MAX_SAFE_INTEGER);
      expect(id).toBe(big);
    });

    it('rejects a caller id beyond 2^53 rather than silently corrupting it', async () => {
      // 2^53 + 1 cannot be represented exactly as a JS number; the REST wire
      // only accepts JSON numbers, so this must throw, not lose precision.
      const tooBig = '9007199254740993';
      await expect(
        backend.storeProceduralPattern('patterns', {
          id: tooBig, name: 'p', steps: ['a'], embedding: [0.1],
        })
      ).rejects.toThrow();
    });
  });

  // ── explicit caller-provided ids everywhere ───────────────────────────────
  describe('caller-provided ids for episodic/procedural', () => {
    it('uses the caller id for an episodic event', async () => {
      mockFetch.mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({}) });

      const id = await backend.recordEpisodicEvent('events', {
        id: 42, eventType: 'x', embedding: [0.1], data: {},
      });

      const point = JSON.parse(mockFetch.mock.calls[0][1].body).points[0];
      expect(point.id).toBe(42);
      expect(id).toBe('42');
    });

    it('uses the caller id for a procedural pattern', async () => {
      mockFetch.mockResolvedValueOnce({ ok: true, json: () => Promise.resolve({}) });

      const id = await backend.storeProceduralPattern('patterns', {
        id: 99, name: 'p', steps: ['a'], embedding: [0.1],
      });

      const point = JSON.parse(mockFetch.mock.calls[0][1].body).points[0];
      expect(point.id).toBe(99);
      expect(id).toBe('99');
    });
  });

  // ── temporal recall mirrors core recent / older_than ──────────────────────
  describe('recallRecent / recallOlderThan', () => {
    // Single scroll page of episodic points at distinct timestamps.
    const scrollPage = {
      ok: true,
      json: () => Promise.resolve({
        points: [
          { id: 1, payload: { _memory_type: 'episodic', event_type: 'a', timestamp: 100 } },
          { id: 2, payload: { _memory_type: 'episodic', event_type: 'b', timestamp: 300 } },
          { id: 3, payload: { _memory_type: 'episodic', event_type: 'c', timestamp: 200 } },
          // Non-episodic / missing-timestamp points must be ignored.
          { id: 4, payload: { _memory_type: 'semantic', content: 'x' } },
          { id: 5, payload: { _memory_type: 'episodic', event_type: 'd' } },
        ],
        next_cursor: null,
      }),
    };

    it('recallRecent returns all episodic events most-recent-first', async () => {
      mockFetch.mockResolvedValueOnce(scrollPage);

      const events = await backend.recallRecentEvents('events');

      expect(events.map((e) => e.timestamp)).toEqual([300, 200, 100]);
      expect(events.map((e) => e.id)).toEqual(['2', '3', '1']);
    });

    it('recallRecent honours the since lower bound (inclusive)', async () => {
      mockFetch.mockResolvedValueOnce(scrollPage);

      const events = await backend.recallRecentEvents('events', 200);

      expect(events.map((e) => e.timestamp)).toEqual([300, 200]);
    });

    it('recallOlderThan returns events strictly below the threshold', async () => {
      mockFetch.mockResolvedValueOnce(scrollPage);

      const events = await backend.recallOlderThanEvents('events', 200);

      // 200 is excluded (strict <), 100 included.
      expect(events.map((e) => e.timestamp)).toEqual([100]);
    });

    it('paginates across scroll cursors', async () => {
      mockFetch
        .mockResolvedValueOnce({
          ok: true,
          json: () => Promise.resolve({
            points: [{ id: 1, payload: { _memory_type: 'episodic', timestamp: 100 } }],
            next_cursor: 1,
          }),
        })
        .mockResolvedValueOnce({
          ok: true,
          json: () => Promise.resolve({
            points: [{ id: 2, payload: { _memory_type: 'episodic', timestamp: 200 } }],
            next_cursor: null,
          }),
        });

      const events = await backend.recallRecentEvents('events');

      expect(mockFetch).toHaveBeenCalledTimes(2);
      expect(events.map((e) => e.timestamp)).toEqual([200, 100]);
    });
  });
});
