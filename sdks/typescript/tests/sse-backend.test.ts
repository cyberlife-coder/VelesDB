/**
 * Tests for the SSE streaming traversal endpoint
 * (GET /collections/{name}/graph/traverse/stream).
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { streamTraverse, type SseTransport } from '../src/backends/sse-backend';
import { RestBackend } from '../src/backends/rest';
import { VelesDB } from '../src/client';
import type { TraverseStreamEvent } from '../src/types';

const mockFetch = vi.fn();
global.fetch = mockFetch;

function sseBody(events: string): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  return new ReadableStream({
    start(controller) {
      // deliver in two chunks to exercise the incremental parser
      const half = Math.ceil(events.length / 2);
      controller.enqueue(encoder.encode(events.slice(0, half)));
      controller.enqueue(encoder.encode(events.slice(half)));
      controller.close();
    },
  });
}

const transport: SseTransport = {
  baseUrl: 'http://localhost:8080',
  apiKey: 'k',
  timeout: 5000,
};

async function collect(
  t: SseTransport,
  collection: string,
  request: Parameters<typeof streamTraverse>[2]
): Promise<TraverseStreamEvent[]> {
  const out: TraverseStreamEvent[] = [];
  for await (const ev of streamTraverse(t, collection, request)) out.push(ev);
  return out;
}

describe('streamTraverse (SSE)', () => {
  beforeEach(() => vi.clearAllMocks());
  afterEach(() => vi.restoreAllMocks());

  it('parses node/stats/done events and builds the query string', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      headers: { get: () => 'text/event-stream' },
      body: sseBody(
        'event: node\ndata: {"id":"7","depth":0,"path":[]}\n\n' +
          'event: node\ndata: {"id":"8","depth":1,"path":["7"]}\n\n' +
          'event: stats\ndata: {"nodes_visited":2,"elapsed_ms":3}\n\n' +
          'event: done\ndata: {"total_nodes":2,"max_depth_reached":1,"elapsed_ms":4}\n\n'
      ),
    });

    const events = await collect(transport, 'kg', {
      startNode: 7,
      algorithm: 'dfs',
      maxDepth: 5,
      limit: 100,
      relationshipTypes: ['KNOWS', 'LIKES'],
    });

    expect(events).toEqual([
      { type: 'node', id: '7', depth: 0, path: [] },
      { type: 'node', id: '8', depth: 1, path: ['7'] },
      { type: 'stats', nodesVisited: 2, elapsedMs: 3 },
      { type: 'done', totalNodes: 2, maxDepthReached: 1, elapsedMs: 4 },
    ]);
    const [url, init] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect(url).toBe(
      'http://localhost:8080/collections/kg/graph/traverse/stream?start_node=7&algorithm=dfs&max_depth=5&limit=100&relationship_types=KNOWS%2CLIKES'
    );
    expect(init.method).toBe('GET');
    expect((init.headers as Record<string, string>).Accept).toBe('text/event-stream');
    expect((init.headers as Record<string, string>).Authorization).toBe('Bearer k');
  });

  it('yields error events as data and skips unknown events', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      headers: { get: () => 'text/event-stream' },
      body: sseBody(
        'event: message\ndata: {"ignored":true}\n\n' +
          'event: error\ndata: {"error":"boom"}\n\n'
      ),
    });
    const events = await collect(transport, 'kg', { startNode: 1 });
    expect(events).toEqual([{ type: 'error', message: 'boom' }]);
  });

  it('throws the typed error on a non-200 status', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 409,
      headers: { get: () => null },
      json: () => Promise.resolve({ code: 'VELES-021', error: 'not a graph collection' }),
    });
    await expect(collect(transport, 'docs', { startNode: 1 })).rejects.toMatchObject({
      code: 'VELES-021',
    });
  });

  it('wraps network failures in ConnectionError', async () => {
    mockFetch.mockRejectedValueOnce(new Error('socket hangup'));
    await expect(collect(transport, 'kg', { startNode: 1 })).rejects.toThrow(/Stream traverse failed/);
  });

  it('wraps midstream body failures in ConnectionError (not a raw AbortError/TypeError)', async () => {
    const encoder = new TextEncoder();
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(encoder.encode('event: node\ndata: {"id":"7","depth":0,"path":[]}\n\n'));
        controller.error(new Error('connection reset mid-stream'));
      },
    });
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      headers: { get: () => 'text/event-stream' },
      body,
    });
    await expect(collect(transport, 'kg', { startNode: 1 })).rejects.toMatchObject({
      code: 'CONNECTION_ERROR',
      message: expect.stringMatching(/Stream traverse failed/),
    });
  });

  it('aborts the request when the consumer breaks out of the loop early', async () => {
    const encoder = new TextEncoder();
    let pull = 0;
    const body = new ReadableStream<Uint8Array>({
      pull(controller) {
        pull += 1;
        if (pull === 1) {
          controller.enqueue(encoder.encode('event: node\ndata: {"id":"7","depth":0,"path":[]}\n\n'));
        }
        // never close: the stream stays open, like a live traversal
      },
    });
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      headers: { get: () => 'text/event-stream' },
      body,
    });
    const seen: TraverseStreamEvent[] = [];
    for await (const ev of streamTraverse(transport, 'kg', { startNode: 1 })) {
      seen.push(ev);
      break; // consumer stops after the first event
    }
    const [, init] = mockFetch.mock.calls[0] as [string, RequestInit & { signal: AbortSignal }];
    expect(seen).toEqual([{ type: 'node', id: '7', depth: 0, path: [] }]);
    expect(init.signal.aborted).toBe(true);
  });
});

describe('client streamTraverse delegation', () => {
  beforeEach(() => vi.clearAllMocks());

  it('refuses NOT_SUPPORTED when the backend lacks the route', async () => {
    const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
    (db as unknown as { backend: unknown }).backend = { capabilities: () => ({}) };
    (db as unknown as { initialized: boolean }).initialized = true;
    expect(() => db.streamTraverse('kg', { startNode: 1 })).toThrow(/NOT_SUPPORTED|not supported/);
  });

  it('delegates to the backend when present', async () => {
    async function* fake(): AsyncIterable<TraverseStreamEvent> {
      yield { type: 'done', totalNodes: 0, maxDepthReached: 0, elapsedMs: 0 };
    }
    const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
    (db as unknown as { backend: unknown }).backend = {
      capabilities: () => ({}),
      streamTraverse: vi.fn(() => fake()),
    };
    (db as unknown as { initialized: boolean }).initialized = true;
    const events: TraverseStreamEvent[] = [];
    for await (const ev of db.streamTraverse('kg', { startNode: 1 })) events.push(ev);
    expect(events).toEqual([{ type: 'done', totalNodes: 0, maxDepthReached: 0, elapsedMs: 0 }]);
  });

  it('exposes the route on RestBackend', async () => {
    const backend = new RestBackend('http://localhost:8080');
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ status: 'ok' }),
    });
    await backend.init();
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      headers: { get: () => 'text/event-stream' },
      body: sseBody('event: done\ndata: {"total_nodes":1,"max_depth_reached":0,"elapsed_ms":1}\n\n'),
    });
    const events: TraverseStreamEvent[] = [];
    for await (const ev of backend.streamTraverse('kg', { startNode: 2 })) events.push(ev);
    expect(events).toEqual([{ type: 'done', totalNodes: 1, maxDepthReached: 0, elapsedMs: 1 }]);
  });
});
