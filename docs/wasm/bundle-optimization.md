# WASM Bundle Optimization (EPIC-053/US-006)

## Overview

This document describes the bundle optimization strategies for `velesdb-wasm`.

## Build Configuration

### Release Build (Optimized)

```bash
wasm-pack build --release --target web
```

The release build enables:
- **wasm-opt -Os**: Size optimization
- **SIMD support**: `--enable-simd` for faster vector operations
- **Dead code elimination**: Automatic tree-shaking

### Development Build (Fast)

```bash
wasm-pack build --dev --target web
```

Development builds skip wasm-opt for faster iteration.

## Bundle Size Targets

| Build | Target Size | Notes |
|-------|-------------|-------|
| Release (gzipped) | < 200 KB | Core vector operations |
| Release (raw) | < 800 KB | Before compression |
| Dev | N/A | Speed over size |

## Tree-Shaking

The package supports tree-shaking via ES modules:

```javascript
// Only imports VectorStore - other modules are tree-shaken
import { VectorStore } from '@wiscale/velesdb-wasm';

// Full import (larger bundle)
import * as velesdb from '@wiscale/velesdb-wasm';
```

## Module Structure

```
velesdb-wasm/
├── VectorStore      # Core vector operations
├── GraphStore       # Knowledge graph
├── GraphPersistence # IndexedDB persistence
├── VelesQL          # Query parser
├── SemanticMemory   # Agent memory
└── graph_worker     # Web Worker support
```

## Lazy Loading

For large applications, consider lazy loading:

```javascript
// Load WASM module on demand
async function initVelesDB() {
  const { default: init, VectorStore } = await import('@wiscale/velesdb-wasm');
  await init();
  return new VectorStore(384, 'cosine');
}
```

## Performance Tips

1. **Use Web Workers** for heavy traversals (see `graph_worker` module)
2. **Batch inserts** with `insert_batch()` instead of individual `insert()`
3. **Reuse VectorStore** instances instead of creating new ones
4. **Use SQ8 storage mode** for 4x memory reduction with minimal accuracy loss

## Measuring Bundle Size

```bash
# Build and analyze
wasm-pack build --release --target web
ls -la pkg/*.wasm

# With wasm-opt stats
wasm-opt -Os --print-stack-ir pkg/velesdb_wasm_bg.wasm -o /dev/null
```

## Changelog

| Date | Change |
|------|--------|
| 2026-01-29 | Initial documentation (US-006) |
