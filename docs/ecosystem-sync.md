# VelesDB Ecosystem Sync Report

> Generated: 2026-01-27  
> Status: Comprehensive audit of Core ↔ Ecosystem feature parity

## Executive Summary

This document tracks feature parity between `velesdb-core` and all ecosystem components (SDKs, integrations, demos, examples).

## Core Features Inventory

### Distance Metrics (5 metrics)
| Metric | Core | WASM | Python | Mobile | TypeScript | Server | CLI | LangChain | LlamaIndex |
|--------|------|------|--------|--------|------------|--------|-----|-----------|------------|
| Cosine | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Euclidean | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| DotProduct | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Hamming | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🔴 | 🔴 |
| Jaccard | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🔴 | 🔴 |

### Storage Modes (Quantization)
| Mode | Core | WASM | Python | Mobile | TypeScript | Server | CLI | LangChain | LlamaIndex |
|------|------|------|--------|--------|------------|--------|-----|-----------|------------|
| Full (f32) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| SQ8 (8-bit) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🔴 | 🔴 |
| Binary (1-bit) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🔴 | 🔴 |

### Search Features
| Feature | Core | WASM | Python | Mobile | TypeScript | Server | CLI | LangChain | LlamaIndex |
|---------|------|------|--------|--------|------------|--------|-----|-----------|------------|
| Vector Search | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Text Search (BM25) | ✅ | ✅ | ✅ | 🔴 | ✅ | ✅ | ✅ | ✅ | ✅ |
| Hybrid Search | ✅ | ✅ | ✅ | 🔴 | ✅ | ✅ | ✅ | ✅ | ✅ |
| Multi-Query Search | ✅ | ✅ | ✅ | 🔴 | ✅ | ✅ | 🔴 | 🔴 | 🔴 |
| Batch Search | ✅ | 🔴 | ✅ | 🔴 | ✅ | ✅ | 🔴 | 🔴 | 🔴 |
| Filter Expressions | ✅ | ✅ | ✅ | 🔴 | ✅ | ✅ | ✅ | ✅ | ✅ |

### Fusion Strategies
| Strategy | Core | WASM | Python | Mobile | TypeScript | Server | CLI |
|----------|------|------|--------|--------|------------|--------|-----|
| RRF | ✅ | ✅ | ✅ | 🔴 | ✅ | ✅ | 🔴 |
| Average | ✅ | ✅ | ✅ | 🔴 | ✅ | ✅ | 🔴 |
| Maximum | ✅ | ✅ | ✅ | 🔴 | ✅ | ✅ | 🔴 |
| Weighted | ✅ | ✅ | ✅ | 🔴 | ✅ | ✅ | 🔴 |

### Knowledge Graph (EPIC-004/016)
| Feature | Core | WASM | Python | Mobile | TypeScript | Server | CLI | LangChain |
|---------|------|------|--------|--------|------------|--------|-----|-----------|
| Node CRUD | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Edge CRUD | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| BFS Traversal | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| DFS Traversal | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🔴 | 🔴 |
| Streaming Traversal | ✅ | 🔴 | ✅ | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 |
| Graph Schema | ✅ | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 |

### VelesQL Query Language
| Feature | Core | WASM | Python | TypeScript | Server | CLI |
|---------|------|------|--------|------------|--------|-----|
| SELECT | ✅ | 🔴 | 🔴 | ✅ | ✅ | ✅ |
| WHERE filters | ✅ | 🔴 | 🔴 | ✅ | ✅ | ✅ |
| NEAR (vector) | ✅ | 🔴 | 🔴 | ✅ | ✅ | ✅ |
| ORDER BY | ✅ | 🔴 | 🔴 | ✅ | ✅ | ✅ |
| LIMIT/OFFSET | ✅ | 🔴 | 🔴 | ✅ | ✅ | ✅ |
| JOIN | ✅ | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 |
| USING FUSION | ✅ | 🔴 | 🔴 | ✅ | 🔴 | 🔴 |
| EXPLAIN | ✅ | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 |

### Column Store (EPIC-007)
| Feature | Core | WASM | Python | TypeScript | Server | CLI |
|---------|------|------|--------|------------|--------|-----|
| Typed Columns | ✅ | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 |
| Batch Update | ✅ | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 |
| String Interning | ✅ | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 |
| TTL/Expiration | ✅ | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 |

### Advanced Features
| Feature | Core | WASM | Python | Mobile | TypeScript | Server | CLI |
|---------|------|------|--------|--------|------------|--------|-----|
| Half Precision (f16) | ✅ | ✅ | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 |
| SIMD Acceleration | ✅ | ✅ | ✅ | ✅ | N/A | N/A | N/A |
| Async Operations | ✅ | 🔴 | 🔴 | 🔴 | ✅ | ✅ | 🔴 |
| Metrics/Telemetry | ✅ | 🔴 | 🔴 | 🔴 | 🔴 | ✅ | 🔴 |
| Guard Rails | ✅ | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 |

---

## Gap Analysis by Component

### 1. velesdb-wasm (crates/velesdb-wasm)
**Status**: 🟡 Good coverage, some gaps

**Missing Features**:
- VelesQL query execution
- Batch search
- Streaming graph traversal
- Graph schema support
- Column store integration

**Priority**: P1 - VelesQL support would enable advanced queries in browser

### 2. velesdb-python (crates/velesdb-python)
**Status**: 🟡 Good coverage, VelesQL missing

**Missing Features**:
- VelesQL query execution
- Graph schema
- Half precision support
- Column store APIs

**Priority**: P1 - VelesQL critical for Python data science workflows

### 3. velesdb-mobile (crates/velesdb-mobile)
**Status**: 🔴 Significant gaps

**Missing Features**:
- Text/Hybrid search
- Multi-query search
- Fusion strategies
- Streaming traversal
- Graph schema
- Filter expressions

**Priority**: P2 - Mobile apps need hybrid search

### 4. TypeScript SDK (sdks/typescript)
**Status**: 🟢 Good coverage

**Missing Features**:
- JOIN queries
- Streaming traversal
- Column store
- EXPLAIN query

**Priority**: P2 - Core functionality present

### 5. velesdb-server (crates/velesdb-server)
**Status**: 🟢 Good coverage

**Missing Features**:
- VelesQL JOIN endpoint
- EXPLAIN endpoint
- Streaming traversal endpoint
- Column store endpoints

**Priority**: P1 - Server should expose all Core features

### 6. velesdb-cli (crates/velesdb-cli)
**Status**: 🟡 Good for REPL, missing advanced features

**Missing Features**:
- Multi-query search command
- Batch search command
- DFS traversal
- Fusion strategy options
- Streaming traversal

**Priority**: P3 - CLI sufficient for basic operations

### 7. LangChain Integration (integrations/langchain)
**Status**: 🟡 VectorStore OK, missing advanced

**Missing Features**:
- Hamming/Jaccard metrics
- SQ8/Binary storage modes
- Multi-query search
- Batch search
- DFS traversal
- Streaming traversal

**Priority**: P2 - Quantization useful for large datasets

### 8. LlamaIndex Integration (integrations/llamaindex)
**Status**: 🟡 Basic VectorStore

**Missing Features**:
- Hamming/Jaccard metrics
- SQ8/Binary storage modes
- Multi-query search
- Batch search
- Graph features

**Priority**: P2 - Similar to LangChain gaps

### 9. Tauri Plugin (crates/tauri-plugin-velesdb)
**Status**: 🟢 Good coverage for desktop apps

**Verified Features**:
- Collection CRUD
- Vector operations
- Search (vector, text, hybrid)
- VelesQL queries

**Priority**: P3 - Sufficient for desktop RAG apps

---

## Demos & Examples Status

### demos/rag-pdf-demo
- Uses Python SDK
- Demonstrates: PDF → embeddings → VelesDB → RAG
- **Status**: ✅ Up to date

### demos/tauri-rag-app
- Uses Tauri plugin
- Demonstrates: Desktop RAG application
- **Status**: ✅ Up to date

### examples/python
- Basic Python SDK usage
- **Status**: 🟡 Needs update for new features (fusion, graph)

### examples/rust
- Rust direct usage
- **Status**: 🟡 Needs update for VelesQL examples

### examples/wasm-browser-demo
- Browser vector search
- **Status**: 🟡 Needs update for graph features

---

## Recommended Actions

### High Priority (P1)

1. **VelesQL for Python SDK** → **EPIC-056 US-001/002/003**
   - Add `collection.query(velesql_string)` method
   - Expose parser bindings via PyO3

2. **VelesQL for WASM** → **EPIC-056 US-004/005/006**
   - Add `store.query(velesql_string)` method
   - Enable browser-side query parsing

3. **Server JOIN/EXPLAIN endpoints** → **EPIC-058 US-001/002**
   - `/api/v1/query/explain` endpoint
   - Full VelesQL support including JOINs

### Medium Priority (P2)

4. **Mobile Hybrid Search** → **EPIC-036 (existing)**
   - Add text_search and hybrid_search to UniFFI bindings
   - Add filter support

5. **LangChain/LlamaIndex Quantization** → **EPIC-057**
   - Expose storage_mode parameter
   - Document memory savings

6. **Streaming Traversal** → **EPIC-058 US-003**
   - Add to WASM, Server, TypeScript
   - Important for large graphs

### Low Priority (P3)

7. **Examples Update** → **EPIC-059 US-005/006/007**
   - Add VelesQL examples to all language examples
   - Add graph traversal examples
   - Add fusion strategy examples

8. **CLI Enhancements** → **EPIC-059 US-001/002/003/004**
   - Add `--fusion` flag to search commands
   - Add `velesdb traverse` command

---

## EPICs Created for Gap Resolution

| EPIC | Titre | Priorité | US | Estimation |
|------|-------|----------|-----|------------|
| **EPIC-053** | WASM Graph Support | P1 | 6 | 29h |
| **EPIC-056** | VelesQL SDK Propagation | P1 | 8 | 35h |
| **EPIC-057** | LangChain/LlamaIndex Parity | P2 | 9 | 33h |
| **EPIC-058** | Server API Completeness | P1 | 6 | 28h |
| **EPIC-059** | CLI & Examples Refresh | P3 | 7 | 20h |
| **EPIC-036** | Mobile SDK UniFFI (existing) | P2 | 5 TODO | ~20h |

**Total: ~165h de travail**

### Implementation Order (Recommended)

1. **EPIC-056** (VelesQL SDKs) - Foundation for Python/WASM users
2. **EPIC-058** (Server API) - Enable backend integrations  
3. **EPIC-053** (WASM Graph) - Browser graph support
4. **EPIC-057** (LangChain/LlamaIndex) - Framework integrations
5. **EPIC-036** (Mobile) - iOS/Android packaging
6. **EPIC-059** (CLI/Examples) - Documentation and DX

---

## Version Matrix

| Component | Version | Last Updated | Core Compatibility |
|-----------|---------|--------------|-------------------|
| velesdb-core | 1.3.1 | 2026-01-27 | - |
| velesdb-wasm | 1.3.1 | 2026-01-27 | ✅ |
| velesdb-python | 1.3.1 | 2026-01-27 | ✅ |
| velesdb-mobile | 0.8.0 | 2026-01-15 | 🟡 |
| velesdb-server | 1.3.1 | 2026-01-27 | ✅ |
| velesdb-cli | 1.3.1 | 2026-01-27 | ✅ |
| TypeScript SDK | 0.8.10 | 2026-01-20 | ✅ |
| LangChain | 0.8.10 | 2026-01-20 | 🟡 |
| LlamaIndex | 0.8.10 | 2026-01-20 | 🟡 |
| Tauri Plugin | 0.8.0 | 2026-01-15 | ✅ |

---

## Legend

- ✅ Fully implemented and tested
- 🟡 Partially implemented or needs update
- 🔴 Not implemented
- N/A Not applicable for this component
