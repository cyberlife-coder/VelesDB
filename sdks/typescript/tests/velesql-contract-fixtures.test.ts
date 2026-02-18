import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { RestBackend } from '../src/backends/rest';
import { VelesDBError } from '../src/types';

interface ConformanceCase {
  id: string;
  runtimes: string[];
  body: {
    query: string;
    params?: Record<string, unknown>;
    collection?: string;
  };
  expected_status: number;
  expected_error_code?: string;
}

interface ConformanceFixture {
  contract_version: string;
  cases: ConformanceCase[];
}

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const fixturePath = path.resolve(__dirname, '../../../conformance/velesql_contract_cases.json');
const fixture = JSON.parse(readFileSync(fixturePath, 'utf8')) as ConformanceFixture;

const mockFetch = vi.fn();
global.fetch = mockFetch;

describe('VelesQL contract fixtures (TypeScript runtime)', () => {
  let backend: RestBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = new RestBackend('http://localhost:8080');

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

  it('maps nested server error payloads to VelesDBError.code from fixtures', async () => {
    const errorCases = fixture.cases.filter(
      (c) => c.runtimes.includes('typescript') && c.expected_error_code
    );

    for (const c of errorCases) {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        status: c.expected_status,
        json: () =>
          Promise.resolve({
            error: {
              code: c.expected_error_code,
              message: `fixture error for ${c.id}`,
            },
          }),
      });

      await expect(
        backend.query(
          c.body.collection ?? 'docs_conformance',
          c.body.query,
          c.body.params ?? {}
        )
      ).rejects.toMatchObject({
        code: c.expected_error_code,
      });
    }
  });

  it('keeps query forwarding compatible for top-level MATCH fixture case', async () => {
    const matchCase = fixture.cases.find((c) => c.id === 'C005');
    expect(matchCase).toBeDefined();

    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () =>
        Promise.resolve({
          results: [],
          timing_ms: 0.3,
          rows_returned: 0,
          meta: {
            velesql_contract_version: fixture.contract_version,
          },
        }),
    });

    await backend.query(
      matchCase!.body.collection ?? 'docs_conformance',
      matchCase!.body.query,
      matchCase!.body.params ?? {}
    );

    expect(mockFetch).toHaveBeenCalledWith(
      'http://localhost:8080/query',
      expect.objectContaining({
        method: 'POST',
        body: JSON.stringify({
          query: matchCase!.body.query,
          params: matchCase!.body.params ?? {},
          collection: matchCase!.body.collection ?? 'docs_conformance',
        }),
      })
    );
  });
});
