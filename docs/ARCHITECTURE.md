# VelesDB Architecture

This document describes the internal architecture of VelesDB.

## High-Level Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           CLIENT LAYER                                   │
├─────────────────────────────────────────────────────────────────────────┤
│  TypeScript SDK   │   Python SDK   │   REST Client   │   VelesQL CLI   │
│  (@velesdb/sdk)   │   (velesdb)    │   (curl/HTTP)   │   (velesdb-cli) │
└────────┬──────────┴───────┬────────┴────────┬────────┴────────┬────────┘
         │                  │                 │                 │
         ▼                  ▼                 ▼                 ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                           API LAYER                                      │
├─────────────────────────────────────────────────────────────────────────┤
│   WASM Module     │   Python Bindings   │    REST Server    │   CLI    │
│  (velesdb-wasm)   │   (velesdb-python)  │  (velesdb-server) │  (REPL)  │
│                   │       PyO3          │      Axum         │          │
└────────┬──────────┴───────┬─────────────┴────────┬──────────┴────┬─────┘
         │                  │                      │               │
         ▼                  ▼                      ▼               ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                          CORE ENGINE                                     │
│                         (velesdb-core)                                   │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌─────────────┐ │
│  │   Database   │  │  Collection  │  │   VelesQL    │  │   Filter    │ │
│  │  Management  │  │  Operations  │  │   Parser     │  │   Engine    │ │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └──────┬──────┘ │
│         │                 │                 │                 │         │
│         ▼                 ▼                 ▼                 ▼         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                       INDEX LAYER                                │   │
│  ├─────────────────────────────────────────────────────────────────┤   │
│  │   ┌─────────────┐    ┌─────────────┐    ┌─────────────────────┐ │   │
│  │   │  HNSW Index │    │ BM25 Index  │    │  ColumnStore Filter │ │   │
│  │   │  (ANN)      │    │ (Full-Text) │    │  (RoaringBitmap)    │ │   │
│  │   └──────┬──────┘    └──────┬──────┘    └──────────┬──────────┘ │   │
│  └──────────┼──────────────────┼─────────────────────┼─────────────┘   │
│             │                  │                     │                  │
│             ▼                  ▼                     ▼                  │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                     DISTANCE LAYER (SIMD)                        │   │
│  ├─────────────────────────────────────────────────────────────────┤   │
│  │  Cosine  │  Euclidean  │  Dot Product  │  Hamming  │  Jaccard   │   │
│  │  (76ns)  │   (47ns)    │    (45ns)     │   (6ns)   │   (SIMD)   │   │
│  │                                                                  │   │
│  │  AVX2/AVX-512 │ WASM SIMD128 │ Auto-vectorization │ Fallback   │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                                                          │
└────────────────────────────────────┬────────────────────────────────────┘
                                     │
                                     ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         STORAGE LAYER                                    │
├─────────────────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌────────────────┐ │
│  │ Vector Data │  │   Payload   │  │     WAL     │  │  Binary Export │ │
│  │  (mmap)     │  │   Storage   │  │  (durability│  │  (VELS format) │ │
│  └─────────────┘  └─────────────┘  └─────────────┘  └────────────────┘ │
│                                                                          │
│  File System / Memory / IndexedDB (WASM)                                │
└─────────────────────────────────────────────────────────────────────────┘
```

## Component Details

### 1. Client Layer

| Component | Language | Purpose |
|-----------|----------|---------|
| **TypeScript SDK** | TypeScript | Unified client for browser/Node.js |
| **Python SDK** | Python | Native bindings via PyO3 |
| **REST Client** | Any | HTTP API access |
| **VelesQL CLI** | Rust | Interactive query REPL |

### 2. API Layer

#### velesdb-wasm
- WebAssembly module for browser/Node.js
- SIMD128 optimized distance calculations
- IndexedDB persistence via binary export/import
- ~50KB gzipped

#### velesdb-server
- Axum-based REST API server
- OpenAPI/Swagger documentation
- 11 REST endpoints
- Prometheus metrics (planned)

#### velesdb-python
- PyO3 bindings for Python
- NumPy array support
- Zero-copy when possible

### 3. Core Engine (velesdb-core)

#### Database
- Collection management
- Multi-collection support
- Automatic persistence

#### Collection
- Point CRUD operations
- Vector search (single & batch)
- Text search (BM25)
- Hybrid search (vector + text)

#### VelesQL Parser
- SQL-like query language
- ~1.3M queries/sec parsing
- Bound parameters support

#### Filter Engine
- ColumnStore-based filtering
- RoaringBitmap for set operations
- 122x faster than JSON filtering

### 4. Index Layer

#### HNSW Index
```
                    Entry Point (Layer L)
                          │
            ┌─────────────┼─────────────┐
            ▼             ▼             ▼
         Node A ─────── Node B ─────── Node C   (Layer L-1)
            │             │             │
    ┌───────┼───────┐     │     ┌───────┼───────┐
    ▼       ▼       ▼     ▼     ▼       ▼       ▼
   ...     ...     ...   ...   ...     ...     ... (Layer 0)
```

- **Parameters**:
  - `M`: Max connections per node (default: 16)
  - `ef_construction`: Build-time search width (default: 100)
  - `ef_search`: Query-time search width (default: 50)

- **Features**:
  - Thread-safe parallel insertions
  - Automatic level assignment
  - Persistent storage with WAL recovery

#### BM25 Index
- Term frequency / inverse document frequency
- Tokenization with stopword removal
- Persistent storage

#### ColumnStore
- Columnar storage for typed metadata
- String interning for efficient comparisons
- RoaringBitmap for fast set operations

### 5. Distance Layer (SIMD)

| Metric | Implementation | Latency (768D) |
|--------|---------------|----------------|
| Cosine | AVX2 FMA | **76 ns** |
| Euclidean | AVX2 FMA | **47 ns** |
| Dot Product | AVX2 FMA | **45 ns** |
| Hamming | POPCNT | **6 ns** |
| Jaccard | Auto-vectorized | ~100 ns |

**SIMD Strategy**:
1. **Native (x86_64)**: AVX2/AVX-512 via `wide` crate
2. **WASM**: SIMD128 (128-bit vectors)
3. **Fallback**: Scalar with loop unrolling

### 6. Storage Layer

#### Vector Data
- Memory-mapped files for large datasets
- Contiguous f32 buffer for cache locality
- Lazy loading support

#### Payload Storage
- JSON-based payload storage
- Nested field access with dot notation
- Type-aware indexing

#### WAL (Write-Ahead Log)
- Durability guarantees
- Automatic recovery on restart
- Configurable sync policy

#### Binary Export (WASM)
```
┌────────┬─────────┬───────────┬────────┬─────────┬─────────────────────┐
│ "VELS" │ Version │ Dimension │ Metric │  Count  │      Vectors        │
│ 4 bytes│ 1 byte  │  4 bytes  │ 1 byte │ 8 bytes │ (id + data) × count │
└────────┴─────────┴───────────┴────────┴─────────┴─────────────────────┘
```

## Data Flow

### Vector Search Flow

```
Query Vector
     │
     ▼
┌─────────────────┐
│  VelesQL Parse  │ (optional)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Filter Engine  │ (if filters present)
│  (ColumnStore)  │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   HNSW Search   │
│  (entry → L0)   │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  SIMD Distance  │
│  Calculations   │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Top-K Results  │
│  (min-heap)     │
└────────┬────────┘
         │
         ▼
   Sorted Results
```

### Hybrid Search Flow

```
Query Vector + Text Query
         │
    ┌────┴────┐
    ▼         ▼
┌───────┐ ┌───────┐
│ HNSW  │ │ BM25  │
│Search │ │Search │
└───┬───┘ └───┬───┘
    │         │
    ▼         ▼
┌─────────────────┐
│  RRF Fusion     │
│ (Reciprocal     │
│  Rank Fusion)   │
└────────┬────────┘
         │
         ▼
   Merged Results
```

## Performance Characteristics

### Memory Usage

| Component | Per Vector (768D) |
|-----------|-------------------|
| Vector Data (f32) | 3,072 bytes |
| Vector Data (f16) | 1,536 bytes |
| Vector Data (SQ8) | 768 bytes |
| HNSW Links | ~256 bytes |
| Payload (avg) | ~200 bytes |

### Throughput

| Operation | Throughput |
|-----------|------------|
| Insert | ~50K vec/sec |
| Search (10K vectors) | ~1ms |
| Search (100K vectors) | ~10ms |
| VelesQL Parse | 1.3M queries/sec |
| Export (WASM) | 4,479 MB/s |
| Import (WASM) | 2,943 MB/s |

## Platform Support

| Platform | Status | Notes |
|----------|--------|-------|
| Linux x86_64 | ✅ Full | AVX2/AVX-512 |
| macOS x86_64 | ✅ Full | AVX2 |
| macOS ARM64 | ✅ Full | NEON |
| Windows x86_64 | ✅ Full | AVX2 |
| WASM (Browser) | ✅ Full | SIMD128 |
| WASM (Node.js) | ✅ Full | SIMD128 |

## Future Architecture

### Planned Components

```
┌─────────────────────────────────────────────────────────────────────────┐
│                       DISTRIBUTED LAYER (v1.0+)                          │
├─────────────────────────────────────────────────────────────────────────┤
│   Coordinator   │   Sharding   │   Replication   │   Consensus (Raft)  │
└─────────────────────────────────────────────────────────────────────────┘
```

- **Product Quantization (PQ)**: 8-32x compression
- **Sparse Vectors**: For hybrid sparse-dense search
- **GPU Acceleration**: CUDA kernels for large-scale
