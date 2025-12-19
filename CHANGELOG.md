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

## [Unreleased]

### Planned
- Python bindings (PyO3)
- VelesQL CLI/REPL
- EXPLAIN query plans
- Hybrid search (BM25 + vector)

[0.1.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v0.1.0
[Unreleased]: https://github.com/cyberlife-coder/VelesDB/compare/v0.1.0...HEAD
