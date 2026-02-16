/**
 * SSE Streaming Tests for Graph Traversal (P2 #8)
 *
 * Tests the SSE parser logic: processSseLine, dispatchSseEvent,
 * buildStreamUrl, and validateStreamResponse.
 *
 * These are unit tests that don't require a running server.
 */

import { describe, it, expect, vi } from 'vitest';

// Re-export internal helpers for testing by importing the module
// and testing through the public streamTraverseGraph function behavior.
// Since helpers are not exported, we test via integration with mocked fetch.

import type { StreamTraverseCallbacks } from '../src/types';
import { VelesDBError, NotFoundError } from '../src/types';

// Mock global fetch
const mockFetch = vi.fn();
globalThis.fetch = mockFetch;

// Helper: create a ReadableStream from SSE text chunks
function createSseStream(chunks: string[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  let index = 0;
  return new ReadableStream({
    pull(controller) {
      if (index < chunks.length) {
        controller.enqueue(encoder.encode(chunks[index]));
        index++;
      } else {
        controller.close();
      }
    },
  });
}

// We need to import streamTraverseGraph from the graph module
// but it depends on HttpClient. Let's test the SSE parsing logic
// by importing and calling streamTraverseGraph with a properly mocked client.

describe('SSE Streaming - Graph Traversal', () => {
  // Dynamically import to avoid module-level side effects
  let streamTraverseGraph: typeof import('../src/backends/rest/graph').streamTraverseGraph;

  beforeAll(async () => {
    const mod = await import('../src/backends/rest/graph');
    streamTraverseGraph = mod.streamTraverseGraph;
  });

  it('should parse node events from SSE stream', async () => {
    const nodes: Array<{ id: number; depth: number; path: number[] }> = [];

    const sseData = [
      'event:node\ndata:{"id":1,"depth":0,"path":[]}\n\n',
      'event:node\ndata:{"id":2,"depth":1,"path":[100]}\n\n',
      'event:done\ndata:{"total_nodes":2,"max_depth_reached":1,"elapsed_ms":5}\n\n',
    ];

    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      body: createSseStream(sseData),
    });

    const callbacks: StreamTraverseCallbacks = {
      onNode: (node) => nodes.push(node),
    };

    const mockClient = {
      ensureInitialized: vi.fn(),
      getBaseUrl: () => 'http://localhost:8080',
      getHeaders: () => ({ 'Authorization': 'Bearer test' }),
    };

    await streamTraverseGraph(
      mockClient as any,
      'test-collection',
      { source: 1 },
      callbacks,
    );

    expect(nodes).toHaveLength(2);
    expect(nodes[0]).toEqual({ id: 1, depth: 0, path: [] });
    expect(nodes[1]).toEqual({ id: 2, depth: 1, path: [100] });
  });

  it('should parse stats and done events', async () => {
    let statsResult: any = null;
    let doneResult: any = null;

    const sseData = [
      'event:node\ndata:{"id":1,"depth":0,"path":[]}\n\n',
      'event:stats\ndata:{"nodes_visited":10,"elapsed_ms":3}\n\n',
      'event:done\ndata:{"total_nodes":10,"max_depth_reached":2,"elapsed_ms":5}\n\n',
    ];

    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      body: createSseStream(sseData),
    });

    const callbacks: StreamTraverseCallbacks = {
      onNode: vi.fn(),
      onStats: (stats) => { statsResult = stats; },
      onDone: (done) => { doneResult = done; },
    };

    const mockClient = {
      ensureInitialized: vi.fn(),
      getBaseUrl: () => 'http://localhost:8080',
      getHeaders: () => ({}),
    };

    await streamTraverseGraph(mockClient as any, 'col', { source: 1 }, callbacks);

    expect(statsResult).toEqual({ nodesVisited: 10, elapsedMs: 3 });
    expect(doneResult).toEqual({ totalNodes: 10, maxDepthReached: 2, elapsedMs: 5 });
  });

  it('should handle error events from SSE stream', async () => {
    let errorResult: Error | null = null;

    const sseData = [
      'event:error\ndata:{"error":"traversal timeout"}\n\n',
    ];

    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      body: createSseStream(sseData),
    });

    const callbacks: StreamTraverseCallbacks = {
      onNode: vi.fn(),
      onError: (err) => { errorResult = err; },
    };

    const mockClient = {
      ensureInitialized: vi.fn(),
      getBaseUrl: () => 'http://localhost:8080',
      getHeaders: () => ({}),
    };

    await streamTraverseGraph(mockClient as any, 'col', { source: 1 }, callbacks);

    expect(errorResult).toBeInstanceOf(VelesDBError);
    expect((errorResult as VelesDBError).message).toContain('traversal timeout');
  });

  it('should throw NotFoundError on 404 response', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 404,
      body: null,
    });

    const mockClient = {
      ensureInitialized: vi.fn(),
      getBaseUrl: () => 'http://localhost:8080',
      getHeaders: () => ({}),
    };

    await expect(
      streamTraverseGraph(mockClient as any, 'missing-col', { source: 1 }, { onNode: vi.fn() })
    ).rejects.toThrow(NotFoundError);
  });

  it('should throw VelesDBError when response has no body', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      body: null,
    });

    const mockClient = {
      ensureInitialized: vi.fn(),
      getBaseUrl: () => 'http://localhost:8080',
      getHeaders: () => ({}),
    };

    await expect(
      streamTraverseGraph(mockClient as any, 'col', { source: 1 }, { onNode: vi.fn() })
    ).rejects.toThrow(VelesDBError);
  });

  it('should handle malformed JSON in SSE data gracefully', async () => {
    let errorResult: Error | null = null;

    const sseData = [
      'event:node\ndata:{broken json}\n\n',
    ];

    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      body: createSseStream(sseData),
    });

    const callbacks: StreamTraverseCallbacks = {
      onNode: vi.fn(),
      onError: (err) => { errorResult = err; },
    };

    const mockClient = {
      ensureInitialized: vi.fn(),
      getBaseUrl: () => 'http://localhost:8080',
      getHeaders: () => ({}),
    };

    await streamTraverseGraph(mockClient as any, 'col', { source: 1 }, callbacks);

    expect(errorResult).toBeInstanceOf(VelesDBError);
    expect((errorResult as VelesDBError).message).toContain('Failed to parse SSE data');
  });
});
