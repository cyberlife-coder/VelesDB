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
    hybrid: boolean;
  };
  performance: {
    vectorSearchUs: number | null;
    filteredSearchUs: number | null;
    graphLookupUs: number | null;
    hybridUs: number | null;
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
      hybrid: false,
    },
    performance: {
      vectorSearchUs: null,
      filteredSearchUs: null,
      graphLookupUs: null,
      hybridUs: null,
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
  result.queryResults.hybrid = output.includes('QUERY 4: HYBRID');

  // Parse performance metrics from Rust's Debug Duration format.
  // Rust prints durations as e.g. "187.123µs", "1.234ms", "2.5s", or "500ns".
  // We normalize everything to microseconds.
  function parseDurationToUs(durationStr: string): number | null {
    const nsMatch = durationStr.match(/([\d.]+)ns/);
    if (nsMatch) return Number.parseFloat(nsMatch[1]) / 1000;
    const usMatch = durationStr.match(/([\d.]+)µs/);
    if (usMatch) return Number.parseFloat(usMatch[1]);
    const msMatch = durationStr.match(/([\d.]+)ms/);
    if (msMatch) return Number.parseFloat(msMatch[1]) * 1000;
    const sMatch = durationStr.match(/([\d.]+)s$/);
    if (sMatch) return Number.parseFloat(sMatch[1]) * 1_000_000;
    return null;
  }

  const vectorSearchMatch = output.match(/Found \d+ similar products in ([\d.]+(?:ns|µs|ms|s))/);
  if (vectorSearchMatch) {
    result.performance.vectorSearchUs = parseDurationToUs(vectorSearchMatch[1]);
  }

  const filteredMatch = output.match(/Found \d+ filtered results in ([\d.]+(?:ns|µs|ms|s))/);
  if (filteredMatch) {
    result.performance.filteredSearchUs = parseDurationToUs(filteredMatch[1]);
  }

  const graphMatch = output.match(/Found \d+ co-purchased products in ([\d.]+(?:ns|µs|ms|s))/);
  if (graphMatch) {
    result.performance.graphLookupUs = parseDurationToUs(graphMatch[1]);
  }

  const hybridMatch = output.match(/Found \d+ recommendations in ([\d.]+(?:ns|µs|ms|s))/);
  if (hybridMatch) {
    result.performance.hybridUs = parseDurationToUs(hybridMatch[1]);
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
    const m = output.raw.match(/Found (\d+) similar products in/);
    expect(m).not.toBeNull();
    expect(Number.parseInt(m![1], 10)).toBeGreaterThan(0);
  });

  test('should execute Vector + Filter query (Query 2)', () => {
    expect(output.queryResults.vectorFilter).toBe(true);
    expect(output.raw).toMatch(/Found \d+ filtered results in/);
  });

  test('should execute Graph Lookup query (Query 3)', () => {
    expect(output.queryResults.graphLookup).toBe(true);
    const m = output.raw.match(/Found (\d+) co-purchased products/);
    expect(m).not.toBeNull();
    expect(Number.parseInt(m![1], 10)).toBeGreaterThan(0);
  });

  test('should execute Hybrid query (Query 4)', () => {
    expect(output.queryResults.hybrid).toBe(true);
    // The hybrid result line is printed unconditionally after RRF fusion +
    // post-filtering, so it ties this test to the hybrid path,
    // not just the static section header. Count may be 0 but the line prints.
    expect(output.raw).toMatch(/Found \d+ recommendations in /);
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

    test('hybrid query should be under 10ms', () => {
      expect(output.performance.hybridUs).not.toBeNull();
      expect(output.performance.hybridUs!).toBeLessThan(10000);
    });
  });

  test.describe('Output Validation', () => {
    test('should show product details with name, price, rating', () => {
      // Check that results include product details
      expect(output.raw).toMatch(/\$[\d.]+/); // Price format
      expect(output.raw).toMatch(/[\d.]+\/5 ⭐/); // Rating format
    });

    test('should show hybrid search details', () => {
      expect(output.raw).toContain('Vector similarity (60% weight)');
      expect(output.raw).toContain('BM25 tag matching (40% weight)');
      expect(output.raw).toContain('in_stock=true');
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
