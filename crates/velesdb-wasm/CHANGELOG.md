# Changelog

All notable changes to `velesdb-wasm` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Performance Optimizations
- **`with_capacity()`** - Create store with pre-allocated memory for known vector counts
- **`reserve()`** - Pre-allocate memory before bulk insertions
- **`insert_batch()`** - Insert multiple vectors in a single call (5-10x faster than individual inserts)

#### Documentation
- High-performance bulk insert examples in README
- Updated API documentation with all new methods
- Interactive browser demo with batch insert

### Changed
- IDs now use `bigint` (u64) instead of `number` for JavaScript interoperability

## [0.2.0] - 2025-12-22

### Added

#### Core Features
- **VectorStore** - In-memory vector storage with SIMD-optimized distance calculations
- **Multiple metrics** - Cosine, Euclidean, Dot Product support
- **WASM SIMD128** - Hardware-accelerated vector operations

#### API
- `new(dimension, metric)` - Create vector store
- `insert(id, vector)` - Insert single vector
- `search(query, k)` - Find k nearest neighbors
- `remove(id)` - Remove vector by ID
- `clear()` - Clear all vectors
- `memory_usage()` - Get memory consumption estimate

#### Properties
- `len` - Number of vectors
- `is_empty` - Check if store is empty
- `dimension` - Vector dimension

### Performance
- Insert: ~1µs per vector (128D)
- Search: ~50µs for 10k vectors (128D)
- Memory: 4 bytes per dimension + 8 bytes per ID
