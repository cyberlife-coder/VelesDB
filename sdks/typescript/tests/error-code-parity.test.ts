/**
 * REST ↔ WASM error-code parity (#2307)
 *
 * The same bad input must earn the same error code whichever backend the
 * caller runs on. Two holes broke that: a type-level bad value over REST
 * (negative `top_k`, a string or fractional `rrf_k`, a `null` weight, a
 * non-string strategy) fails axum's body deserialization, which answers a
 * plain-text `422` carrying no JSON code — unmapped, the SDK surfaced
 * `UNKNOWN_ERROR` where WASM gives `BAD_REQUEST`. And `multiQuerySearch([])`
 * was `VALIDATION_ERROR` at the client layer while the WASM backend answers
 * `BAD_REQUEST` for the same input.
 *
 * The WASM backend is exercised through the mocked `@wiscale/velesdb-wasm`
 * module (as `wasm-facade.test.ts` does); REST through mocked `fetch`.
 * Server answers are simulated faithfully: `422` rejects `response.json()`
 * the way a plain-text body does, and a `400` refusal carries the server's
 * own `ErrorResponse` shape.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { VelesDB } from '../src/client';
import { RestBackend } from '../src/backends/rest';
import { WasmBackend } from '../src/backends/wasm';
import { VelesDBError } from '../src/types';

// Mock WASM module — the minimal surface the backend touches on the paths
// under test (collection creation, then a refused multi-query search that
// must never reach the binding).
class MockVectorStore {
  insert = vi.fn();
  insert_with_payload = vi.fn();
  free = vi.fn();
  search = vi.fn(() => []);
  search_with_quality = vi.fn(() => []);
  search_with_filter = vi.fn(() => []);
  multi_query_search = vi.fn(() => []);
  query = vi.fn(() => []);
  len = 0;
  is_empty = true;
  constructor(public dimension: number, _metric: string) {}

  static new_with_mode(dimension: number, metric: string, _mode: string): MockVectorStore {
    return new MockVectorStore(dimension, metric);
  }

  static new_metadata_only(): MockVectorStore {
    return new MockVectorStore(0, 'cosine');
  }
}

const mockWasmModule = {
  default: vi.fn(() => Promise.resolve()),
  VectorStore: MockVectorStore,
  hybrid_search_fuse: vi.fn(() => []),
};

vi.mock('@wiscale/velesdb-wasm', () => mockWasmModule);

const mockFetch = vi.fn();
global.fetch = mockFetch;

/** The response a server gives when axum's `Json` extractor rejects the body. */
const DESERIALIZATION_422 = {
  ok: false,
  status: 422,
  json: () => Promise.reject(new SyntaxError('Unexpected token < in JSON')),
};

const HEALTH_OK = {
  ok: true,
  json: () => Promise.resolve({ status: 'ok' }),
};

function settle(promise: Promise<unknown>): Promise<unknown> {
  return promise.then(
    () => 'resolved',
    (error: unknown) => error
  );
}

describe('REST ↔ WASM error-code parity (#2307)', () => {
  let dbRest: VelesDB;
  let dbWasm: VelesDB;
  let restBackend: RestBackend;
  let wasmBackend: WasmBackend;

  beforeEach(async () => {
    vi.clearAllMocks();

    mockFetch.mockResolvedValueOnce(HEALTH_OK);
    dbRest = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
    await dbRest.init();

    mockFetch.mockResolvedValueOnce(HEALTH_OK);
    restBackend = new RestBackend('http://localhost:8080');
    await restBackend.init();

    dbWasm = new VelesDB({ backend: 'wasm' });
    await dbWasm.init();
    await dbWasm.createCollection('docs', { dimension: 2, metric: 'cosine' });

    wasmBackend = new WasmBackend();
    await wasmBackend.init();
    await wasmBackend.createCollection('docs', { dimension: 2, metric: 'cosine' });

    vi.clearAllMocks();
  });

  describe('multiQuerySearch — a value the request schema refuses is BAD_REQUEST on both', () => {
    it.each([
      { label: 'k: -1', options: { k: -1 } },
      { label: 'k: 1.5', options: { k: 1.5 } },
      { label: 'k: "x"', options: { k: 'x' } },
      { label: 'k: NaN', options: { k: Number.NaN } },
      { label: 'fusion: 5', options: { fusion: 5 } },
      { label: 'fusion: { name: "rrf" }', options: { fusion: { name: 'rrf' } } },
      { label: 'fusionParams.k: "abc"', options: { fusionParams: { k: 'abc' } } },
      { label: 'fusionParams.k: 1.5', options: { fusionParams: { k: 1.5 } } },
      { label: 'fusionParams.k: -1', options: { fusionParams: { k: -1 } } },
      {
        label: 'fusionParams.avgWeight: "0.5"',
        options: { fusion: 'weighted', fusionParams: { avgWeight: '0.5' } },
      },
      {
        label: 'fusionParams.avgWeight: null',
        options: { fusion: 'weighted', fusionParams: { avgWeight: null } },
      },
      {
        label: 'fusionParams.avgWeight: NaN',
        options: { fusion: 'weighted', fusionParams: { avgWeight: Number.NaN } },
      },
      {
        label: 'fusionParams.branchWeights: "x"',
        options: { fusionParams: { branchWeights: 'x' } },
      },
      {
        label: 'fusionParams.branchWeights: [0.5, NaN]',
        options: { fusionParams: { branchWeights: [0.5, Number.NaN] } },
      },
      {
        label: 'weighted_rrf without branchWeights',
        options: { fusion: 'weighted_rrf' },
      },
      {
        label: 'weighted_rrf with two weights for one vector',
        options: { fusion: 'weighted_rrf', fusionParams: { branchWeights: [0.5, 0.5] } },
      },
    ])('$label gives BAD_REQUEST on REST (422) and on WASM (local), never UNKNOWN_ERROR', async ({ options }) => {
      // REST: axum refuses the body before the handler runs → plain-text 422.
      mockFetch.mockResolvedValueOnce(DESERIALIZATION_422);
      const restError = await settle(
        dbRest.multiQuerySearch('docs', [[0.1, 0.2]], options as never)
      );
      const wasmError = await settle(
        dbWasm.multiQuerySearch('docs', [[0.1, 0.2]], options as never)
      );

      expect(restError).toBeInstanceOf(VelesDBError);
      expect((restError as VelesDBError).code).toBe('BAD_REQUEST');
      expect(wasmError).toBeInstanceOf(VelesDBError);
      expect((wasmError as VelesDBError).code).toBe('BAD_REQUEST');
    });

    it('an unknown strategy name is BAD_REQUEST on both: REST 400 code:null, WASM local', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        status: 400,
        json: () => Promise.resolve({ error: 'unknown fusion strategy', code: null }),
      });

      const restError = await settle(
        dbRest.multiQuerySearch('docs', [[0.1, 0.2]], { fusion: 'bogus' as never })
      );
      const wasmError = await settle(
        dbWasm.multiQuerySearch('docs', [[0.1, 0.2]], { fusion: 'bogus' as never })
      );

      expect((restError as VelesDBError).code).toBe('BAD_REQUEST');
      expect((wasmError as VelesDBError).code).toBe('BAD_REQUEST');
    });
  });

  describe('multiQuerySearch — a vector count core refuses is BAD_REQUEST everywhere', () => {
    it.each([
      { label: '[]', vectors: [] as Array<number[]> },
      { label: 'null', vectors: null as never },
      { label: '11 vectors', vectors: Array.from({ length: 11 }, () => [0.1, 0.2]) },
    ])('$label gives BAD_REQUEST through the client on both backends', async ({ vectors }) => {
      const restError = await settle(dbRest.multiQuerySearch('docs', vectors));
      const wasmError = await settle(dbWasm.multiQuerySearch('docs', vectors));

      expect(restError).toBeInstanceOf(VelesDBError);
      expect((restError as VelesDBError).code).toBe('BAD_REQUEST');
      expect(wasmError).toBeInstanceOf(VelesDBError);
      expect((wasmError as VelesDBError).code).toBe('BAD_REQUEST');
      // Nothing was sent: the REST call count must still be zero and the
      // binding's multi_query_search was never invoked.
      expect(mockFetch).not.toHaveBeenCalled();
    });

    it.each([
      { label: '[]', vectors: [] as Array<number[]> },
      { label: 'null', vectors: null as never },
      { label: '11 vectors', vectors: Array.from({ length: 11 }, () => [0.1, 0.2]) },
    ])('$label gives BAD_REQUEST on a direct backend call too', async ({ vectors }) => {
      const restError = await settle(restBackend.multiQuerySearch('docs', vectors));
      const wasmError = await settle(wasmBackend.multiQuerySearch('docs', vectors));

      expect((restError as VelesDBError).code).toBe('BAD_REQUEST');
      expect((wasmError as VelesDBError).code).toBe('BAD_REQUEST');
      expect(mockFetch).not.toHaveBeenCalled();
    });
  });

  describe('query — a LIMIT value core refuses is a caller error on both', () => {
    // REST surfaces the server's own typed code (`VELES-010` QueryError) for
    // a VelesQL the parser refuses; WASM names the same refusal with the
    // SDK-local `BAD_REQUEST`. The parity pinned here is that neither side
    // calls it NOT_SUPPORTED or UNKNOWN_ERROR — it is the caller's input that
    // is wrong, not a capability gap.
    it.each(['-1', '+5', '1e3'])(
      'LIMIT %s: REST → VELES-010 (server code), WASM → BAD_REQUEST',
      async (token) => {
        const queryString = `SELECT * FROM docs WHERE vector NEAR $v LIMIT ${token}`;

        mockFetch.mockResolvedValueOnce({
          ok: false,
          status: 400,
          json: () => Promise.resolve({ error: `Invalid LIMIT value '${token}'`, code: 'VELES-010' }),
        });
        const restError = await settle(
          dbRest.query('docs', queryString, { v: [0.1, 0.2] })
        );
        const wasmError = await settle(
          dbWasm.query('docs', queryString, { v: [0.1, 0.2] })
        );

        expect((restError as VelesDBError).code).toBe('VELES-010');
        expect(wasmError).toBeInstanceOf(VelesDBError);
        expect((wasmError as VelesDBError).code).toBe('BAD_REQUEST');
      }
    );
  });
});
