/**
 * VelesQL Query Builder ROUND-TRIP conformance test (#11).
 *
 * The pre-existing query-builder tests only string-matched the builder
 * output, which is exactly why three invalid-VelesQL bugs shipped
 * (`vector NEAR $x TOP n`, MATCH-only output with ORDER BY/LIMIT before
 * the mandatory RETURN, and fusion rendered as an inert `/* FUSION ... *​/`
 * comment).
 *
 * This suite feeds every `toVelesQL()` output through the REAL core parser
 * shipped in `@wiscale/velesdb-wasm` (`VelesQL.parse`) and asserts ZERO
 * syntax errors. Anything the builder can emit MUST parse.
 *
 * The WASM module is loaded once (Node path: read the `.wasm` bytes off
 * disk and pass them to the wasm-pack initializer), mirroring how
 * `src/backends/wasm-node-loader.ts` boots WASM under Node.
 */

import { describe, it, expect, beforeAll } from 'vitest';
import { readFile, readdir } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { velesql } from '../src/query-builder';

const require = createRequire(import.meta.url);

// Loaded in beforeAll; typed loosely because the WASM .d.ts is not part of
// the SDK's own type graph here.
let parse: (q: string) => unknown;

async function loadRealParser(): Promise<(q: string) => unknown> {
  const pkgJson = require.resolve('@wiscale/velesdb-wasm/package.json');
  const pkgDir = path.dirname(pkgJson);
  const files = await readdir(pkgDir);
  const wasmFile = files.find((f) => f.endsWith('.wasm'));
  if (!wasmFile) throw new Error('velesdb-wasm: no .wasm binary found on disk');
  const jsEntry = files.find((f) => f.endsWith('.js')) ?? 'velesdb_wasm.js';
  const mod = (await import(path.join(pkgDir, jsEntry))) as {
    default: (init: { module_or_path: BufferSource }) => Promise<unknown>;
    VelesQL: { parse: (q: string) => unknown };
  };
  const bytes = await readFile(path.join(pkgDir, wasmFile));
  await mod.default({ module_or_path: new Uint8Array(bytes) });
  return (q: string) => mod.VelesQL.parse(q);
}

beforeAll(async () => {
  parse = await loadRealParser();
});

/** Assert a builder output parses with the real core parser (no throw). */
function expectParses(query: string): void {
  expect(() => parse(query), `should parse: ${query}`).not.toThrow();
}

const embedding = [0.1, 0.2, 0.3, 0.4];

describe('Query builder round-trip through the REAL core parser', () => {
  it('parses a plain MATCH (RETURN auto-appended)', () => {
    expectParses(velesql().match('n', 'Person').toVelesQL());
  });

  it('parses MATCH + WHERE', () => {
    expectParses(velesql().match('n', 'Person').where('n.age > 21').toVelesQL());
  });

  it('parses MATCH + RETURN + ORDER BY + LIMIT (RETURN before ORDER BY/LIMIT)', () => {
    expectParses(
      velesql()
        .match('n', 'Person')
        .return(['n.name', 'n.email'])
        .orderBy('n.name', 'DESC')
        .limit(10)
        .toVelesQL()
    );
  });

  it('parses a relationship traversal', () => {
    expectParses(
      velesql().match('a', 'Person').rel('KNOWS', 'r').to('b', 'Person').toVelesQL()
    );
  });

  it('parses a variable-length path', () => {
    expectParses(
      velesql()
        .match('a', 'Person')
        .rel('KNOWS', 'p', { minHops: 1, maxHops: 3 })
        .to('b', 'Person')
        .return(['b'])
        .toVelesQL()
    );
  });

  it('parses nearVector with topK (mapped to LIMIT, no TOP keyword)', () => {
    expectParses(
      velesql().match('d', 'Document').nearVector('$q', embedding, { topK: 50 }).toVelesQL()
    );
  });

  it('parses the README "Vector similarity with filters" example (SELECT mode)', () => {
    expectParses(
      velesql()
        .from('documents', 'd')
        .nearVector('$queryVector', embedding)
        .andWhere('d.category = $cat', { cat: 'tech' })
        .orderBy('score', 'DESC')
        .limit(10)
        .toVelesQL()
    );
  });

  it('parses a plain SELECT projection', () => {
    expectParses(velesql().from('docs').select(['title', 'category']).limit(5).toVelesQL());
  });

  it('parses every fusion strategy as a real USING FUSION clause', () => {
    const strategies = ['rrf', 'average', 'maximum', 'weighted', 'relative_score'] as const;
    for (const strategy of strategies) {
      const q = velesql()
        .from('docs')
        .nearVector('$q', embedding)
        .andWhere("content MATCH 'x'")
        .limit(10)
        .fusion(strategy, { k: 60, vectorWeight: 0.7, graphWeight: 0.3 })
        .toVelesQL();
      expect(q).not.toContain('/*');
      expectParses(q);
    }
  });

  it('parses weighted fusion with only a vector weight (#11 verify case)', () => {
    const q = velesql()
      .from('docs')
      .nearVector('$q', embedding)
      .andWhere("content MATCH 'x'")
      .fusion('weighted', { vectorWeight: 0.7 })
      .toVelesQL();
    expect(q).toContain("USING FUSION(strategy='weighted', vector_weight=0.7");
    expectParses(q);
  });

  it('parses a NEAR_FUSED multi-vector query for every allowed strategy', () => {
    const strategies = ['rrf', 'average', 'maximum'] as const;
    for (const strategy of strategies) {
      const q = velesql()
        .from('docs')
        .nearFused(['$a', '$b'], [embedding, embedding], { strategy })
        .limit(10)
        .toVelesQL();
      expect(q).toContain(`USING FUSION '${strategy}'`);
      expectParses(q);
    }
  });

  it('parses NEAR_FUSED without an explicit strategy', () => {
    expectParses(
      velesql()
        .from('docs')
        .nearFused(['$a', '$b'], [embedding, embedding])
        .limit(5)
        .toVelesQL()
    );
  });

  it('parses the complete RAG MATCH query', () => {
    expectParses(
      velesql()
        .match('d', 'Document')
        .nearVector('$embedding', embedding, { topK: 100 })
        .andWhere('d.language = $lang', { lang: 'en' })
        .andWhere('d.published = $pub', { pub: true })
        .orderBy('score', 'DESC')
        .limit(20)
        .return(['d.title', 'd.content', 'score'])
        .toVelesQL()
    );
  });
});
