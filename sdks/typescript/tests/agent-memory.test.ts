/**
 * Agent Memory SDK Tests (Issues #6, #7 from PR 306)
 *
 * - #6: storeProceduralPattern must NOT send a vector field
 * - #7: generateUniqueId() must produce unique IDs under rapid-fire calls
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

  describe('storeProceduralPattern (Issue #6)', () => {
    it('should NOT include a vector field in the request body', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({}),
      });

      await backend.storeProceduralPattern('patterns', {
        name: 'deploy',
        steps: ['build', 'test', 'push'],
        metadata: { env: 'prod' },
      });

      expect(mockFetch).toHaveBeenCalledTimes(1);
      const body = JSON.parse(mockFetch.mock.calls[0][1].body);
      const point = body.points[0];

      // vector must be absent
      expect(point).not.toHaveProperty('vector');
      // payload must still be present
      expect(point.payload._memory_type).toBe('procedural');
      expect(point.payload.name).toBe('deploy');
      expect(point.payload.steps).toEqual(['build', 'test', 'push']);
      expect(point.payload.env).toBe('prod');
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
      });
      await backend.storeProceduralPattern('patterns', {
        name: 'b',
        steps: ['2'],
      });

      const id1 = JSON.parse(mockFetch.mock.calls[0][1].body).points[0].id;
      const id2 = JSON.parse(mockFetch.mock.calls[1][1].body).points[0].id;

      expect(id1).not.toBe(id2);
    });
  });
});
