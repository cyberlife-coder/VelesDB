/**
 * Graph Backend Tests (#598)
 *
 * Covers `src/backends/graph-backend.ts`: addEdge, getEdges,
 * traverseGraph, getNodeDegree, createGraphCollection, traverseParallel.
 *
 * Focus: URL shape, snake_case to camelCase mapping, default value
 * application (strategy, maxDepth, limit, relTypes, metric, schemaMode),
 * and the string-to-number coercion for ids returned as strings by the
 * server (to avoid u64 precision loss).
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  addEdge,
  getEdges,
  traverseGraph,
  getNodeDegree,
  createGraphCollection,
  traverseParallel,
  type GraphTransport,
} from '../src/backends/graph-backend';
import type { TransportResponse } from '../src/backends/shared';
import { CollectionNotFoundError } from '../src/errors';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const graphSchemaFixture = JSON.parse(
  readFileSync(
    path.resolve(
      path.dirname(fileURLToPath(import.meta.url)),
      '../../../conformance/graph_schema_wire_cases.json'
    ),
    'utf8'
  )
) as { cases: Array<{ schema_mode: string; graph_schema: unknown }> };

function buildTransport(
  overrides: Partial<GraphTransport> = {}
): GraphTransport {
  return {
    requestJson: vi.fn(),
    ...overrides,
  };
}

function typedError(
  code = 'VELES-002',
  message = "Collection 'missing' not found"
): TransportResponse<never> {
  return { error: { code, message } };
}

describe('addEdge', () => {
  beforeEach(() => vi.clearAllMocks());

  it('POSTs to /graph/edges with default properties={}', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {},
    } satisfies TransportResponse<unknown>);

    await addEdge(transport, 'kg', {
      id: 1,
      source: 10,
      target: 20,
      label: 'KNOWS',
    });

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/kg/graph/edges',
      {
        id: 1,
        source: 10,
        target: 20,
        label: 'KNOWS',
        properties: {},
      }
    );
  });

  it('forwards explicit properties', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {},
    });

    await addEdge(transport, 'kg', {
      id: 1,
      source: 10,
      target: 20,
      label: 'KNOWS',
      properties: { weight: 0.5 },
    });

    const call = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]!;
    expect((call[2] as { properties: unknown }).properties).toEqual({
      weight: 0.5,
    });
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(
      addEdge(transport, 'missing', {
        id: 1,
        source: 10,
        target: 20,
        label: 'KNOWS',
      })
    ).rejects.toThrow(CollectionNotFoundError);
  });
});

describe('getEdges', () => {
  beforeEach(() => vi.clearAllMocks());

  it('GETs /graph/edges without query params by default', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { edges: [], count: 0 },
    });

    await getEdges(transport, 'kg');

    expect(transport.requestJson).toHaveBeenCalledWith(
      'GET',
      '/collections/kg/graph/edges'
    );
  });

  it('adds URL-encoded label filter when provided', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { edges: [], count: 0 },
    });

    await getEdges(transport, 'kg', { label: 'KNOWS OF' });

    const call = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]!;
    expect(call[1]).toBe('/collections/kg/graph/edges?label=KNOWS%20OF');
  });

  it('coerces string ids (u64 safety) to numbers', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {
        edges: [
          {
            id: '42',
            source: '10',
            target: '20',
            label: 'KNOWS',
            properties: { k: 1 },
          },
        ],
        count: 1,
      },
    });

    const edges = await getEdges(transport, 'kg');

    expect(edges).toEqual([
      {
        id: '42',
        source: '10',
        target: '20',
        label: 'KNOWS',
        properties: { k: 1 },
      },
    ]);
  });

  it('preserves graph ids above Number.MAX_SAFE_INTEGER', async () => {
    const transport = buildTransport();
    const largeId = '9007199254740993';
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {
        edges: [{ id: largeId, source: largeId, target: '9007199254740995', label: 'KNOWS' }],
        count: 1,
      },
    });

    const edges = await getEdges(transport, 'kg');

    expect(edges[0].id).toBe(largeId);
    expect(edges[0].source).toBe(largeId);
    expect(edges[0].target).toBe('9007199254740995');
  });

  it('passes through numeric ids unchanged', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {
        edges: [{ id: 7, source: 1, target: 2, label: 'L' }],
        count: 1,
      },
    });

    const edges = await getEdges(transport, 'kg');
    expect(edges[0]!.id).toBe(7);
    expect(edges[0]!.properties).toBeUndefined();
  });

  it('returns [] when edges field is missing', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { count: 0 },
    });

    const edges = await getEdges(transport, 'kg');
    expect(edges).toEqual([]);
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(getEdges(transport, 'missing')).rejects.toThrow(
      CollectionNotFoundError
    );
  });
});

describe('traverseGraph', () => {
  beforeEach(() => vi.clearAllMocks());

  const baseReply = {
    data: {
      results: [{ target_id: 20, depth: 1, path: [10, 20] }],
      has_more: true,
      stats: { visited: 5, depth_reached: 1 },
    },
  };

  it('POSTs with all snake_case defaults when optional fields omitted', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      baseReply
    );

    await traverseGraph(transport, 'kg', { source: 10 });

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/kg/graph/traverse',
      {
        source: 10,
        strategy: 'bfs',
        max_depth: 3,
        limit: 100,
        rel_types: [],
      }
    );
  });

  it('forwards explicit strategy / maxDepth / limit / relTypes', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      baseReply
    );

    await traverseGraph(transport, 'kg', {
      source: 10,
      strategy: 'dfs',
      maxDepth: 5,
      limit: 50,
      relTypes: ['KNOWS'],
    });

    const call = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]!;
    expect(call[2]).toEqual({
      source: 10,
      strategy: 'dfs',
      max_depth: 5,
      limit: 50,
      rel_types: ['KNOWS'],
    });
    // The server's TraverseRequest has no cursor; the SDK no longer sends one.
    expect(call[2]).not.toHaveProperty('cursor');
  });

  it('maps the response (targetId, depthReached), which carries no cursor', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {
        results: [{ target_id: 20, depth: 1, path: [10, 20] }],
        has_more: false,
        stats: { visited: 1, depth_reached: 1 },
      },
    });

    const result = await traverseGraph(transport, 'kg', { source: 10 });
    expect(result).toEqual({
      results: [{ targetId: 20, depth: 1, path: [10, 20] }],
      hasMore: false,
      stats: { visited: 1, depthReached: 1 },
    });
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(
      traverseGraph(transport, 'missing', { source: 10 })
    ).rejects.toThrow(CollectionNotFoundError);
  });
});

describe('getNodeDegree', () => {
  beforeEach(() => vi.clearAllMocks());

  it('GETs /graph/nodes/{id}/degree and maps snake_case', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: { in_degree: 3, out_degree: 7 },
    });

    const result = await getNodeDegree(transport, 'kg', 42);

    expect(transport.requestJson).toHaveBeenCalledWith(
      'GET',
      '/collections/kg/graph/nodes/42/degree'
    );
    expect(result).toEqual({ inDegree: 3, outDegree: 7 });
  });

  it('defaults to 0/0 when fields are missing', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {},
    });

    const result = await getNodeDegree(transport, 'kg', 42);
    expect(result).toEqual({ inDegree: 0, outDegree: 0 });
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(getNodeDegree(transport, 'missing', 1)).rejects.toThrow(
      CollectionNotFoundError
    );
  });
});

describe('createGraphCollection', () => {
  beforeEach(() => vi.clearAllMocks());

  // velesdb-server reads `graph_schema` (a core GraphSchema); the values are
  // pinned by conformance/graph_schema_wire_cases.json, which velesdb-core's
  // tests/graph_schema_wire_conformance.rs parses as GraphSchema.
  const graphSchemaFor = (mode: string): unknown => {
    const found = graphSchemaFixture.cases.find((c) => c.schema_mode === mode);
    if (found === undefined) throw new Error(`fixture case ${mode} is missing`);
    return found.graph_schema;
  };

  it('POSTs with default metric=cosine and a schemaless graph_schema', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {},
    });

    await createGraphCollection(transport, 'kg');

    expect(transport.requestJson).toHaveBeenCalledWith('POST', '/collections', {
      name: 'kg',
      collection_type: 'graph',
      dimension: undefined,
      metric: 'cosine',
      graph_schema: graphSchemaFor('schemaless'),
    });
  });

  it('forwards dimension / metric and sends schemaMode strict as a strict graph_schema', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {},
    });

    await createGraphCollection(transport, 'kg', {
      dimension: 128,
      metric: 'euclidean',
      schemaMode: 'strict',
    });

    const call = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]!;
    expect(call[2]).toEqual({
      name: 'kg',
      collection_type: 'graph',
      dimension: 128,
      metric: 'euclidean',
      graph_schema: graphSchemaFor('strict'),
    });
  });

  it('throws on error response (no resourceLabel → typed VelesError)', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      error: { code: 'VELES-010', message: 'bad config' },
    });

    await expect(createGraphCollection(transport, 'kg')).rejects.toThrow(
      /bad config/
    );
  });
});

describe('traverseParallel', () => {
  beforeEach(() => vi.clearAllMocks());

  it('POSTs to /graph/traverse/parallel with snake_case defaults', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {
        results: [],
        has_more: false,
        stats: { visited: 0, depth_reached: 0 },
      },
    });

    await traverseParallel(transport, 'kg', { sources: [1, 2, 3] });

    expect(transport.requestJson).toHaveBeenCalledWith(
      'POST',
      '/collections/kg/graph/traverse/parallel',
      {
        sources: [1, 2, 3],
        max_depth: 3,
        limit: 100,
        rel_types: [],
      }
    );
  });

  it('forwards explicit maxDepth / limit / relTypes', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: {
        results: [{ target_id: 5, depth: 2, path: [1, 5] }],
        has_more: true,
        stats: { visited: 10, depth_reached: 2 },
      },
    });

    const result = await traverseParallel(transport, 'kg', {
      sources: [1],
      maxDepth: 10,
      limit: 42,
      relTypes: ['KNOWS', 'WORKS_WITH'],
    });

    const call = (transport.requestJson as ReturnType<typeof vi.fn>).mock
      .calls[0]!;
    expect(call[2]).toEqual({
      sources: [1],
      max_depth: 10,
      limit: 42,
      rel_types: ['KNOWS', 'WORKS_WITH'],
    });
    expect(result.results).toEqual([
      { targetId: 5, depth: 2, path: [1, 5] },
    ]);
    expect(result).not.toHaveProperty('nextCursor');
    expect(result.stats.depthReached).toBe(2);
  });

  it('throws CollectionNotFoundError on VELES-002', async () => {
    const transport = buildTransport();
    (transport.requestJson as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      typedError()
    );

    await expect(
      traverseParallel(transport, 'missing', { sources: [1] })
    ).rejects.toThrow(CollectionNotFoundError);
  });
});
