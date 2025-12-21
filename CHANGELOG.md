# Changelog

All notable changes to VelesDB will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

## [Unreleased]

### Planned
- LlamaIndex integration
- Prometheus /metrics endpoint (WIS-49)
- EXPLAIN query plans

[0.2.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.2.0
[0.1.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.1.0
[Unreleased]: https://github.com/cyberlife-coder/VelesDB/compare/v0.2.0...HEAD
