# VelesDB WASM E2E Browser Tests

End-to-end browser tests for VelesDB WASM using Playwright.

## Prerequisites

- Node.js 18+
- wasm-pack (`cargo install wasm-pack`)

## Setup

```bash
# Install dependencies
npm install

# Install Playwright browsers
npx playwright install

# Build WASM module
npm run build:wasm
```

## Running Tests

```bash
# Run all tests (headless)
npm test

# Run with browser visible
npm run test:headed

# Run with Playwright UI
npm run test:ui
```

## Test Coverage

| Test | Description |
|------|-------------|
| Load WASM module | Verifies WASM initializes correctly |
| Create VectorStore | Tests cosine, euclidean, dot metrics |
| Insert & Search | Vector CRUD operations |
| Batch insert | Performance with 100 vectors |
| Error handling | Invalid metric, dimension mismatch |
| Remove/Clear | Vector deletion operations |
| Large vectors | 1536-dim (OpenAI embeddings) |
| Multi-query fusion | RRF and average fusion, batch search |
| VelesQL | `VelesQL.parse` / `isValid`: SELECT, WHERE, NEAR, FUSION, ORDER BY, a refused query |
| Memory wedge | remember / recall / relate / why / forget, refused counts, `NOT_FOUND` |

## CI Integration

The `wasm-check` job of `.github/workflows/ci.yml` runs the whole suite on
chromium, over the `wasm-pack --target web` package it builds for the
TypeScript SDK's real-binding test.
