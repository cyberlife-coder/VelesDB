# Changelog

All notable changes to `tauri-plugin-velesdb` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2025-12-22

### Added

#### Core Plugin
- **Plugin initialization** with `init(path)` and `init_default()`
- **State management** with thread-safe database access via `VelesDbState`
- **Error handling** with `Error` and `CommandError` types

#### Collection Management
- `create_collection` - Create vector collections with configurable metrics
- `delete_collection` - Remove collections
- `list_collections` - List all collections with metadata
- `get_collection` - Get detailed collection info

#### Vector Operations
- `upsert` - Insert or update vectors with JSON payloads
- `search` - Vector similarity search with configurable top_k

#### Text Search (BM25)
- `text_search` - Full-text search using BM25 ranking

#### Hybrid Search
- `hybrid_search` - Combined vector + text search with RRF fusion
- Configurable `vector_weight` parameter (0.0-1.0)

#### VelesQL Support
- `query` - Execute VelesQL queries with MATCH support

#### Distance Metrics
- Cosine similarity (default)
- Euclidean distance
- Dot product
- Hamming distance
- Jaccard similarity

#### Tauri v2 Integration
- Auto-generated permissions for all commands
- TypeScript type definitions
- Comprehensive documentation

### Performance

| Operation | Latency |
|-----------|---------|
| Vector search (10k) | < 1ms |
| Text search (BM25) | < 5ms |
| Hybrid search | < 10ms |
| Batch insert (100) | < 10ms |

### Testing

- 26 unit tests covering all modules
- TDD approach with tests written before implementation
- Full clippy pedantic compliance

---

[0.1.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/tauri-plugin-v0.1.0
