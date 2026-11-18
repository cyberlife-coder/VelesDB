import { test, expect } from '@playwright/test';

/**
 * VelesQL E2E Tests for WASM SDK
 * 
 * Tests the VelesQL query parsing functionality in the browser, through the
 * static `VelesQL.parse` / `VelesQL.isValid` (a refused query throws).
 * EPIC-060: Complete E2E test coverage
 */
test.describe('VelesDB WASM VelesQL', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => window['VelesDB']?.ready === true, { timeout: 10000 });
  });

  test('should parse basic SELECT query', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { VelesQL } = window['VelesDB'];
      const parsed = VelesQL.parse('SELECT * FROM documents LIMIT 10');
      return {
        isValid: parsed.isValid,
        tableName: parsed.tableName,
        // `limit` is a BigInt (a u64 getter); compare it as a number.
        limit: Number(parsed.limit),
      };
    });

    expect(result.isValid).toBe(true);
    expect(result.tableName).toBe('documents');
    expect(result.limit).toBe(10);
  });

  test('should parse SELECT with WHERE clause', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { VelesQL } = window['VelesDB'];
      const parsed = VelesQL.parse("SELECT id, name FROM users WHERE status = 'active'");
      return {
        isValid: parsed.isValid,
        hasWhereClause: parsed.hasWhereClause,
        columns: parsed.columns,
      };
    });

    expect(result.isValid).toBe(true);
    expect(result.hasWhereClause).toBe(true);
  });

  test('should parse NEAR vector search query', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { VelesQL } = window['VelesDB'];
      const parsed = VelesQL.parse('SELECT * FROM embeddings WHERE vector NEAR $query LIMIT 5');
      return {
        isValid: parsed.isValid,
        hasVectorSearch: parsed.hasVectorSearch,
      };
    });

    expect(result.isValid).toBe(true);
    expect(result.hasVectorSearch).toBe(true);
  });

  test('should parse FUSION hybrid search query', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { VelesQL } = window['VelesDB'];
      const parsed = VelesQL.parse(
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 USING FUSION(strategy = 'rrf', k = 60)"
      );
      return {
        isValid: parsed.isValid,
        hasFusion: parsed.hasFusion,
      };
    });

    expect(result.isValid).toBe(true);
    expect(result.hasFusion).toBe(true);
  });

  test('should reject invalid SQL syntax', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { VelesQL } = window['VelesDB'];
      let error = null;
      try {
        VelesQL.parse('INVALID QUERY SYNTAX');
      } catch (e) {
        error = e.message ?? String(e);
      }
      return { isValid: VelesQL.isValid('INVALID QUERY SYNTAX'), error };
    });

    expect(result.isValid).toBe(false);
    expect(result.error).toBeTruthy();
  });

  test('should parse ORDER BY clause', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { VelesQL } = window['VelesDB'];
      const parsed = VelesQL.parse('SELECT * FROM items ORDER BY price DESC LIMIT 20');
      return {
        isValid: parsed.isValid,
        hasOrderBy: parsed.hasOrderBy,
      };
    });

    expect(result.isValid).toBe(true);
    expect(result.hasOrderBy).toBe(true);
  });

  test('should parse complex filter expressions', async ({ page }) => {
    const result = await page.evaluate(() => {
      const { VelesQL } = window['VelesDB'];
      const parsed = VelesQL.parse("SELECT * FROM products WHERE category = 'tech' AND price > 100 AND in_stock = true");
      return {
        isValid: parsed.isValid,
        hasWhereClause: parsed.hasWhereClause,
      };
    });

    expect(result.isValid).toBe(true);
    expect(result.hasWhereClause).toBe(true);
  });
});
