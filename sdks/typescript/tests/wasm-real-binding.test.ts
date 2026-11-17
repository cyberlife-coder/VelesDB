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
    expect(results[0]!.score).toBeCloseTo(1, 5);
    expect(results[1]!.score).toBeCloseTo(0.6, 5);
    expect(results[0]!.payload).toEqual(RUST);
    expect(results[1]!.payload).toBeUndefined();
  });

  it('filtered search returns plain-object records', async () => {
    const results = await db.search('docs', [1, 0], {
      k: 3,
      filter: { condition: { type: 'eq', field: 'category', value: 'docs' } },
    });

    expect(results).toHaveLength(1);
    expect(results[0]!.id).toBe('2');
    expect(results[0]!.score).toBeCloseTo(0, 5);
    expect(results[0]!.payload).toEqual(NOTES);
  });

  it('textSearch returns ids and payloads, and no score: it ranks nothing', async () => {
    const results = await db.textSearch('docs', 'E', { k: 3 });

    // Case-insensitive substring match, in insertion order, unscored.
    expect(results).toEqual([
      { id: '1', score: null, payload: RUST },
      { id: '2', score: null, payload: NOTES },
    ]);
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
    expect(results[0]!.payload).toEqual(NOTES);
    expect(results[1]!.payload).toBeUndefined();
    expect(results[2]!.payload).toEqual(RUST);
  });

  it('multiQuerySearch returns fused ids, scores and payloads', async () => {
    // RRF over the two rankings [1, 3, 2] and [3, 2, 1] puts 3 first, then 1.
    const results = await db.multiQuerySearch('docs', [[1, 0], [0.6, 0.8]], { k: 2 });

    expect(results.map(r => r.id)).toEqual(['3', '1']);
    expect(results[0]!.score).toBeGreaterThan(results[1]!.score);
    expect(results[1]!.score).toBeGreaterThan(0);
    expect(results[1]!.payload).toEqual(RUST);
  });

  it('get returns the point with its payload', async () => {
    const point = await db.get('docs', 1);

    expect(point).toEqual({ id: '1', vector: [1, 0], payload: RUST });
  });

  it('query runs a VelesQL WHERE over the payloads the backend wrote', async () => {
    // The collection's store is attached to the binding's WasmDatabase, so
    // the executor filters the very points upserted above (was NOT_SUPPORTED).
    const response = await db.query('docs', "SELECT * FROM docs WHERE category = 'docs' LIMIT 10");

    expect('results' in response && response.results.map(r => r.id)).toEqual([2]);
  });

  it('query classifies by the parser: an aggregate word inside a literal is still a WHERE', async () => {
    const response = await db.query('docs', "SELECT * FROM docs WHERE title = 'hello rust' OR title = 'Having fun' LIMIT 10");

    expect('results' in response && response.results.map(r => r.id)).toEqual([1]);
  });

  it('query keeps refusing a real aggregation on WASM', async () => {
    await expect(
      db.query('docs', 'SELECT category, COUNT(*) FROM docs GROUP BY category')
    ).rejects.toMatchObject({ code: 'NOT_SUPPORTED' });
  });

  it('graph: edges between stored points are listed, counted and matched with REST shapes', async () => {
    await db.addEdge('docs', { id: 10, source: 1, target: 2, label: 'NEXT', properties: { weight: 0.5 } });

    expect(await db.getEdges('docs', { label: 'NEXT' })).toEqual([
      { id: '10', source: '1', target: '2', label: 'NEXT', properties: { weight: 0.5 } },
    ]);
    expect(await db.getEdgeCount('docs')).toBe(1);

    // The collection's points are the MATCH's nodes: their payloads project.
    const response = await db.matchQuery('docs', 'MATCH (a)-[r:NEXT]->(b) RETURN a.title, b.category, r.weight');
    expect(response.results).toEqual([
      {
        bindings: { a: '1', b: '2' },
        depth: 1,
        projected: { 'a.title': 'hello rust', 'b.category': 'docs', 'r.weight': 0.5 },
      },
    ]);
    expect(response.count).toBe(1);
    expect(response.meta.velesqlContractVersion).toMatch(/^\d+\.\d+\.\d+$/);

    expect(await db.removeEdge('docs', 10)).toBe(true);
    expect(await db.getEdgeCount('docs')).toBe(0);
  });

  it('graph: a derived edge id past 2^53 is listed and removed exactly', async () => {
    await db.addEdge('docs', { id: '9007199254740993', source: 1, target: 2, label: 'BIG' });

    const [edge] = await db.getEdges('docs', { label: 'BIG' });
    expect(edge?.id).toBe('9007199254740993');
    expect(await db.removeEdge('docs', edge!.id)).toBe(true);
    expect(await db.getEdges('docs', { label: 'BIG' })).toEqual([]);
  });

  it('graph: deleting a point deletes its edges, so a new point under its id starts unconnected', async () => {
    await db.addEdge('docs', { id: 30, source: 1, target: 2, label: 'GONE' });

    expect(await db.delete('docs', 2)).toBe(true);
    expect(await db.getEdges('docs', { label: 'GONE' })).toEqual([]);

    await db.upsert('docs', { id: 2, vector: [0, 1], payload: NOTES });
    const response = await db.matchQuery('docs', 'MATCH (a)-[:GONE]->(b) RETURN b.id');
    expect(response.results).toEqual([]);
  });

  it('graph: an edge to a point that does not exist is NOT_FOUND, as on REST', async () => {
    await expect(
      db.addEdge('docs', { id: 11, source: 1, target: 99, label: 'NEXT' })
    ).rejects.toMatchObject({ code: 'NOT_FOUND' });
  });

  it('graph: matchQuery scores the points\' vectors with similarity(), as on REST', async () => {
    await db.addEdge('docs', { id: 20, source: 1, target: 2, label: 'SEES' });
    await db.addEdge('docs', { id: 21, source: 1, target: 3, label: 'SEES' });

    // Cosine to [0, 1]: point 2 ([0, 1]) 1.0, point 3 ([0.6, 0.8]) 0.8.
    const where = await db.matchQuery(
      'docs',
      'MATCH (a)-[:SEES]->(b) WHERE similarity(b.embedding, $v) > 0.5 RETURN b.id ORDER BY similarity() DESC',
      { v: [0, 1] }
    );
    expect(where.results.map(r => r.bindings.b)).toEqual(['2', '3']);
    expect(where.results[0]!.score).toBeCloseTo(1, 4);

    const scored = await db.matchQuery('docs', 'MATCH (a)-[:SEES]->(b) RETURN b.id', undefined, {
      vector: [0, 1],
      threshold: 0.9,
    });
    expect(scored.results.map(r => r.bindings.b)).toEqual(['2']);
  });

  it('an upsert replaces the whole point: a payload or sparse vector it leaves out is gone', async () => {
    await db.createCollection('replace', { dimension: 2, metric: 'cosine' });
    await db.upsert('replace', { id: 1, vector: [1, 0], payload: RUST, sparseVector: { 1: 1 } });
    await db.upsert('replace', { id: 1, vector: [1, 0] });

    expect((await db.get('replace', 1))?.payload ?? null).toBeNull();
    const [hit] = await db.search('replace', [1, 0], { k: 1 });
    expect(hit!.payload).toBeUndefined();
    // Point 1 lost term 1 with its sparse vector: only point 2 still ranks
    // on the sparse side, which lifts it above point 1 in the fusion.
    await db.upsert('replace', { id: 2, vector: [0, 1], sparseVector: { 1: 0.5 } });
    const fused = await db.search('replace', [1, 0], { k: 2, sparseVector: { 1: 1 } });
    expect(fused.map(r => r.id)).toEqual(['2', '1']);
  });

  it('a point id past 2^53 comes back exactly from every search and read', async () => {
    // The binding threw serializing such an id as a number, and the SDK
    // refused it up front; it now leaves every surface as its decimal string.
    const big = '9007199254740993';
    await db.createCollection('big', { dimension: 2, metric: 'cosine' });
    await db.upsert('big', { id: big, vector: [1, 0], payload: RUST, sparseVector: { 1: 1 } });
    await db.upsert('big', { id: 5, vector: [0, 1], payload: NOTES, sparseVector: { 1: 0.5 } });

    const [hit] = await db.search('big', [1, 0], { k: 1 });
    expect(hit).toMatchObject({ id: big, payload: RUST });
    expect(await db.get('big', big)).toEqual({ id: big, vector: [1, 0], payload: RUST });
    const fused = await db.search('big', [1, 0], { k: 2, sparseVector: { 1: 1 } });
    expect(fused.map(r => r.id)).toEqual([big, '5']);
    expect((await db.textSearch('big', 'rust', { k: 2 })).map(r => r.id)).toEqual([big]);
    expect((await db.hybridSearch('big', [1, 0], 'rust', { k: 1 }))[0]!.id).toBe(big);
    expect((await db.multiQuerySearch('big', [[1, 0]], { k: 1 }))[0]!.id).toBe(big);

    expect(await db.delete('big', big)).toBe(true);
    expect(await db.get('big', big)).toBeNull();
    expect((await db.search('big', [1, 0], { k: 2, sparseVector: { 1: 1 } })).map(r => r.id)).toEqual(['5']);
  });
});
