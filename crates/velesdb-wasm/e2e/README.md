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

## CI Integration

Add to `.github/workflows/ci.yml`:

```yaml
wasm-e2e:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - uses: actions/setup-node@v4
      with:
        node-version: '20'
    - name: Install wasm-pack
      run: cargo install wasm-pack
    - name: Build WASM
      run: |
        cd crates/velesdb-wasm
        wasm-pack build --target web --out-dir e2e/pkg
    - name: Install & Test
      run: |
        cd crates/velesdb-wasm/e2e
        npm ci
        npx playwright install --with-deps chromium
        npm test -- --project=chromium
```
