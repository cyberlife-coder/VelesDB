/**
 * WASM Wave 4 Stubs Tests (S4-07)
 *
 * Verifies that the 12 server-only endpoints exposed as WASM stubs throw
 * VelesDBError with code NOT_SUPPORTED and a message mentioning the
 * feature name + REST backend.
 *
 * IMPORTANT: each stub is written as
 *   export function X(): Promise<T> { return Promise.resolve(wasmNotSupported(...)); }
 *
 * `wasmNotSupported()` throws SYNCHRONOUSLY during argument evaluation of
 * `Promise.resolve(...)` — the promise is never actually constructed.
 * Therefore callers observe a synchronous throw, not a rejected promise,
 * even though the return type is `Promise<T>`. Tests reflect that with
 * `expect(fn).toThrow(...)` rather than `rejects.toThrow(...)`.
 */

import { describe, expect, it } from 'vitest';
import * as wasmWaveFour from '../src/backends/wasm-wave4-stubs';
import { VelesDBError } from '../src/types';
import type {
  AggregateQueryOptions,
  GetNodeEdgesOptions,
  GraphSearchRequest,
  GuardRailsUpdateRequest,
  MatchQueryOptions,
} from '../src/types';

interface StubCase {
  name: string;
  call: () => Promise<unknown>;
  feature: string;
}

const stubCases: StubCase[] = [
  {
    name: 'wasmRebuildIndex',
    call: () => wasmWaveFour.wasmRebuildIndex('c'),
    feature: 'Index rebuild',
  },
  {
    name: 'wasmGetGuardrails',
    call: () => wasmWaveFour.wasmGetGuardrails(),
    feature: 'Guardrails',
  },
  {
    name: 'wasmUpdateGuardrails',
    call: () =>
      wasmWaveFour.wasmUpdateGuardrails({} as GuardRailsUpdateRequest),
    feature: 'Guardrails',
  },
  {
    name: 'wasmAggregate',
    call: () =>
      wasmWaveFour.wasmAggregate('q', {}, {} as AggregateQueryOptions),
    feature: 'Aggregate queries',
  },
  {
    name: 'wasmMatchQuery',
    call: () =>
      wasmWaveFour.wasmMatchQuery('c', 'q', {}, {} as MatchQueryOptions),
    feature: 'MATCH queries',
  },
  {
    name: 'wasmRemoveEdge',
    call: () => wasmWaveFour.wasmRemoveEdge('c', 1),
    feature: 'Graph edge removal',
  },
  {
    name: 'wasmGetEdgeCount',
    call: () => wasmWaveFour.wasmGetEdgeCount('c'),
    feature: 'Graph edge count',
  },
  {
    name: 'wasmListNodes',
    call: () => wasmWaveFour.wasmListNodes('c'),
    feature: 'Graph list nodes',
  },
  {
    name: 'wasmGetNodeEdges',
    call: () =>
      wasmWaveFour.wasmGetNodeEdges('c', 1, {} as GetNodeEdgesOptions),
    feature: 'Graph node edges',
  },
  {
    name: 'wasmGetNodePayload',
    call: () => wasmWaveFour.wasmGetNodePayload('c', 1),
    feature: 'Graph node payload (read)',
  },
  {
    name: 'wasmUpsertNodePayload',
    call: () => wasmWaveFour.wasmUpsertNodePayload('c', 1, { k: 'v' }),
    feature: 'Graph node payload (upsert)',
  },
  {
    name: 'wasmGraphSearch',
    call: () => wasmWaveFour.wasmGraphSearch('c', {} as GraphSearchRequest),
    feature: 'Graph search',
  },
];

describe.each(stubCases)('$name', ({ call, feature }) => {
  it('throws VelesDBError synchronously with NOT_SUPPORTED code and feature name', () => {
    expect(call).toThrow(VelesDBError);
    expect(call).toThrow(feature);

    try {
      void call();
    } catch (e) {
      expect(e).toBeInstanceOf(VelesDBError);
      expect((e as VelesDBError).code).toBe('NOT_SUPPORTED');
      expect((e as VelesDBError).message).toMatch(/REST backend/);
    }
  });
});

describe('WASM Wave 4 stubs — coverage guard', () => {
  it('exports exactly 12 stubs', () => {
    expect(stubCases).toHaveLength(12);
  });
});
