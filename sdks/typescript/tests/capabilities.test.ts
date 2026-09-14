/**
 * Backend Capability Map Tests (Sprint 2 Wave 4 — #24 F-BACK-002)
 *
 * Verifies that the `capabilities()` API on VelesDB and its two
 * backends returns a frozen, backend-specific feature map with the
 * correct contrast between REST (everything on) and WASM (focused
 * subset).
 */

import { describe, it, expect } from 'vitest';
import { VelesDB, RestBackend, WasmBackend } from '../src/index';
import {
  CAPABILITY_LIST_UNIVERSES,
  REST_CAPABILITIES,
  WASM_CAPABILITIES,
  type CapabilityMap,
  type ListCapability,
} from '../src/capabilities';

// Every CapabilityMap key that must exist — any addition here must
// also be reflected in the two static maps AND in the interface.
const ALL_CAPABILITY_KEYS: readonly (keyof CapabilityMap)[] = [
  'vectorSearch',
  'textSearch',
  'hybridSearch',
  'multiQuerySearch',
  'sparseSearch',
  'namedSparseIndexes',
  'includeVectors',
  'idOnlySearch',
  'scroll',
  'graphTraversal',
  'secondaryIndexes',
  'agentMemory',
  'enableStreaming',
  'streamInsert',
  'pqTraining',
  'velesqlQuery',
  'collectionIntrospection',
  'velesqlMatchOrderBy',
  'velesqlAlterCollection',
] as const;

describe('CapabilityMap — structural contract', () => {
  it('exposes every documented key with a boolean value on REST', () => {
    for (const key of ALL_CAPABILITY_KEYS) {
      expect(typeof REST_CAPABILITIES[key]).toBe('boolean');
    }
  });

  it('exposes every documented key with a boolean value on WASM', () => {
    for (const key of ALL_CAPABILITY_KEYS) {
      expect(typeof WASM_CAPABILITIES[key]).toBe('boolean');
    }
  });

  it('both maps are frozen (immutable)', () => {
    expect(Object.isFrozen(REST_CAPABILITIES)).toBe(true);
    expect(Object.isFrozen(WASM_CAPABILITIES)).toBe(true);
  });

  it.each(Object.keys(CAPABILITY_LIST_UNIVERSES) as ListCapability[])(
    'exposes %s as a frozen string array on both maps',
    (key) => {
      for (const map of [REST_CAPABILITIES, WASM_CAPABILITIES]) {
        const list = map[key];
        expect(Array.isArray(list)).toBe(true);
        expect(Object.isFrozen(list)).toBe(true);
        for (const s of list) {
          expect(typeof s).toBe('string');
        }
      }
    }
  );
});

describe('REST_CAPABILITIES — full-feature contract', () => {
  it('advertises every feature the SDK wraps', () => {
    expect(REST_CAPABILITIES.vectorSearch).toBe(true);
    expect(REST_CAPABILITIES.textSearch).toBe(true);
    expect(REST_CAPABILITIES.hybridSearch).toBe(true);
    expect(REST_CAPABILITIES.multiQuerySearch).toBe(true);
    expect(REST_CAPABILITIES.sparseSearch).toBe(true);
    expect(REST_CAPABILITIES.scroll).toBe(true);
    expect(REST_CAPABILITIES.graphTraversal).toBe(true);
    expect(REST_CAPABILITIES.secondaryIndexes).toBe(true);
    expect(REST_CAPABILITIES.agentMemory).toBe(true);
    expect(REST_CAPABILITIES.enableStreaming).toBe(true);
    expect(REST_CAPABILITIES.streamInsert).toBe(true);
    expect(REST_CAPABILITIES.pqTraining).toBe(true);
    expect(REST_CAPABILITIES.velesqlQuery).toBe(true);
    expect(REST_CAPABILITIES.collectionIntrospection).toBe(true);
    expect(REST_CAPABILITIES.velesqlMatchOrderBy).toBe(true);
    expect(REST_CAPABILITIES.velesqlAlterCollection).toBe(true);
    expect(REST_CAPABILITIES.namedSparseIndexes).toBe(true);
    expect(REST_CAPABILITIES.includeVectors).toBe(true);
    expect(REST_CAPABILITIES.idOnlySearch).toBe(true);
    // Every filter-taking entry point but /search/multi/ids, whose server
    // endpoint refuses a filter.
    expect([...REST_CAPABILITIES.filteredSearch]).toEqual([
      'search',
      'sparseSearch',
      'searchBatch',
      'searchIds',
      'textSearch',
      'hybridSearch',
      'multiQuerySearch',
      'sparseSearchNamed',
      'scroll',
    ]);
    for (const key of ['storageModes', 'collectionTypes', 'collectionConfig', 'queryOptions'] as const) {
      expect(REST_CAPABILITIES[key]).toEqual(CAPABILITY_LIST_UNIVERSES[key]);
    }
    expect([...REST_CAPABILITIES.multiQueryFusionParams]).toEqual([
      'k',
      'avgWeight',
      'maxWeight',
      'hitWeight',
      'denseWeight',
      'sparseWeight',
    ]);
    expect([...REST_CAPABILITIES.velesqlFusionStrategies]).toEqual([
      'rrf',
      'weighted',
      'maximum',
      'rsf',
      'average',
    ]);
  });
});

describe('WASM_CAPABILITIES — focused subset', () => {
  it('supports the core search paths', () => {
    expect(WASM_CAPABILITIES.vectorSearch).toBe(true);
    expect(WASM_CAPABILITIES.textSearch).toBe(true);
    expect(WASM_CAPABILITIES.hybridSearch).toBe(true);
    expect(WASM_CAPABILITIES.multiQuerySearch).toBe(true);
    // Sparse search runs on WASM; the map used to deny it (#2095).
    expect(WASM_CAPABILITIES.sparseSearch).toBe(true);
  });

  it('filters dense search only, and lists the fusion params its binding takes', () => {
    expect([...WASM_CAPABILITIES.filteredSearch]).toEqual(['search', 'searchBatch']);
    expect([...WASM_CAPABILITIES.storageModes]).toEqual(['full', 'sq8', 'binary']);
    expect([...WASM_CAPABILITIES.collectionTypes]).toEqual(['vector']);
    expect([...WASM_CAPABILITIES.collectionConfig]).toEqual([
      'dimension',
      'metric',
      'storageMode',
      'collectionType',
      'description',
    ]);
    expect([...WASM_CAPABILITIES.queryOptions]).toEqual([]);
    expect([...WASM_CAPABILITIES.multiQueryFusionParams]).toEqual([
      'k',
      'avgWeight',
      'maxWeight',
      'hitWeight',
    ]);
    expect(WASM_CAPABILITIES.namedSparseIndexes).toBe(false);
    expect(WASM_CAPABILITIES.includeVectors).toBe(false);
    expect(WASM_CAPABILITIES.idOnlySearch).toBe(false);
  });

  it('does NOT support persistent / graph / streaming / VelesQL features', () => {
    expect(WASM_CAPABILITIES.scroll).toBe(false);
    expect(WASM_CAPABILITIES.graphTraversal).toBe(false);
    expect(WASM_CAPABILITIES.secondaryIndexes).toBe(false);
    expect(WASM_CAPABILITIES.agentMemory).toBe(false);
    expect(WASM_CAPABILITIES.enableStreaming).toBe(false);
    expect(WASM_CAPABILITIES.streamInsert).toBe(false);
    expect(WASM_CAPABILITIES.pqTraining).toBe(false);
    // query() only executes pure top-k NEAR statements — full VelesQL is
    // not advertised so callers route VelesQL workloads to REST upfront.
    expect(WASM_CAPABILITIES.velesqlQuery).toBe(false);
    expect(WASM_CAPABILITIES.collectionIntrospection).toBe(false);
    // VelesQL sub-capabilities are all off when velesqlQuery is false.
    expect(WASM_CAPABILITIES.velesqlMatchOrderBy).toBe(false);
    expect(WASM_CAPABILITIES.velesqlAlterCollection).toBe(false);
    expect(WASM_CAPABILITIES.velesqlFusionStrategies).toHaveLength(0);
  });
});

describe('RestBackend.capabilities()', () => {
  it('returns the frozen REST_CAPABILITIES singleton', () => {
    const backend = new RestBackend('http://localhost:8080');
    expect(backend.capabilities()).toBe(REST_CAPABILITIES);
  });
});

describe('WasmBackend.capabilities()', () => {
  it('returns the frozen WASM_CAPABILITIES singleton', () => {
    const backend = new WasmBackend();
    expect(backend.capabilities()).toBe(WASM_CAPABILITIES);
  });
});

describe('VelesDB.capabilities() — client facade', () => {
  it('REST backend client surfaces REST_CAPABILITIES', () => {
    const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
    expect(db.capabilities()).toBe(REST_CAPABILITIES);
  });

  it('WASM backend client surfaces WASM_CAPABILITIES', () => {
    const db = new VelesDB({ backend: 'wasm' });
    expect(db.capabilities()).toBe(WASM_CAPABILITIES);
  });

  it('client.capabilities() enables caller-side graceful degradation', () => {
    const wasmDb = new VelesDB({ backend: 'wasm' });
    // Real-world pattern: feature-flag a UI branch on the capability
    // instead of catching NOT_SUPPORTED at the call site.
    const caps = wasmDb.capabilities();
    expect(caps.graphTraversal).toBe(false);
    expect(caps.vectorSearch).toBe(true);
  });
});
