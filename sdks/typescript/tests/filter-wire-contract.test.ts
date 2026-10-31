/**
 * Filter wire contract shared with velesdb-core.
 *
 * `conformance/filter_wire_cases.json` is also read by
 * `crates/velesdb-core/tests/filter_wire_conformance.rs`, which proves every
 * `valid` case deserializes as `velesdb_core::Filter` (and selects exactly its
 * sample payloads) and every `invalid` case is refused. This half proves the
 * SDK sends those exact shapes, so an agent-memory call cannot drift back to a
 * flat `{ _memory_type: ... }` map the server answers with a 400.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { RestBackend } from '../src/backends/rest';
import { normalizeFilter } from '../src/filter';
import { ValidationError } from '../src/types';

interface WireCase {
  id: string;
  filter: Record<string, unknown>;
}

interface WireFixture {
  valid: WireCase[];
  invalid: WireCase[];
}

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const fixture = JSON.parse(
  readFileSync(path.resolve(__dirname, '../../../conformance/filter_wire_cases.json'), 'utf8')
) as WireFixture;

function wireFilter(id: string): Record<string, unknown> {
  const found = fixture.valid.find((c) => c.id === id);
  if (found === undefined) throw new Error(`fixture case ${id} is missing`);
  return found.filter;
}

const mockFetch = vi.fn();
global.fetch = mockFetch;

function respond(data: unknown): void {
  mockFetch.mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(data) });
}

/** Body of the only request the call under test sent. */
function sentBody(): Record<string, unknown> {
  expect(mockFetch).toHaveBeenCalledTimes(1);
  return JSON.parse(mockFetch.mock.calls[0][1].body) as Record<string, unknown>;
}

describe('agent-memory REST filters match the core wire fixture', () => {
  let backend: RestBackend;

  beforeEach(async () => {
    vi.clearAllMocks();
    backend = new RestBackend('http://localhost:8080');
    respond({ status: 'ok' });
    await backend.init();
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it.each([
    ['searchSemanticMemory', 'agent_memory_semantic'],
    ['recallEpisodicEvents', 'agent_memory_episodic'],
    ['matchProceduralPatterns', 'agent_memory_procedural'],
  ] as const)('%s sends the %s filter to /search', async (method, caseId) => {
    respond({ results: [] });

    await backend[method]('mem', [0.1, 0.2], 3);

    expect(mockFetch.mock.calls[0][0]).toBe('http://localhost:8080/collections/mem/search');
    expect(sentBody().filter).toEqual(wireFilter(caseId));
  });

  it.each([
    ['recallRecentEvents without since', () => backend.recallRecentEvents('mem'), 'agent_memory_episodic'],
    ['recallRecentEvents since 200', () => backend.recallRecentEvents('mem', 200), 'agent_memory_episodic_since_200'],
    ['recallOlderThanEvents before 200', () => backend.recallOlderThanEvents('mem', 200), 'agent_memory_episodic_before_200'],
  ])('%s sends its filter to /points/scroll', async (_name, call, caseId) => {
    respond({ points: [], next_cursor: null });

    await call();

    expect(mockFetch.mock.calls[0][0]).toBe('http://localhost:8080/collections/mem/points/scroll');
    expect(sentBody().filter).toEqual(wireFilter(caseId));
  });
});

describe('normalizeFilter agrees with core on the fixture', () => {
  it.each(fixture.valid.map((c) => [c.id, c.filter] as const))(
    'forwards valid case %s verbatim',
    (_id, filter) => {
      expect(normalizeFilter(filter)).toBe(filter);
    }
  );

  it.each(fixture.invalid.map((c) => [c.id, c.filter] as const))(
    'refuses invalid case %s',
    (_id, filter) => {
      expect(() => normalizeFilter(filter)).toThrow(ValidationError);
    }
  );
});
