/**
 * Missing REST Endpoint Wrapper Tests (Sprint 2 Wave 4 — S2-NEW-10)
 *
 * BDD coverage for the 12 REST wrappers added in this commit. Every
 * test stubs `fetch`, asserts the wire format sent to the server,
 * and verifies the response is mapped back into camelCase TS types.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { RestBackend } from '../src/backends/rest';

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

function lastCall(): { url: string; method: string; body?: Record<string, unknown> } {
  const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
  const url = call[0] as string;
  const init = call[1];
  const body = init?.body ? JSON.parse(init.body as string) : undefined;
  return { url, method: init?.method as string, body };
}

// ============================================================================
// Admin endpoints
// ============================================================================

describe('rebuildIndex', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('POSTs to /collections/{name}/index/rebuild and maps response', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          message: 'Index rebuilt',
          collection: 'docs',
          compacted_entries: 42,
        }),
    });

    const result = await backend.rebuildIndex('docs');

    expect(lastCall().method).toBe('POST');
    expect(lastCall().url).toContain('/collections/docs/index/rebuild');
    expect(result).toEqual({
      message: 'Index rebuilt',
      collection: 'docs',
      compactedEntries: 42,
    });
  });
});

describe('getGuardrails', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('GETs /guardrails and maps snake_case → camelCase', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          max_depth: 10,
          max_cardinality: 100_000,
          memory_limit_bytes: 104_857_600,
          timeout_ms: 30_000,
          rate_limit_qps: 100,
          circuit_failure_threshold: 5,
          circuit_recovery_seconds: 30,
        }),
    });

    const cfg = await backend.getGuardrails();
    expect(lastCall().method).toBe('GET');
    expect(lastCall().url).toContain('/guardrails');
    expect(cfg.maxDepth).toBe(10);
    expect(cfg.maxCardinality).toBe(100_000);
    expect(cfg.memoryLimitBytes).toBe(104_857_600);
    expect(cfg.timeoutMs).toBe(30_000);
    expect(cfg.rateLimitQps).toBe(100);
    expect(cfg.circuitFailureThreshold).toBe(5);
    expect(cfg.circuitRecoverySeconds).toBe(30);
  });
});

describe('updateGuardrails', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('PUTs only the supplied fields as snake_case', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          max_depth: 15,
          max_cardinality: 100_000,
          memory_limit_bytes: 104_857_600,
          timeout_ms: 30_000,
          rate_limit_qps: 200,
          circuit_failure_threshold: 5,
          circuit_recovery_seconds: 30,
        }),
    });

    const updated = await backend.updateGuardrails({
      maxDepth: 15,
      rateLimitQps: 200,
    });

    const call = lastCall();
    expect(call.method).toBe('PUT');
    expect(call.body).toEqual({ max_depth: 15, rate_limit_qps: 200 });
    expect(updated.maxDepth).toBe(15);
    expect(updated.rateLimitQps).toBe(200);
  });

  it('omits unset fields entirely from the PUT body', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          max_depth: 10,
          max_cardinality: 100_000,
          memory_limit_bytes: 104_857_600,
          timeout_ms: 30_000,
          rate_limit_qps: 100,
          circuit_failure_threshold: 5,
          circuit_recovery_seconds: 30,
        }),
    });

    await backend.updateGuardrails({});
    expect(lastCall().body).toEqual({});
  });
});

// ============================================================================
// Query endpoints
// ============================================================================

describe('aggregate', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('POSTs to /aggregate with query + params and parses AggregateResponse', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          result: [{ category: 'tech', n: 42 }],
          timing_ms: 12.5,
          meta: {
            velesql_contract_version: '3.6',
            count: 1,
          },
        }),
    });

    const resp = await backend.aggregate(
      'SELECT category, COUNT(*) FROM docs GROUP BY category',
      { min_score: 0.5 }
    );

    const call = lastCall();
    expect(call.method).toBe('POST');
    expect(call.url).toContain('/aggregate');
    expect(call.body?.query).toBe('SELECT category, COUNT(*) FROM docs GROUP BY category');
    expect(call.body?.params).toEqual({ min_score: 0.5 });
    expect(call.body).not.toHaveProperty('timeout_ms');
    expect(resp.result).toEqual([{ category: 'tech', n: 42 }]);
    expect(resp.timingMs).toBe(12.5);
    expect(resp.meta.velesqlContractVersion).toBe('3.6');
    expect(resp.meta.count).toBe(1);
  });

  it('forwards options.collection as snake_case', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          result: {},
          timing_ms: 0,
          meta: { velesql_contract_version: '3.6', count: 0 },
        }),
    });
    await backend.aggregate('SELECT COUNT(*)', {}, { collection: 'docs' });
    expect(lastCall().body?.collection).toBe('docs');
  });
});

describe('matchQuery', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('POSTs to /collections/{name}/match and maps MatchQueryResponse', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          results: [
            {
              bindings: { a: 1, b: 2 },
              score: 0.92,
              depth: 2,
              projected: { 'a.name': 'Alice' },
            },
          ],
          took_ms: 7,
          count: 1,
          meta: { velesql_contract_version: '3.6' },
        }),
    });

    const resp = await backend.matchQuery(
      'kg',
      'MATCH (a:Person)-[:KNOWS]->(b) RETURN b',
      { source: 42 }
    );

    const call = lastCall();
    expect(call.method).toBe('POST');
    expect(call.url).toContain('/collections/kg/match');
    expect(call.body?.query).toBe('MATCH (a:Person)-[:KNOWS]->(b) RETURN b');
    expect(call.body?.params).toEqual({ source: 42 });
    expect(call.body).not.toHaveProperty('timeout_ms');

    expect(resp.results).toHaveLength(1);
    expect(resp.results[0].bindings).toEqual({ a: 1, b: 2 });
    expect(resp.results[0].score).toBe(0.92);
    expect(resp.results[0].depth).toBe(2);
    expect(resp.results[0].projected).toEqual({ 'a.name': 'Alice' });
    expect(resp.tookMs).toBe(7);
    expect(resp.count).toBe(1);
    expect(resp.meta.velesqlContractVersion).toBe('3.6');
  });

  it('forwards options.vector + options.threshold for similarity() MATCH', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          results: [],
          took_ms: 0,
          count: 0,
          meta: { velesql_contract_version: '3.6' },
        }),
    });
    await backend.matchQuery(
      'kg',
      'MATCH (a:Person) WHERE similarity(a.vec, $v) > 0.7 RETURN a',
      {},
      { vector: [0.1, 0.2, 0.3], threshold: 0.7 }
    );
    const body = lastCall().body!;
    expect(body.vector).toEqual([0.1, 0.2, 0.3]);
    expect(body.threshold).toBe(0.7);
  });

  it('defaults projected to empty object when server omits it', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          results: [{ bindings: { a: 1 }, depth: 0 }],
          took_ms: 1,
          count: 1,
          meta: { velesql_contract_version: '3.6' },
        }),
    });
    const resp = await backend.matchQuery('kg', 'MATCH (a) RETURN a');
    expect(resp.results[0].projected).toEqual({});
    expect(resp.results[0].score).toBeUndefined();
  });
});

// ============================================================================
// Graph endpoints
// ============================================================================

describe('removeEdge', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('DELETEs and returns true on success', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({}),
    });
    const removed = await backend.removeEdge('kg', 42);
    expect(lastCall().method).toBe('DELETE');
    expect(lastCall().url).toContain('/collections/kg/graph/edges/42');
    expect(removed).toBe(true);
  });

  it('returns false when the server answers typed VELES-020 (edge not found)', async () => {
    // Modern server: `core_error_response` propagates the VELES-020 code
    // through the wire, which `isNotFoundError` recognises via the
    // `EdgeNotFoundError` branch.
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 404,
      json: () =>
        Promise.resolve({
          code: 'VELES-020',
          error: "[VELES-020] Edge with ID '999' not found",
        }),
    });
    const removed = await backend.removeEdge('kg', 999);
    expect(removed).toBe(false);
  });

  it('returns false when the server answers legacy NOT_FOUND (no code field)', async () => {
    // Legacy server: `error_response` omits `code`, the transport layer
    // falls back to `mapStatusToErrorCode(404) → 'NOT_FOUND'`. This guards
    // against the regression described in PR #586 Devin finding #2 where
    // the pre-fix check `response.error.code === 'VELES-020'` missed the
    // status-derived format and threw instead of returning false.
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 404,
      json: () =>
        Promise.resolve({
          error: "Edge with ID '999' not found in collection 'kg'",
        }),
    });
    const removed = await backend.removeEdge('kg', 999);
    expect(removed).toBe(false);
  });
});

describe('getEdgeCount', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('GETs /graph/edges/count and returns the count', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ count: 1337 }),
    });
    const n = await backend.getEdgeCount('kg');
    expect(lastCall().url).toContain('/graph/edges/count');
    expect(n).toBe(1337);
  });
});

describe('listNodes', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('GETs /graph/nodes and maps snake_case → camelCase', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ node_ids: [1, 2, 3], count: 3 }),
    });
    const result = await backend.listNodes('kg');
    expect(lastCall().url).toContain('/graph/nodes');
    expect(result).toEqual({ nodeIds: [1, 2, 3], count: 3 });
  });
});

describe('getNodeEdges', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('passes direction + label as query params', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          edges: [
            { id: 1, source: 10, target: 20, label: 'KNOWS', properties: {} },
          ],
          count: 1,
        }),
    });

    const edges = await backend.getNodeEdges('kg', 10, {
      direction: 'in',
      label: 'KNOWS',
    });

    expect(lastCall().url).toContain('/graph/nodes/10/edges');
    expect(lastCall().url).toContain('direction=in');
    expect(lastCall().url).toContain('label=KNOWS');
    expect(edges).toHaveLength(1);
    expect(edges[0].label).toBe('KNOWS');
  });

  it('coerces string-typed IDs from serialize_id_as_string back to number', async () => {
    // The server's `EdgeResponse` struct uses
    // `#[serde(serialize_with = "serde_id::serialize_id_as_string")]`
    // on `id`/`source`/`target`, so the wire format carries strings
    // even though the TS `GraphEdge` interface declares `id: number`.
    // This guards against the regression where the raw string would
    // leak into the public API (PR #586 Devin 4th-wave scan finding).
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          edges: [
            {
              id: '42',
              source: '10',
              target: '20',
              label: 'KNOWS',
              properties: { since: '2020' },
            },
          ],
          count: 1,
        }),
    });

    const edges = await backend.getNodeEdges('kg', 10);
    expect(edges).toHaveLength(1);
    expect(typeof edges[0].id).toBe('number');
    expect(edges[0].id).toBe(42);
    expect(typeof edges[0].source).toBe('number');
    expect(edges[0].source).toBe(10);
    expect(typeof edges[0].target).toBe('number');
    expect(edges[0].target).toBe(20);
    expect(edges[0].properties).toEqual({ since: '2020' });
  });

  it('omits the query string entirely when no options supplied', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ edges: [], count: 0 }),
    });
    await backend.getNodeEdges('kg', 10);
    expect(lastCall().url).not.toContain('?');
  });
});

describe('getNodePayload', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('GETs /graph/nodes/{id}/payload and coerces node_id string → number', async () => {
    // Server uses `serialize_id_as_string` so `node_id` arrives as a string.
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          node_id: '42',
          payload: { name: 'Alice' },
        }),
    });

    const result = await backend.getNodePayload('kg', 42);
    expect(lastCall().url).toContain('/graph/nodes/42/payload');
    expect(typeof result.nodeId).toBe('number');
    expect(result.nodeId).toBe(42);
    expect(result.payload).toEqual({ name: 'Alice' });
  });

  it('preserves a null payload', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ node_id: 42, payload: null }),
    });
    const result = await backend.getNodePayload('kg', 42);
    expect(result.payload).toBeNull();
  });
});

describe('upsertNodePayload', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('PUTs the payload wrapped in `{ payload: ... }`', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({}),
    });
    await backend.upsertNodePayload('kg', 42, { name: 'Alice' });
    const call = lastCall();
    expect(call.method).toBe('PUT');
    expect(call.url).toContain('/graph/nodes/42/payload');
    expect(call.body).toEqual({ payload: { name: 'Alice' } });
  });
});

describe('graphSearch', () => {
  let backend: RestBackend;
  beforeEach(async () => {
    mockFetch.mockReset();
    backend = await initBackend();
  });

  it('POSTs vector + top_k to /graph/search and maps id → number', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          results: [
            { id: '10', score: 0.92, payload: { name: 'Alice' } },
            { id: '20', score: 0.81 },
          ],
        }),
    });

    const res = await backend.graphSearch('kg', {
      vector: [0.1, 0.2, 0.3],
      k: 5,
    });

    const call = lastCall();
    expect(call.method).toBe('POST');
    expect(call.url).toContain('/graph/search');
    expect(call.body?.vector).toEqual([0.1, 0.2, 0.3]);
    expect(call.body?.top_k).toBe(5);
    expect(res.results).toHaveLength(2);
    expect(res.results[0].id).toBe(10);
    expect(typeof res.results[0].id).toBe('number');
    expect(res.results[0].score).toBe(0.92);
    // Payload must round-trip (not dropped by the mapper).
    expect(res.results[0].payload).toEqual({ name: 'Alice' });
    expect(res.results[1].id).toBe(20);
    expect(res.results[1].payload).toBeUndefined();
  });

  it('defaults k to 10 when omitted', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ results: [] }),
    });
    await backend.graphSearch('kg', { vector: [0.1] });
    expect(lastCall().body?.top_k).toBe(10);
  });
});
