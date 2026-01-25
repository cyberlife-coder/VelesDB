# VelesDB Architecture

This document describes the internal architecture of VelesDB.

## High-Level Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           CLIENT LAYER                                   │
├─────────────────────────────────────────────────────────────────────────┤
│  TypeScript SDK │ Python SDK │ REST Client │ VelesQL CLI │ Mobile SDK  │
│  (@velesdb/sdk) │ (velesdb)  │ (curl/HTTP) │ (velesdb)   │ (iOS/Android)│
└───────┬─────────┴──────┬─────┴───────┬─────┴──────┬──────┴──────┬──────┘
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
│  │  (81ns)  │   (49ns)    │    (39ns)     │   (6ns)   │   (SIMD)   │   │
│  │                                                                  │   │
│  │  AVX2/AVX-512 │ WASM SIMD128 │ Auto-vectorization │ Fallback   │   │
│  │               │              │                    │ (ARM64)    │   │
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
| **Mobile SDK** | Swift/Kotlin | Native iOS and Android bindings via UniFFI |
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

#### velesdb-mobile
- UniFFI bindings for iOS (Swift) and Android (Kotlin)
- Thread-safe `Arc`-wrapped handles
- StorageMode support (Full, SQ8, Binary) for IoT/Edge
- Targets: `aarch64-apple-ios`, `aarch64-linux-android`, etc.

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

#### VelesQL Parser (v2.0)
- SQL-like query language
- ~1.3M queries/sec parsing
- Bound parameters support
- **v2.0 Features**:
  - `GROUP BY` / `HAVING` (AND/OR)
  - `ORDER BY` (multi-column, similarity)
  - `JOIN` with aliases
  - `UNION` / `INTERSECT` / `EXCEPT`
  - `USING FUSION` (hybrid search)
  - `WITH` (max_groups, group_limit)

#### Filter Engine
- ColumnStore-based filtering
- RoaringBitmap for set operations
- 122x faster than JSON filtering

### 4. Knowledge Graph Layer (EPIC-019)

```
┌─────────────────────────────────────────────────────────────────────────┐
│                      KNOWLEDGE GRAPH ENGINE                              │
├─────────────────────────────────────────────────────────────────────────┤
│  ┌──────────────────┐  ┌──────────────────┐  ┌────────────────────────┐ │
│  │   GraphSchema    │  │    GraphNode     │  │      GraphEdge         │ │
│  │  (labels, types) │  │ (id, properties) │  │ (src, tgt, label, props)│ │
│  └────────┬─────────┘  └────────┬─────────┘  └───────────┬────────────┘ │
│           │                     │                        │              │
│           ▼                     ▼                        ▼              │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                    ConcurrentEdgeStore                           │   │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │   │
│  │  │  256 Shards │  │  edge_ids   │  │    Label Indices        │  │   │
│  │  │ (RwLock<    │  │  HashMap    │  │  by_label, outgoing_    │  │   │
│  │  │  EdgeStore>)│  │ (edge→src)  │  │  by_label               │  │   │
│  │  └──────┬──────┘  └──────┬──────┘  └───────────┬─────────────┘  │   │
│  │         │                │                     │                 │   │
│  │         ▼                ▼                     ▼                 │   │
│  │  ┌──────────────────────────────────────────────────────────┐   │   │
│  │  │  Optimized Operations:                                    │   │   │
│  │  │  • add_edge: O(1) with cross-shard dual-insert           │   │   │
│  │  │  • remove_edge: O(1) 2-shard lookup (not 256)            │   │   │
│  │  │  • get_edges_by_label: O(k) via label index              │   │   │
│  │  └──────────────────────────────────────────────────────────┘   │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                                                          │
│  ┌──────────────────┐  ┌──────────────────┐  ┌────────────────────────┐ │
│  │   LabelTable     │  │   BfsIterator    │  │    GraphMetrics        │ │
│  │ String interning │  │ Streaming BFS    │  │  LatencyHistogram      │ │
│  │  LabelId (u32)   │  │ memory-bounded   │  │  node/edge counters    │ │
│  └──────────────────┘  └──────────────────┘  └────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────┘
```

**Scalability (10M+ edges)**:
- Adaptive sharding: 1-512 shards based on graph size
- 2-shard removal: O(1) instead of O(256) lock acquisitions
- Label indices: O(k) edge lookup by relationship type
- String interning: ~60% memory reduction for labels

### 5. Index Layer

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
| Dot Product | AVX2 FMA | **39 ns** |
| Euclidean | AVX2 FMA | **49 ns** |
| Cosine | AVX2 FMA | **81 ns** |
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

### VelesQL v2.0 Query Flow

```
┌─────────────────────────────────────────────────────────────────┐
│                      VelesQL v2.0 Parser                         │
├─────────────────────────────────────────────────────────────────┤
│  SQL Query                                                       │
│    │                                                             │
│    ▼                                                             │
│  ┌────────────────┐                                              │
│  │  Pest Grammar  │  compound_query → select_stmt [set_op]       │
│  └────────┬───────┘                                              │
│           │                                                      │
│           ▼                                                      │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │                        AST                                  │ │
│  ├────────────────────────────────────────────────────────────┤ │
│  │  Query {                                                    │ │
│  │    select: SelectStatement {                                │ │
│  │      columns, from, joins[], where_clause,                  │ │
│  │      group_by, having, order_by, limit, offset,             │ │
│  │      with_clause, fusion_clause                             │ │
│  │    },                                                       │ │
│  │    compound: Option<CompoundQuery> {                        │ │
│  │      operator: UNION|INTERSECT|EXCEPT,                      │ │
│  │      right: SelectStatement                                 │ │
│  │    }                                                        │ │
│  │  }                                                          │ │
│  └────────────────────────────────────────────────────────────┘ │
│           │                                                      │
│           ▼                                                      │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │                    Execution Engine                         │ │
│  ├────────────────────────────────────────────────────────────┤ │
│  │  1. Filter Pushdown → ColumnStore                           │ │
│  │  2. Vector Search → HNSW (if NEAR clause)                   │ │
│  │  3. JOIN Execution → Cross-collection merge                 │ │
│  │  4. Aggregation → GROUP BY + HAVING                         │ │
│  │  5. Ordering → ORDER BY (columns, similarity)               │ │
│  │  6. Set Operations → UNION/INTERSECT/EXCEPT                 │ │
│  │  7. Fusion → RRF/Weighted/Maximum (if USING FUSION)         │ │
│  └────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

**VelesQL v2.0 Supported Syntax:**

```sql
-- Aggregation with GROUP BY and HAVING
SELECT category, COUNT(*), AVG(price) 
FROM products 
GROUP BY category 
HAVING COUNT(*) > 5 AND AVG(price) > 50

-- ORDER BY with similarity function
SELECT * FROM docs 
ORDER BY similarity(vector, $query) DESC 
LIMIT 10

-- JOIN across collections
SELECT * FROM orders 
JOIN customers AS c ON orders.customer_id = c.id
WHERE status = 'active'

-- Set operations
SELECT * FROM active_users UNION SELECT * FROM archived_users

-- Hybrid fusion search
SELECT * FROM documents 
USING FUSION(strategy='rrf', k=60)
LIMIT 20
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

| Platform | Status | SIMD | Performance |
|----------|--------|------|-------------|
| Linux x86_64 | ✅ Full | AVX2/AVX-512 | 100% |
| Windows x86_64 | ✅ Full | AVX2 | 100% |
| macOS x86_64 | ✅ Full | AVX2 | 100% |
| **macOS ARM64** | ✅ Full | **Fallback** | **~80%** |
| WASM (Browser) | ✅ Full | SIMD128 | ~70% |
| WASM (Node.js) | ✅ Full | SIMD128 | ~70% |
| **iOS (ARM64)** | ✅ Full | NEON | ~90% |
| **Android (ARM64)** | ✅ Full | NEON | ~90% |
| **Android (ARMv7)** | ✅ Full | Fallback | ~70% |

### ARM64 (Apple Silicon) Note

On macOS ARM64 (M1/M2/M3), VelesDB uses a **scalar fallback** for distance calculations 
instead of SIMD. This is because the underlying `hnsw_rs` library's `simdeez` feature 
only supports x86 SIMD instructions (AVX2/SSE).

**Impact:**
- Distance calculations are ~20% slower than x86_64 with AVX2
- All other operations (indexing, storage, queries) are unaffected
- Overall search latency remains in the microsecond range

**Technical details:**
```toml
# Cargo.toml - Conditional compilation
[dependencies.hnsw_rs]
version = "0.3"
default-features = false  # ARM64: no SIMD

[target.'cfg(target_arch = "x86_64")'.dependencies.hnsw_rs]
features = ["simdeez_f"]   # x86_64: AVX2/SSE enabled
```

**Future:** NEON SIMD support for ARM64 may be added when `hnsw_rs` upstream supports it.

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

### v0.9.0 Architecture Improvements (Q1 2026)

Based on the technical audit (January 2026), the following architectural changes are planned:

| Change | Current | Target |
|--------|---------|--------|
| **Concurrency** | Global `RwLock<HashMap>` | `DashMap` + 16-shard storage |
| **Memory** | `Vec<f32>` allocations per read | Zero-copy `&[f32]` from mmap |
| **SIMD Dispatch** | Per-call feature detection | `OnceLock` function pointer |
| **Unsafe** | `'static` lifetime tricks | Safe self-referential via `ouroboros` |

**Expected Impact**:
- Insert throughput: 50k/s → 150k/s (16 threads)
- Search p99 latency: 15ms → 10ms
- Allocations per search: ~10k → 0

See `docs/internal/TECHNICAL_AUDIT_PLAN.md` for full details.
