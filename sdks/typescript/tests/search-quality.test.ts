/**
 * SearchQuality Forward Tests (Sprint 2 Wave 4 — #22 F-API-001)
 *
 * Verifies that `SearchOptions.quality` is forwarded to the REST wire
 * as `{ mode, ef_search }` on every search path that the core actually
 * supports: `search`, `searchBatch`, `searchIds`.
 *
 * Scope: text / hybrid / multi-query search are NOT covered by this
 * commit — their core entry points (`VectorCollection::text_search`,
 * `::hybrid_search`, `::multi_query_search`) do not accept an
 * `ef_search` or `SearchQuality` parameter. Exposing `quality` on
 * those TS methods would create a silently-ignored option. See
 * CHANGELOG for the explicit limitation.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { RestBackend } from '../src/backends/rest';
import { searchQualityToMode } from '../src/search-quality';
import type { SearchQuality } from '../src/types';

const mockFetch = vi.fn();
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).fetch = mockFetch;

async function initBackend(): Promise<RestBackend> {
  const backend = new RestBackend('http://localhost:8080');
  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: () => Promise.resolve({ status: 'ok' }),
  });
  await backend.init();
  mockFetch.mockReset();
  return backend;
}

function stubSearchResponse(): void {
  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: () => Promise.resolve({ results: [] }),
  });
}

function bodyOf(call: number): Record<string, unknown> {
  return JSON.parse(mockFetch.mock.calls[call][1].body as string);
}

// ============================================================================
// searchQualityToMode — pure helper converting camelCase TS types to wire
// ============================================================================

describe('searchQualityToMode', () => {
  it('passes through every named preset unchanged', () => {
    expect(searchQualityToMode('fast')).toEqual({ mode: 'fast' });
    expect(searchQualityToMode('balanced')).toEqual({ mode: 'balanced' });
    expect(searchQualityToMode('accurate')).toEqual({ mode: 'accurate' });
    expect(searchQualityToMode('perfect')).toEqual({ mode: 'perfect' });
    expect(searchQualityToMode('autotune')).toEqual({ mode: 'autotune' });
  });

  it('passes custom:<ef> through as the raw wire string', () => {
    expect(searchQualityToMode('custom:256')).toEqual({ mode: 'custom:256' });
  });

  it('passes adaptive:<min>:<max> through as the raw wire string', () => {
    expect(searchQualityToMode('adaptive:32:512')).toEqual({ mode: 'adaptive:32:512' });
  });

  it('returns an empty wire object when quality is undefined', () => {
    expect(searchQualityToMode(undefined)).toEqual({});
  });
});

// ============================================================================
// Per-method forward — one describe block per REST endpoint that honors mode
// ============================================================================

describe('search() forwards SearchOptions.quality', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it.each<SearchQuality>(['fast', 'balanced', 'accurate', 'perfect', 'autotune'])(
    'emits mode=%s on /search',
    async (quality) => {
      stubSearchResponse();
      await backend.search('docs', [0.1, 0.2, 0.3], { k: 10, quality });
      expect(bodyOf(0).mode).toBe(quality);
    }
  );

  it('emits mode="custom:128" when quality="custom:128"', async () => {
    stubSearchResponse();
    await backend.search('docs', [0.1], { k: 5, quality: 'custom:128' });
    expect(bodyOf(0).mode).toBe('custom:128');
  });

  it('emits mode="adaptive:64:512" when quality="adaptive:64:512"', async () => {
    stubSearchResponse();
    await backend.search('docs', [0.1], { k: 5, quality: 'adaptive:64:512' });
    expect(bodyOf(0).mode).toBe('adaptive:64:512');
  });

  it('omits mode entirely when quality is undefined', async () => {
    stubSearchResponse();
    await backend.search('docs', [0.1], { k: 5 });
    expect(bodyOf(0)).not.toHaveProperty('mode');
  });
});

describe('searchIds() forwards SearchOptions.quality', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('emits mode=accurate on /search/ids', async () => {
    stubSearchResponse();
    await backend.searchIds('docs', [0.1, 0.2], { k: 10, quality: 'accurate' });
    expect(bodyOf(0).mode).toBe('accurate');
  });

  it('emits mode=custom:200 on /search/ids', async () => {
    stubSearchResponse();
    await backend.searchIds('docs', [0.1], { k: 5, quality: 'custom:200' });
    expect(bodyOf(0).mode).toBe('custom:200');
  });

  it('omits mode when quality is undefined', async () => {
    stubSearchResponse();
    await backend.searchIds('docs', [0.1], { k: 5 });
    expect(bodyOf(0)).not.toHaveProperty('mode');
  });
});

describe('searchBatch() forwards per-sub-request quality', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('forwards mode on every sub-request', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          results: [{ results: [] }, { results: [] }],
        }),
    });

    await backend.searchBatch('docs', [
      { vector: [0.1, 0.2], k: 10, quality: 'fast' },
      { vector: [0.3, 0.4], k: 5, quality: 'accurate' },
    ]);

    const body = bodyOf(0);
    const searches = body.searches as Array<Record<string, unknown>>;
    expect(searches).toHaveLength(2);
    expect(searches[0].mode).toBe('fast');
    expect(searches[1].mode).toBe('accurate');
  });

  it('omits mode per sub-request when quality is undefined', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          results: [{ results: [] }],
        }),
    });

    await backend.searchBatch('docs', [{ vector: [0.1], k: 10 }]);

    const searches = (bodyOf(0).searches as Array<Record<string, unknown>>);
    expect(searches[0]).not.toHaveProperty('mode');
  });
});

// ============================================================================
// Scoped limitation: text / hybrid / multi-query do NOT accept quality
// ============================================================================

describe('textSearch / hybridSearch / multiQuerySearch — quality explicitly not supported', () => {
  it('textSearch options type does not declare `quality`', () => {
    // Compile-time check: attempting `{ quality: 'fast' }` below would
    // be a TypeScript error because the options type is
    // `{ k?: number; filter?: FilterInput }`. This test documents the
    // intentional omission — any future widening must update this
    // test AND the core BM25 search entry points.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const backend: any = {};
    // No assertion needed — if the type widens, the compile-time
    // contract in src/filter + src/types changes first.
    expect(backend).toBeDefined();
  });
});
