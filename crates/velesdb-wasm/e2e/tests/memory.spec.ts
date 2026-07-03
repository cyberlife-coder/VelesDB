import { test, expect } from '@playwright/test';

/**
 * Memory Wedge E2E Tests for WASM SDK
 *
 * Runs `MemoryService` (remember/recall/recallFused/recallWhere/relate/
 * forget/why) in a real browser, proving the wedge works off the
 * wasm-pack `--target web` build the TypeScript SDK's `MemoryService`
 * class (sdks/typescript/src/memory.ts) also loads.
 */
test.describe('VelesDB WASM Memory Wedge', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => window['VelesDB']?.ready === true, { timeout: 10000 });
  });

  test('should remember, recall, relate, and explain a connected subgraph', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { MemoryService } = window['VelesDB'];
      const memory = new MemoryService(4);

      const decision = memory.remember('we chose parking_lot to avoid lock poisoning', [], null);
      const ticket = memory.remember('EPIC-317 xyzzy quux frobnicate', [], null);
      memory.relate(decision, ticket, 'decided_in');

      const hits = memory.recall('lock poisoning', 5, null);
      const fused = memory.recallFused('we chose parking_lot to avoid lock poisoning', 3, null, null);
      const explanation = memory.why('we chose parking_lot to avoid lock poisoning', 2, null);

      return { decision, ticket, hits, fused, explanation };
    });

    expect(result.hits.some((h) => h.id === result.decision)).toBe(true);
    expect(result.fused.some((h) => h.id === result.ticket)).toBe(true);
    expect(result.explanation.nodes.some((n) => n.id === result.ticket)).toBe(true);
  });

  test('should forget a memory so it no longer surfaces in recall', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { MemoryService } = window['VelesDB'];
      const memory = new MemoryService(4);

      const id = memory.remember('a fact to forget', [], null);
      memory.forget(id);
      const hits = memory.recall('a fact to forget', 5, null);

      return { id, hits };
    });

    expect(result.hits.every((h) => h.id !== result.id)).toBe(true);
  });

  test('should surface a structured NOT_FOUND error when relating to a missing memory', async ({
    page,
  }) => {
    const code = await page.evaluate(() => {
      const { MemoryService } = window['VelesDB'];
      const memory = new MemoryService(4);
      const a = memory.remember('fact a', [], null);
      try {
        memory.relate(a, '999999999', 'x');
        return null;
      } catch (e) {
        return e.code;
      }
    });

    expect(code).toBe('NOT_FOUND');
  });
});
