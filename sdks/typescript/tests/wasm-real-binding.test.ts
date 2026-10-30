/**
 * The WASM backend against a REAL velesdb-wasm build, not a fake (#2332).
 *
 * Every other WASM test stubs the binding with a store that returns plain
 * objects. The real binding returned ES2015 `Map` records from
 * `hybrid_search`, `text_search`, `search_with_filter` and `get` (velesdb-wasm
 * 6.x and 7.x), on which `r.id` is `undefined`, so the SDK threw on every
 * query that matched a row, and no fake could show it. This file runs the
 * SDK's public API over the binding itself and checks what comes back.
 *
 * It runs when `VELESDB_WASM_PKG` names a `wasm-pack build --target web`
 * output directory (CI builds one from the workspace, see `wasm-check` in
 * `.github/workflows/ci.yml`):
 *
 *   wasm-pack build crates/velesdb-wasm --target web --dev --out-dir <dir>
 *   VELESDB_WASM_PKG=<dir> npx vitest run tests/wasm-real-binding.test.ts
 *
 * Without it the suite is skipped: the binding the lockfile installs from npm
 * predates the fix, and the default `npm test` must not depend on a build.
 */
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';
import { VelesDB } from '../src/client';

const PKG = process.env.VELESDB_WASM_PKG;

vi.mock('@wiscale/velesdb-wasm', async () => {
  const dir = process.env.VELESDB_WASM_PKG as string;
  return import(pathToFileURL(join(dir, 'velesdb_wasm.js')).href);
});

vi.mock('../src/backends/wasm-node-loader', async () => ({
  isNodeRuntime: () => true,
  loadWasmBytesNode: async () =>
    readFile(join(process.env.VELESDB_WASM_PKG as string, 'velesdb_wasm_bg.wasm')),
}));

const RUST = { title: 'hello rust', category: 'lang' };
const NOTES = { title: 'release notes', category: 'docs' };

describe.skipIf(!PKG)('WASM backend over the real velesdb-wasm build (#2332)', () => {
  const db = new VelesDB({ backend: 'wasm' });

  beforeAll(async () => {
    await db.init();
    await db.createCollection('docs', { dimension: 2, metric: 'cosine' });
    await db.upsert('docs', { id: 1, vector: [1, 0], payload: RUST });
    await db.upsert('docs', { id: 2, vector: [0, 1], payload: NOTES });
    await db.upsert('docs', { id: 3, vector: [0.6, 0.8] });
  });

  afterAll(async () => {
    await db.close();
  });

  it('search returns ids, scores and payloads', async () => {
    const results = await db.search('docs', [1, 0], { k: 3 });

    expect(results.map(r => r.id)).toEqual(['1', '3', '2']);
    expect(results[0].score).toBeCloseTo(1, 5);
    expect(results[1].score).toBeCloseTo(0.6, 5);
    expect(results[0].payload).toEqual(RUST);
    expect(results[1].payload).toBeUndefined();
  });

  it('filtered search returns plain-object records', async () => {
    const results = await db.search('docs', [1, 0], {
      k: 3,
      filter: { condition: { type: 'eq', field: 'category', value: 'docs' } },
    });

    expect(results).toHaveLength(1);
    expect(results[0].id).toBe('2');
    expect(results[0].score).toBeCloseTo(0, 5);
    expect(results[0].payload).toEqual(NOTES);
  });

  it('textSearch returns ids, a score per match and payloads', async () => {
    const results = await db.textSearch('docs', 'rust', { k: 3 });

    expect(results).toEqual([{ id: '1', score: 1, payload: RUST }]);
  });

  it('hybridSearch returns ids, fused scores and payloads', async () => {
    const results = await db.hybridSearch('docs', [0, 1], 'rust', { k: 3, vectorWeight: 0.7 });

    // 0.7 * cosine + 0.3 * (1 if the text matches): 0.7, 0.56, 0.3.
    expect(results.map(r => r.id)).toEqual(['2', '3', '1']);
    expect(results.map(r => r.score)).toEqual([
      expect.closeTo(0.7, 5),
      expect.closeTo(0.56, 5),
      expect.closeTo(0.3, 5),
    ]);
    expect(results[0].payload).toEqual(NOTES);
    expect(results[1].payload).toBeUndefined();
    expect(results[2].payload).toEqual(RUST);
  });

  it('multiQuerySearch returns fused ids, scores and payloads', async () => {
    // RRF over the two rankings [1, 3, 2] and [3, 2, 1] puts 3 first, then 1.
    const results = await db.multiQuerySearch('docs', [[1, 0], [0.6, 0.8]], { k: 2 });

    expect(results.map(r => r.id)).toEqual(['3', '1']);
    expect(results[0].score).toBeGreaterThan(results[1].score);
    expect(results[1].score).toBeGreaterThan(0);
    expect(results[1].payload).toEqual(RUST);
  });

  it('get returns the point with its payload', async () => {
    const point = await db.get('docs', 1);

    expect(point).toEqual({ id: '1', vector: [1, 0], payload: RUST });
  });
});
