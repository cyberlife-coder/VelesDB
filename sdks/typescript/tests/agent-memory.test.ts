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
      // store returns the generated point ID
      expect(point.id).toBe(id);
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
});
