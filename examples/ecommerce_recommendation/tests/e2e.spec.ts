/**
 * E2E Tests for VelesDB E-commerce Recommendation Example
 * 
 * Validates that the example:
 * 1. Compiles successfully
 * 2. Runs without errors
 * 3. Generates expected output (5000 products, 4 query types)
 * 4. Meets performance requirements (<1s for queries)
 */

import { test, expect } from '@playwright/test';
import { exec } from 'node:child_process';
import { promisify } from 'node:util';
import * as path from 'node:path';

const execAsync = promisify(exec);
const EXAMPLE_DIR = path.join(__dirname, '..');
const TIMEOUT_MS = 120000; // 2 minutes for compilation + execution

interface ExampleOutput {
  raw: string;
  productCount: number;
  relationCount: number;
  queryResults: {
    vectorSimilarity: boolean;
    vectorFilter: boolean;
    graphLookup: boolean;
    combined: boolean;
  };
  performance: {
    vectorSearchUs: number | null;
    filteredSearchUs: number | null;
    graphLookupUs: number | null;
    combinedUs: number | null;
  };
}

function parseOutput(output: string): ExampleOutput {
  const result: ExampleOutput = {
    raw: output,
    productCount: 0,
    relationCount: 0,
    queryResults: {
      vectorSimilarity: false,
      vectorFilter: false,
      graphLookup: false,
      combined: false,
    },
    performance: {
      vectorSearchUs: null,
      filteredSearchUs: null,
      graphLookupUs: null,
      combinedUs: null,
    },
  };

  // Parse product count
  const productMatch = output.match(/Generated (\d+) products/);
  if (productMatch) {
    result.productCount = Number.parseInt(productMatch[1], 10);
  }

  // Parse relation count
  const relationMatch = output.match(/Generated (\d+) co-purchase relationships/);
  if (relationMatch) {
    result.relationCount = Number.parseInt(relationMatch[1], 10);
  }

  // Check query sections
  result.queryResults.vectorSimilarity = output.includes('QUERY 1: Vector Similarity');
  result.queryResults.vectorFilter = output.includes('QUERY 2: Vector + Filter');
  result.queryResults.graphLookup = output.includes('QUERY 3: Graph Lookup');
  result.queryResults.combined = output.includes('QUERY 4: COMBINED');

  // Parse performance metrics (µs values)
  const vectorSearchMatch = output.match(/Found \d+ similar products in ([\d.]+)µs/);
  if (vectorSearchMatch) {
    result.performance.vectorSearchUs = Number.parseFloat(vectorSearchMatch[1]);
  }

  const filteredMatch = output.match(/Found \d+ filtered results in ([\d.]+)µs/);
  if (filteredMatch) {
    result.performance.filteredSearchUs = Number.parseFloat(filteredMatch[1]);
  }

  const graphMatch = output.match(/Found \d+ co-purchased products in ([\d.]+)µs/);
  if (graphMatch) {
    result.performance.graphLookupUs = Number.parseFloat(graphMatch[1]);
  }

  const combinedMatch = output.match(/Found \d+ recommendations in ([\d.]+)µs/);
  if (combinedMatch) {
    result.performance.combinedUs = Number.parseFloat(combinedMatch[1]);
  }

  return result;
}

test.describe('E-commerce Recommendation Example E2E', () => {
  test.setTimeout(TIMEOUT_MS);

  let output: ExampleOutput;

  test.beforeAll(async () => {
    // Build and run the example
    const { stdout, stderr } = await execAsync('cargo run --release', {
      cwd: EXAMPLE_DIR,
      timeout: TIMEOUT_MS,
    });

    if (stderr && !stderr.includes('Compiling') && !stderr.includes('Finished')) {
      console.error('Stderr:', stderr);
    }

    output = parseOutput(stdout);
  });

  test('should generate 5000 products', () => {
    expect(output.productCount).toBe(5000);
  });

  test('should generate co-purchase relationships', () => {
    expect(output.relationCount).toBeGreaterThan(10000);
    expect(output.relationCount).toBeLessThan(30000);
  });

  test('should execute Vector Similarity query (Query 1)', () => {
    expect(output.queryResults.vectorSimilarity).toBe(true);
  });

  test('should execute Vector + Filter query (Query 2)', () => {
    expect(output.queryResults.vectorFilter).toBe(true);
  });

  test('should execute Graph Lookup query (Query 3)', () => {
    expect(output.queryResults.graphLookup).toBe(true);
  });

  test('should execute Combined query (Query 4)', () => {
    expect(output.queryResults.combined).toBe(true);
  });

  test('should complete demo successfully', () => {
    expect(output.raw).toContain('Demo completed!');
    expect(output.raw).toContain('VelesDB powers your recommendations');
  });

  test.describe('Performance Requirements', () => {
    test('vector search should be under 10ms', () => {
      expect(output.performance.vectorSearchUs).not.toBeNull();
      expect(output.performance.vectorSearchUs!).toBeLessThan(10000); // 10ms = 10000µs
    });

    test('filtered search should be under 10ms', () => {
      expect(output.performance.filteredSearchUs).not.toBeNull();
      expect(output.performance.filteredSearchUs!).toBeLessThan(10000);
    });

    test('graph lookup should be under 1ms', () => {
      expect(output.performance.graphLookupUs).not.toBeNull();
      expect(output.performance.graphLookupUs!).toBeLessThan(1000); // 1ms = 1000µs
    });

    test('combined query should be under 10ms', () => {
      expect(output.performance.combinedUs).not.toBeNull();
      expect(output.performance.combinedUs!).toBeLessThan(10000);
    });
  });

  test.describe('Output Validation', () => {
    test('should show product details with name, price, rating', () => {
      // Check that results include product details
      expect(output.raw).toMatch(/\$[\d.]+/); // Price format
      expect(output.raw).toMatch(/[\d.]+\/5 ⭐/); // Rating format
    });

    test('should show VelesQL query example', () => {
      expect(output.raw).toContain('SELECT * FROM products');
      expect(output.raw).toContain('similarity(embedding, ?)');
      expect(output.raw).toContain('in_stock = true');
    });

    test('should show graph query syntax', () => {
      expect(output.raw).toContain('MATCH (p:Product)-[:BOUGHT_TOGETHER]-(other)');
    });

    test('should display performance summary', () => {
      expect(output.raw).toContain('Products indexed:');
      expect(output.raw).toContain('Co-purchase relations:');
      expect(output.raw).toContain('Vector dimensions:');
    });
  });
});
