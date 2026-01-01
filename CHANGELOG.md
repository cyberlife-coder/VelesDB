# Changelog

All notable changes to VelesDB will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.7.1] - 2026-01-01

### ⚡ SIMD Performance Optimization

#### Added

- **32-wide SIMD unrolling** - 4x f32x8 accumulators for maximum ILP
  - `cosine_similarity_fast`: **-12% latency** (768D: 90ns → 79ns)
  - `dot_product_fast`: **-17% latency** (768D: 54ns → 45ns)
  - `euclidean_distance_fast`: **-15% latency**

- **Pre-normalized vector functions** - Fast path for unit vectors
  - `cosine_similarity_normalized()`: **~40% faster** than standard cosine
  - `batch_cosine_normalized()`: Batch with CPU prefetch hints
  - Skips norm computation when vectors are already normalized

- **Benchmark dimensions expanded** - OpenAI embedding support
  - Added 1536D (text-embedding-3-small) to all benchmarks
  - Added 3072D (text-embedding-3-large) to all benchmarks

#### Performance Summary (768D vectors)

| Function | Before | After | Improvement |
|----------|--------|-------|-------------|
| cosine_similarity | 90ns | 79ns | **-12%** |
| dot_product | 54ns | 45ns | **-17%** |
| euclidean | 55ns | 47ns | **-15%** |
| cosine_normalized | N/A | 45ns | **New** |

#### Files Modified

- `src/simd.rs` - Switched to 32-wide optimized implementations
- `src/simd_avx512.rs` - Added `cosine_similarity_normalized`, `batch_cosine_normalized`
- `benches/*.rs` - Added dimensions 1536, 3072

---

## [0.7.0] - 2026-01-01

### 📱 Mobile SDK - iOS & Android

VelesDB now supports native mobile platforms via UniFFI bindings.

#### Added

- **velesdb-mobile crate** - Native bindings for iOS (Swift) and Android (Kotlin)
  - UniFFI-based FFI generation
  - `VelesDatabase` and `VelesCollection` objects
  - Full CRUD operations (upsert, search, delete)
  - Thread-safe, `Arc`-wrapped handles

- **StorageMode for IoT/Edge** - Memory optimization for constrained devices
  - `Full`: Best recall, 4 bytes/dimension
  - `Sq8`: 4x compression, ~1% recall loss (recommended for mobile)
  - `Binary`: 32x compression, ~5-10% recall loss (extreme IoT)

- **Distance Metrics** - All 5 metrics supported
  - Cosine, Euclidean, Dot Product, Hamming, Jaccard

- **GitHub Actions CI** - `mobile-build.yml` workflow
  - iOS targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `x86_64-apple-ios`
  - Android targets: `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android`
  - UniFFI binding generation (Swift/Kotlin)

#### Documentation

- `crates/velesdb-mobile/README.md` - Complete integration guide
  - Swift quick start
  - Kotlin quick start
  - Build instructions for iOS/Android
  - API reference with all methods
  - Memory footprint table

#### Crate Coherence

- All crates aligned on workspace version `0.7.0`
- All crates using ELv2 license (`license-file`)
- All inter-crate dependencies with explicit versions
- Authors aligned on workspace (`VelesDB Team`)

---

## [0.5.2] - 2025-12-30

### 🎯 Quantization & Integrations

#### Added
- **SQ8 SIMD Distance Functions** - AVX2-optimized dot product, Euclidean, cosine for quantized vectors
  - `dot_product_quantized_simd()` - ~1.7x faster than scalar
  - `euclidean_squared_quantized_simd()`
  - `cosine_similarity_quantized_simd()`
- **StorageMode API** - Configurable vector storage at collection creation
  - `POST /collections` now accepts `storage_mode`: `full`, `sq8`, `binary`
  - `db.create_collection_with_options(name, dim, metric, StorageMode::SQ8)`
- **LlamaIndex Integration** - `llamaindex-velesdb` Python package
  - `VelesDBVectorStore` compatible with LlamaIndex pipelines
  - Full test suite and documentation
- **Quantization Benchmarks** - Criterion benchmarks for SQ8 performance
- **4 New E2E Tests** - API tests for storage_mode functionality

#### Documentation
- `docs/QUANTIZATION.md` - Complete French guide for SQ8/Binary quantization
- Updated README.md with quantization section (English)
- Updated `simd_explicit.rs` docs for ARM NEON/WASM support

#### Performance
- **SQ8 Memory**: 4x reduction (768D: 3KB → 770 bytes)
- **Binary Memory**: 32x reduction (768D: 3KB → 96 bytes)
- **No performance regression** on existing SIMD operations

---

## [0.5.1] - 2025-12-30

### 🔐 On-Premises & Documentation

#### Added
- **On-Premises Deployment section** in README - Data sovereignty, air-gap, GDPR/HIPAA compliance
- **P0: Parallel batch search** - `search_batch_parallel` using Rayon for multi-query workloads
- **P1: HNSW prefetch hints** - CPU cache warming during re-ranking phase

#### Changed
- **Simplified BENCHMARKS.md** - Reduced from 430 to 96 lines, focus on key metrics
- **Updated competition table** - Clearer differentiation vs pgvector/Qdrant/Pinecone
- **Version bump to 0.5.1** - All crates and documentation updated

---

## [0.5.0] - 2025-12-29

### 🚀 Performance - 3.2x Faster Than pgvector

Major HNSW insertion optimization making VelesDB significantly faster than pgvector for batch imports.

#### Benchmark Results (5,000 vectors, 768D, Docker)

| Metric | pgvector | VelesDB | Result |
|--------|----------|---------|--------|
| **Insert + Index** | 8.54s | **2.63s** | **3.2x faster** |
| **Recall@10** | 100.0% | 99.7% | Comparable |
| **Search P50** | 3.0ms | 4.0ms | Comparable |

### Added

#### SIMD-Accelerated HNSW Insertion
- **`simdeez_f` feature enabled** for hnsw_rs - AVX2/SSE SIMD distance calculations
- **`parallel_insert`** - Native parallel HNSW graph construction using Rayon
- **`HnswParams::fast()`** - New constructor for pgvector-compatible settings (m=16, ef=200)

#### Async-Safe Server
- **`spawn_blocking`** wrapper for bulk operations - Prevents blocking the Tokio runtime
- **100MB body limit** - Support for large batch uploads via REST API

### Changed

#### HNSW Parameters Aligned with pgvector
- 768D vectors: m=16, ef_construction=200 (was m=24, ef=400)
- Optimized for insertion speed while maintaining >99% recall
- Added `HnswParams::high_recall()` for quality-critical use cases

#### Benchmark Methodology
- Fair comparison: Both databases measured with insert + index time
- pgvector index build time now included in total measurement
- Standardized batch sizes for equitable comparison

### Fixed

- **Async/blocking deadlock** - `upsert_bulk()` no longer blocks async runtime
- **HTTP 413 errors** - Increased body size limit for large batches
- **HNSW insertion blocking** - Replaced sequential insertion with parallel

### Performance Notes

The 3.2x speedup over pgvector is achieved through:
1. **Parallel HNSW insertion** - Utilizes all CPU cores during graph construction
2. **SIMD distance calculations** - AVX2/SSE acceleration in hnsw_rs
3. **Deferred index save** - No disk I/O during batch insertion
4. **Optimized parameters** - pgvector-compatible m=16, ef=200

---

## [0.4.1] - 2025-12-29

### Added

#### Python SDK - Bulk Import Optimization
- **`upsert_bulk()` method** - 7x faster bulk imports
  - Parallel HNSW insertion using Rayon
  - Single flush at the end (no per-batch I/O)
  - 3,300 vectors/sec on 768D embeddings

#### Benchmark Kit
- **`benchmarks/` directory** - Reproducible VelesDB vs pgvectorscale benchmark
  - `benchmark.py` - Full comparison script
  - `benchmark_quick.py` - VelesDB-only quick test
  - `docker-compose.yml` - pgvectorscale container setup
  - Detailed methodology documentation

### Performance Results (10k vectors, 768D)

| Metric | pgvectorscale | VelesDB | Speedup |
|--------|---------------|---------|---------|
| Total Ingest | 22.3s | **3.0s** | **7.4x** |
| Avg Latency | 52.8ms | **4.0ms** | **13x** |
| Throughput | 18.9 QPS | **246.8 QPS** | **13x** |

### Documentation
- Updated README with pgvectorscale benchmark results
- Added `upsert_bulk()` documentation to Python SDK
- Updated `docs/BENCHMARKS.md` with competitor comparison

---

## [0.4.0] - 2025-12-24

### 🎉 License Change - Elastic License 2.0 (ELv2)

VelesDB Core is now licensed under **Elastic License 2.0 (ELv2)** — a **source-available** license.

#### What this means:
- ✅ **Free to use** for any purpose (commercial or personal)
- ✅ **Free to modify** and create derivative works
- ✅ **Free to distribute** with your applications
- ❌ **Cannot provide as a managed service** (DBaaS) without permission

This change ensures VelesDB remains freely available while protecting against cloud providers offering it as a competing service.

### Changed
- Updated all license references from BSL-1.1 to ELv2
- Updated all documentation to use "source-available" terminology
- Updated license badges across all README files
- Updated OpenAPI documentation with correct license

---

## [0.3.8] - 2025-12-23

### Added

#### RAG PDF Demo
- **Complete RAG demo** in `demos/rag-pdf-demo/`
  - PDF upload and text extraction (PyMuPDF)
  - Multilingual embeddings (`paraphrase-multilingual-MiniLM-L12-v2`, 384 dims)
  - Semantic search with VelesDB
  - FastAPI backend with real-time performance metrics
  - Modern UI with Tailwind CSS
  - 21 TDD tests with pytest

#### Performance Benchmarks (500 iterations)
- **VelesDB Search**: 0.89ms mean (P95: 1.45ms)
- **Full API Search**: 19.10ms mean (embed + search)
- **HTTP persistent client**: 0.61ms vs 6.41ms (10x faster)

#### MSI Installer
- RAG PDF Demo now included in Windows installer
- New "Demos" feature in installer with complete Python demo

### Changed
- Updated benchmark documentation with layer-by-layer latency analysis
- Optimized VelesDB client with persistent HTTP connection

---

## [0.3.2] - 2025-12-23

### Added

#### Production Installers
- **Windows MSI Installer** - One-click installation with feature selection
  - VelesDB Server + CLI binaries
  - Optional PATH integration (enabled by default)
  - Documentation and examples included
  - Silent install support: `msiexec /i velesdb.msi /quiet ADDTOPATH=1`

- **Linux DEB Package** - Native Debian/Ubuntu package
  - Installs to `/usr/bin/velesdb` and `/usr/bin/velesdb-server`
  - Documentation in `/usr/share/doc/velesdb/`
  - Tauri RAG example included

#### Documentation
- **[INSTALLATION.md](docs/INSTALLATION.md)** - Complete installation guide
  - All platforms: Windows, Linux, Docker, Python, Rust, WASM
  - Configuration options and environment variables
  - Data persistence explained
  - Troubleshooting guide

### Changed
- README.md Quick Start section reorganized with installers first
- Release workflow now builds `.msi` and `.deb` installers automatically

### Fixed
- **CI**: Added GTK dependencies (`libglib2.0-dev`, `libgtk-3-dev`, `libwebkit2gtk-4.1-dev`) for Tauri plugin builds on Linux
- **Security Audit**: Fixed GitHub Actions permissions error with `rustsec/audit-check`

---

## [0.3.1] - 2025-12-23

### Added

#### Performance Optimizations (P1)
- **ContiguousVectors**: Cache-optimized memory layout for vector storage
  - 64-byte cache-line aligned allocation
  - 40% faster random access vs `Vec<Vec<f32>>`
  - Batch operations with SIMD acceleration

- **CPU Prefetch Hints**: Hardware prefetch for HNSW traversal
  - +12% throughput on neighbor traversal
  - Configurable prefetch distance

- **Batch WAL Write**: Optimized bulk import
  - 10x improvement for large batch inserts
  - Reduced I/O overhead

### Performance

| Mode | Recall@10 | Improvement |
|------|-----------|-------------|
| Balanced | 98.2% | +0.5% |
| Accurate | 99.4% | +0.3% |
| HighRecall | 99.6% | +0.2% |

---

## [0.1.0] - 2025-12-19

### Added

#### Core Engine
- **HNSW Index**: High-performance approximate nearest neighbor search
  - Configurable `M` and `ef_construction` parameters
  - Support for Cosine, Euclidean, and Dot Product metrics
  - Thread-safe parallel insertions with `insert_batch_parallel`
  - Persistence with automatic recovery

- **SIMD Optimizations**: Hardware-accelerated distance calculations
  - 2-3x speedup for vector operations
  - Automatic fallback for non-SIMD platforms

- **Scalar Quantization**: Memory-efficient vector storage
  - INT8 quantization with 4x memory reduction
  - Configurable storage modes (Full, Quantized, Hybrid)

- **Metadata Filtering**: Rich query capabilities
  - Operators: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `in`, `contains`, `is_null`
  - Logical operators: `and`, `or`, `not`
  - Nested payload access with dot notation

#### VelesQL Query Language
- **SQL-like Syntax**: Familiar query interface
  ```sql
  SELECT * FROM documents 
  WHERE vector NEAR $query_vector
    AND category = 'tech'
  LIMIT 10
  ```
- **Features**:
  - Vector search with `NEAR` clause
  - Distance metrics: `COSINE`, `EUCLIDEAN`, `DOT`
  - Bound parameters: `$param_name`
  - Comparison operators: `=`, `!=`, `>`, `<`, `>=`, `<=`
  - `IN`, `BETWEEN`, `LIKE`, `IS NULL` / `IS NOT NULL`
  - Logical operators: `AND`, `OR`

#### REST API Server
- **Collections API**:
  - `POST /collections` - Create collection
  - `GET /collections` - List collections
  - `GET /collections/{name}` - Get collection info
  - `DELETE /collections/{name}` - Delete collection

- **Points API**:
  - `POST /collections/{name}/points` - Upsert points
  - `GET /collections/{name}/points/{id}` - Get point
  - `DELETE /collections/{name}/points/{id}` - Delete point

- **Search API**:
  - `POST /collections/{name}/search` - Vector search
  - `POST /collections/{name}/search/batch` - Batch search

- **VelesQL API**:
  - `POST /query` - Execute VelesQL queries

### Performance

| Operation | Metric | Value |
|-----------|--------|-------|
| Vector Search (768d) | Latency p50 | < 1ms |
| SIMD Cosine | Speedup | 2.3x |
| SIMD Euclidean | Speedup | 2.1x |
| VelesQL Parse (simple) | Throughput | 1.3M queries/sec |
| VelesQL Parse (complex) | Throughput | 200K queries/sec |

### Testing

- **171 tests** total
  - 162 core engine tests
  - 9 REST API integration tests
- **90%+ code coverage**

---

## [0.2.0] - 2025-12-20

### Added

#### Python Bindings (PyO3)
- **Native Python API**: Full-featured Python bindings for VelesDB
  - `velesdb.Database` - Database management
  - `velesdb.Collection` - Collection operations (upsert, search, delete)
  - Support for Python lists and NumPy arrays
  - Automatic `float64` → `float32` conversion

- **NumPy Integration** (WIS-23):
  - Direct support for `numpy.ndarray` in `upsert()` and `search()`
  - Zero-copy when possible for performance
  - Mixed Python list / NumPy array in same batch

#### VelesQL CLI/REPL (WIS-19)
- **Interactive REPL**: `velesdb-cli repl`
  - Syntax highlighting
  - Command history
  - Tab completion
- **Single Query Mode**: `velesdb-cli query "SELECT ..."`
- **Database Info**: `velesdb-cli info ./data`

#### LangChain Integration (WIS-30)
- **`langchain-velesdb` package**: LangChain VectorStore adapter
  - `VelesDBVectorStore` class
  - `add_texts()`, `similarity_search()`, `delete()`
  - `as_retriever()` for RAG pipelines
  - Full test suite (9 tests)

#### Additional Distance Metrics (WIS-33)
- **Hamming Distance**: For binary vectors and locality-sensitive hashing
  - Ultra-fast bit comparison (XOR + popcount)
  - Ideal for: image hashing, fingerprints, duplicate detection
  - Values > 0.5 treated as 1, else 0

- **Jaccard Similarity**: For set-like vectors
  - Measures intersection over union of non-zero elements
  - Ideal for: recommendations, tags, document similarity
  - Returns 1.0 for identical sets, 0.0 for disjoint sets

- **SIMD-Optimized**: Loop unrolling (4x) for auto-vectorization

### Performance

| Operation | Metric | Value |
|-----------|--------|-------|
| Python upsert (1000 vectors) | Throughput | ~50K vec/sec |
| Python search (768d) | Latency | < 2ms |
| VelesQL CLI parse | Throughput | 1.3M queries/sec |

---

## [0.1.2] - 2025-12-21

### Added

#### Performance Optimizations (WIS-44)
- **Explicit SIMD** (WIS-47): 4.2x faster cosine similarity using `wide` crate
  - Cosine: 320ns → **76ns** (4.2x speedup)
  - Euclidean: 138ns → **47ns** (2.9x speedup)
  - Dot Product: 130ns → **45ns** (2.9x speedup)

- **ColumnStore Filtering** (WIS-46): 122x faster metadata filtering
  - Columnar storage for typed metadata (i64, f64, string, bool)
  - String interning for efficient string comparisons
  - RoaringBitmap for combining filters (AND/OR)

- **Binary Hamming Distance**: ~6ns per operation (164M ops/sec)

#### Developer Experience
- **One-liner Installers**: 
  - Linux/macOS: `curl -fsSL .../install.sh | bash`
  - Windows: `irm .../install.ps1 | iex`

- **OpenAPI/Swagger** (WIS-34): Full API documentation
  - Swagger UI at `/swagger-ui`
  - OpenAPI spec at `/api-docs/openapi.json`

- **Python Bindings**: Hamming & Jaccard metric support

#### Documentation
- Updated all README files with new performance metrics
- Added BENCHMARKING_GUIDE.md for reproducible benchmarks
- Added PERFORMANCE_ROADMAP.md

### Performance

| Operation | Time (768d) | Throughput |
|-----------|-------------|------------|
| Cosine Similarity | **76 ns** | 13M ops/sec |
| Euclidean Distance | **47 ns** | 21M ops/sec |
| Hamming (Binary) | **6 ns** | 164M ops/sec |
| ColumnStore Filter | **27 µs** | 122x vs JSON |

---

## [0.1.4] - 2025-12-21

### Added

#### Half-Precision Support (WIS-61)
- **f16/bf16 vectors**: 50% memory reduction
  - `VectorPrecision` enum: F32, F16, BF16
  - `VectorData` with automatic conversions
  - SIMD-optimized distance calculations
  - 24 TDD tests

| Dimension | f32 Size | f16 Size | Savings |
|-----------|----------|----------|---------|
| 768 (BERT)| 3.0 KB   | 1.5 KB   | 50%     |
| 1536 (GPT)| 6.0 KB   | 3.0 KB   | 50%     |

#### WASM Support (WIS-60)
- **`velesdb-wasm` crate**: Vector search in the browser
  - `VectorStore` with insert/search/remove
  - Cosine, Euclidean, Dot Product metrics
  - WASM SIMD128 optimizations via `wide` crate
  - JavaScript API via wasm-bindgen

#### AVX-512 Optimizations (WIS-59)
- **wide32 processing**: 4x f32x8 accumulators for maximum ILP
  - 40-50% improvement on HNSW recall benchmarks
  - Automatic CPU feature detection

### Performance

| Operation | Time (768d) | Speedup |
|-----------|-------------|---------|
| Dot Product | **42 ns** | 6.8x vs baseline |
| Normalize | **209 ns** | 2x vs baseline |
| HNSW Recall | **115 ms** | 45% faster |

---

## [0.2.0] - 2025-12-22

### Added

#### BM25 Full-Text Search (WIS-55)
- **`Bm25Index`**: Full-text search with BM25 ranking algorithm
  - Tokenization with stopword removal
  - Term frequency / inverse document frequency scoring
  - Persistent storage with automatic recovery
  - 15+ TDD tests

- **`Collection::text_search()`**: Search by text content
- **`Collection::hybrid_search()`**: Combined vector + BM25 with RRF fusion
  - Configurable `vector_weight` parameter (0.0-1.0)
  - Reciprocal Rank Fusion for result merging

- **VelesQL MATCH clause**:
  ```sql
  SELECT * FROM documents 
  WHERE content MATCH 'rust programming'
  LIMIT 10
  ```

- **REST API Endpoints**:
  - `POST /collections/{name}/search/text` - BM25 text search
  - `POST /collections/{name}/search/hybrid` - Hybrid search

#### Tauri Desktop Plugin (WIS-67)
- **`tauri-plugin-velesdb`**: Vector search in desktop applications
  - Full Tauri v2 compatibility
  - 9 commands: CRUD, search, text_search, hybrid_search, query
  - TypeScript bindings with full type definitions
  - Auto-generated Tauri permissions
  - 26 TDD tests

- **Commands**:
  | Command | Description |
  |---------|-------------|
  | `create_collection` | Create vector collection |
  | `delete_collection` | Delete collection |
  | `list_collections` | List all collections |
  | `get_collection` | Get collection info |
  | `upsert` | Insert/update vectors |
  | `search` | Vector similarity search |
  | `text_search` | BM25 full-text search |
  | `hybrid_search` | Vector + text fusion |
  | `query` | Execute VelesQL |

- **JavaScript API**:
  ```javascript
  import { invoke } from '@tauri-apps/api/core';
  
  await invoke('plugin:velesdb|search', {
    request: { collection: 'docs', vector: [...], topK: 10 }
  });
  ```

### Performance

| Operation | Latency | Throughput |
|-----------|---------|------------|
| Text search (10k docs) | < 5ms | 200 q/s |
| Hybrid search | < 10ms | 100 q/s |
| Tauri vector search | < 1ms | 1000 q/s |

### Testing

- **374 tests** total (+48 from v0.1.4)
  - 333 core engine tests
  - 26 Tauri plugin tests
  - 6 REST API tests
  - 9 WASM tests

---

## [0.3.0] - 2025-12-22

### Added

#### TypeScript SDK (WIS-71)
- **`@velesdb/sdk`**: Unified TypeScript client for browser and Node.js
  - WASM backend for client-side vector search
  - REST backend for server communication
  - Full type definitions with strict TypeScript
  - Error handling with custom exception classes
  - 61 comprehensive tests

- **API**:
  ```typescript
  import { VelesDB } from '@velesdb/sdk';
  
  const db = new VelesDB({ backend: 'wasm' });
  await db.init();
  await db.createCollection('docs', { dimension: 768 });
  await db.insert('docs', { id: '1', vector: [...] });
  const results = await db.search('docs', query, { k: 5 });
  ```

#### IndexedDB Persistence (WIS-73)
- **`export_to_bytes()`**: Serialize vector store to binary format
- **`import_from_bytes()`**: Restore from binary data
- Custom binary format with "VELS" magic number, versioning
- Perfect for IndexedDB, localStorage, file downloads

- **Performance** (after optimization):
  | Operation | Throughput |
  |-----------|------------|
  | Export | **4479 MB/s** |
  | Import | **2943 MB/s** |

#### Tauri RAG Tutorial (WIS-74)
- **`examples/tauri-rag-app`**: Complete desktop RAG application
  - React + Tailwind UI
  - Document ingestion with chunking
  - Semantic search with VelesDB
  - Ready-to-run Tauri v2 template

### Changed

#### Performance Optimizations
- **Contiguous memory layout**: 58x faster import
  - Vector data stored in single buffer instead of individual allocations
  - Better cache locality for search operations
  - Bulk memory copy via unsafe slice operations

- **Pre-allocation**: Exact buffer sizing to avoid reallocations

### Testing

- **427 tests** total (+53 from v0.2.0)
  - 337 Rust core tests
  - 29 WASM tests
  - 61 TypeScript SDK tests

---

## [0.3.1] - 2025-12-23

### Added

#### Performance Optimizations P1 (WIS-86/87)

- **ContiguousVectors**: Cache-optimized memory layout
  - 64-byte aligned contiguous buffer for cache line efficiency
  - Zero-indirection vector access
  - 14 TDD tests

- **CPU Prefetch Hints**: L2 cache warming for HNSW traversal
  - Lookahead distance of 4 vectors
  - +12% throughput on random access patterns

- **Batch WAL Write**: Single disk write per bulk import
  - `store_batch()` method on `VectorStorage` trait
  - Contiguous mmap allocation for batch vectors

- **Batch Distance Computation**: SIMD-optimized batch operations
  - `batch_dot_products()` with prefetching
  - `batch_cosine_similarities()` for parallel queries

### Performance

| Benchmark | Result | Improvement |
|-----------|--------|-------------|
| Random Access | **2.3 Gelem/s** | +12% with prefetch |
| Insert (128D) | **100M elem/s** | Contiguous layout |
| Insert (768D) | **1.84M elem/s** | Batch WAL |
| Bulk Import | **15.4K vec/s** | 10x vs regular upsert |
| Memory Alloc | **6.75ms** | +8% vs Vec<Vec> |

### Search Quality

| Mode | Recall@10 | Status |
|------|-----------|--------|
| Balanced (ef=128) | **98.2%** | ✅ >= 95% |
| Accurate (ef=256) | **99.4%** | ✅ >= 95% |
| HighRecall (ef=512) | **99.6%** | ✅ >= 95% |

### Testing

- **417 tests** total (all passing)
- Code coverage maintained >= 80%

---

## [Unreleased]

### Planned
- LlamaIndex integration (WIS-66)
- Prometheus /metrics endpoint (WIS-63)
- Product Quantization (WIS-65)
- Multi-tenancy (WIS-68)
- API Authentication (WIS-69)
- Starlight documentation site

[0.7.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.7.0
[0.6.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.6.0
[0.5.2]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.5.2
[0.5.1]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.5.1
[0.5.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.5.0
[0.4.1]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.4.1
[0.4.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.4.0
[0.3.8]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.3.8
[0.3.2]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.3.2
[0.3.1]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.3.1
[0.3.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.3.0
[0.2.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.2.0
[0.1.4]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.1.4
[0.1.2]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.1.2
[0.1.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.1.0
[Unreleased]: https://github.com/cyberlife-coder/VelesDB/compare/v0.7.0...HEAD
