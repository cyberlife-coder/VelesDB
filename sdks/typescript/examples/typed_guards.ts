/**
 * Typed builder guards for the VelesDB TypeScript SDK (#27).
 *
 * This file exists primarily as a COMPILE-TIME contract, type-checked in CI
 * via `npm run typecheck` (tsconfig.examples.json includes `examples/**`).
 * It pins two SDK guarantees:
 *
 *   1. `nearFused()`'s strategy only accepts `rrf` | `average` | `maximum`.
 *      `weighted` / `relative_score` are rejected at compile time — they
 *      have no per-branch weights over N homogeneous query vectors and the
 *      engine silently downgrades them to RRF (the "weighted -> RRF" trap).
 *      The `@ts-expect-error` lines below FAIL the build if the type ever
 *      widens to allow them.
 *
 *   2. `db.setAutoReindex()` / `db.alterCollection()` exist and are typed.
 *
 * Run (against a live server):  npx tsx examples/typed_guards.ts
 */

import { VelesDB, velesql } from '../src';

const a = [0.1, 0.2, 0.3];
const b = [0.4, 0.5, 0.6];

// --- nearFused() compile-time strategy guard -------------------------------

// Allowed strategies type-check cleanly.
velesql().from('docs').nearFused(['$a', '$b'], [a, b], { strategy: 'rrf' });
velesql().from('docs').nearFused(['$a', '$b'], [a, b], { strategy: 'average' });
velesql().from('docs').nearFused(['$a', '$b'], [a, b], { strategy: 'maximum' });

// @ts-expect-error 'weighted' silently degrades to RRF — disallowed by the typed builder.
velesql().from('docs').nearFused(['$a', '$b'], [a, b], { strategy: 'weighted' });

// @ts-expect-error 'relative_score' is not a valid NEAR_FUSED strategy.
velesql().from('docs').nearFused(['$a', '$b'], [a, b], { strategy: 'relative_score' });

// --- ALTER COLLECTION typed helpers ----------------------------------------

export async function toggleAutoReindex(): Promise<void> {
  const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
  await db.init();
  await db.setAutoReindex('docs', true);
  await db.alterCollection('docs', { autoReindex: false });
}
