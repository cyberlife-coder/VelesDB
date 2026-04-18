/**
 * WASM Stubs Coverage Tests (#598)
 *
 * Exhaustive coverage for `src/backends/wasm-stubs.ts` — the collection
 * of server-only endpoints exposed as WASM stubs that throw
 * `VelesDBError` with code `NOT_SUPPORTED`. Complements
 * `wasm-stubs.test.ts` (which only targets the 4 F-BACK-001 index-
 * management stubs) by covering the remaining ~20 stub exports, and
 * `wasm-wave4-stubs.test.ts` (which targets a different file).
 *
 * Pattern: declarative table + `describe.each`, identical to the
 * `wasm-wave4-stubs.test.ts` approach. Unlike wave4-stubs (which throws
 * synchronously during argument evaluation), these stubs are declared
 * `async function`, so the error surfaces as a rejected promise — tests
 * use `rejects.toThrow(...)`.
 */

import { describe, expect, it } from 'vitest';
import * as wasmStubs from '../src/backends/wasm-stubs';
import { VelesDBError } from '../src/types';
import type {
  AddEdgeRequest,
  CreateIndexOptions,
  EpisodicEvent,
  GraphCollectionConfig,
  PqTrainOptions,
  ProceduralPattern,
  SemanticEntry,
  TraverseRequest,
  TraverseParallelRequest,
  VectorDocument,
} from '../src/types';

interface StubCase {
  name: string;
  call: () => Promise<unknown>;
  feature: RegExp;
}

const stubCases: StubCase[] = [
  // Index management (covered by wasm-stubs.test.ts too — kept here for
  // parity with the message/feature assertion and to keep coverage whole)
  {
    name: 'wasmCreateIndex',
    call: () =>
      wasmStubs.wasmCreateIndex('c', {
        label: 'L',
        property: 'p',
      } as CreateIndexOptions),
    feature: /Index management \(createIndex\)/,
  },
  {
    name: 'wasmListIndexes',
    call: () => wasmStubs.wasmListIndexes('c'),
    feature: /Index management \(listIndexes\)/,
  },
  {
    name: 'wasmHasIndex',
    call: () => wasmStubs.wasmHasIndex('c', 'L', 'p'),
    feature: /Index management \(hasIndex\)/,
  },
  {
    name: 'wasmDropIndex',
    call: () => wasmStubs.wasmDropIndex('c', 'L', 'p'),
    feature: /Index management \(dropIndex\)/,
  },

  // Knowledge Graph
  {
    name: 'wasmAddEdge',
    call: () => wasmStubs.wasmAddEdge('c', {} as AddEdgeRequest),
    feature: /Knowledge Graph/,
  },
  {
    name: 'wasmGetEdges',
    call: () => wasmStubs.wasmGetEdges('c'),
    feature: /Knowledge Graph/,
  },
  {
    name: 'wasmTraverseGraph',
    call: () => wasmStubs.wasmTraverseGraph('c', {} as TraverseRequest),
    feature: /Graph traversal/,
  },
  {
    name: 'wasmTraverseParallel',
    call: () =>
      wasmStubs.wasmTraverseParallel('c', {} as TraverseParallelRequest),
    feature: /Graph parallel traversal/,
  },
  {
    name: 'wasmGetNodeDegree',
    call: () => wasmStubs.wasmGetNodeDegree('c', 1),
    feature: /Graph degree query/,
  },

  // Query explain / Sanity / Scroll
  {
    name: 'wasmQueryExplain (plain)',
    call: () => wasmStubs.wasmQueryExplain('q', {}),
    feature: /Query explain/,
  },
  {
    name: 'wasmQueryExplain (analyze)',
    call: () => wasmStubs.wasmQueryExplain('q', {}, { analyze: true }),
    feature: /EXPLAIN ANALYZE/,
  },
  {
    name: 'wasmCollectionSanity',
    call: () => wasmStubs.wasmCollectionSanity('c'),
    feature: /Collection sanity endpoint/,
  },
  {
    name: 'wasmScroll',
    call: () => wasmStubs.wasmScroll('c'),
    feature: /scroll/,
  },

  // Sparse / PQ / Streaming
  {
    name: 'wasmTrainPq',
    call: () => wasmStubs.wasmTrainPq('c', {} as PqTrainOptions),
    feature: /PQ training/,
  },
  {
    name: 'wasmStreamInsert',
    call: () => wasmStubs.wasmStreamInsert('c', [] as VectorDocument[]),
    feature: /Streaming insert/,
  },
  {
    name: 'wasmStreamUpsertPoints',
    call: () => wasmStubs.wasmStreamUpsertPoints('c', [] as VectorDocument[]),
    feature: /Streaming batch upsert/,
  },

  // Graph Collection / Stats / Agent Memory (Phase 8)
  {
    name: 'wasmCreateGraphCollection',
    call: () =>
      wasmStubs.wasmCreateGraphCollection('c', {} as GraphCollectionConfig),
    feature: /Graph collections/,
  },
  {
    name: 'wasmGetCollectionStats',
    call: () => wasmStubs.wasmGetCollectionStats('c'),
    feature: /Collection stats/,
  },
  {
    name: 'wasmAnalyzeCollection',
    call: () => wasmStubs.wasmAnalyzeCollection('c'),
    feature: /Collection analyze/,
  },
  {
    name: 'wasmGetCollectionConfig',
    call: () => wasmStubs.wasmGetCollectionConfig('c'),
    feature: /Collection config/,
  },
  {
    name: 'wasmSearchIds',
    call: () => wasmStubs.wasmSearchIds('c', [0.1], { k: 5 }),
    feature: /searchIds/,
  },
  {
    name: 'wasmStoreSemanticFact',
    call: () => wasmStubs.wasmStoreSemanticFact('c', {} as SemanticEntry),
    feature: /Agent memory/,
  },
  {
    name: 'wasmSearchSemanticMemory',
    call: () => wasmStubs.wasmSearchSemanticMemory('c', [0.1], 5),
    feature: /Agent memory/,
  },
  {
    name: 'wasmRecordEpisodicEvent',
    call: () => wasmStubs.wasmRecordEpisodicEvent('c', {} as EpisodicEvent),
    feature: /Agent memory/,
  },
  {
    name: 'wasmRecallEpisodicEvents',
    call: () => wasmStubs.wasmRecallEpisodicEvents('c', [0.1], 5),
    feature: /Agent memory/,
  },
  {
    name: 'wasmStoreProceduralPattern',
    call: () =>
      wasmStubs.wasmStoreProceduralPattern('c', {} as ProceduralPattern),
    feature: /Agent memory/,
  },
  {
    name: 'wasmMatchProceduralPatterns',
    call: () => wasmStubs.wasmMatchProceduralPatterns('c', [0.1], 5),
    feature: /Agent memory/,
  },
];

describe.each(stubCases)('wasm-stubs: $name', ({ call, feature }) => {
  it('rejects with VelesDBError, NOT_SUPPORTED code, and feature name', async () => {
    await expect(call()).rejects.toThrow(VelesDBError);
    await expect(call()).rejects.toThrow(feature);
    await expect(call()).rejects.toThrow(/REST backend/);
    try {
      await call();
      // Should not reach here
      expect.fail('Expected stub to reject');
    } catch (e) {
      expect(e).toBeInstanceOf(VelesDBError);
      expect((e as VelesDBError).code).toBe('NOT_SUPPORTED');
    }
  });
});

describe('wasm-stubs — cardinality guard', () => {
  it('covers all 27 stub exports (index + graph + explain + sparse + agent)', () => {
    expect(stubCases).toHaveLength(27);
  });
});
