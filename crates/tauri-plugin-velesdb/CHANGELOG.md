# Changelog

All notable changes to `tauri-plugin-velesdb` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.4.5] - 2026-03-05

### Added

#### Collection Management
- `create_metadata_collection` — metadata-only collections (no vectors, BM25 text search only)
- `upsert_metadata` — insert/update points in metadata collections
- `is_empty` — check if a collection has no points
- `flush` — explicit WAL flush to disk

#### Knowledge Graph API
- `add_edge` — add a directed, labelled edge between two node IDs with optional properties
- `get_edges` — query edges by label, source, and/or target
- `traverse_graph` — BFS or DFS traversal from a source node with depth, relation-type, and limit controls
- `get_node_degree` — return in-degree and out-degree of a node

#### Search
- `batch_search` — parallel batch search (multiple independent queries in one call)
- `multi_query_search` — Multi-Query Generation (MQG) fusion: RRF, Average, Maximum, Weighted strategies
- Filter parameter added to `search`, `batch_search`, `hybrid_search`, `multi_query_search`

#### Agent Memory SDK
- `semantic_store` — store a knowledge fact with its embedding
- `semantic_query` — retrieve semantically similar facts by embedding

#### Event System
- `velesdb://collection-created` event on collection creation
- `velesdb://collection-deleted` event on collection deletion
- `velesdb://collection-updated` event on upsert/delete with item count
- `velesdb://operation-progress` event for long-running operations
- `velesdb://operation-complete` event with success/error and duration

#### Storage Modes
- `pq` (Product Quantization) storage mode added alongside `full`, `sq8`, `binary`

#### Init Variants
- `init()` — default `./velesdb_data` path
- `init_with_path(path)` — explicit data directory
- `init_with_app_data(app_name)` — platform-specific app-data directory
  (Windows: `%APPDATA%`, macOS: `~/Library/Application Support`, Linux: `~/.local/share`)

#### Public Rust API
- `VelesDbState` is `pub` — accessible from custom Tauri commands via `app.state::<VelesDbState>()`
- `VelesDbState::with_db(closure)` executes a closure with a read lock on `Arc<Database>`
- `VelesDbExt` trait — `app.velesdb()` returns a `SimpleVectorIndex` handle for in-memory demos

#### TypeScript SDK (`guest-js/index.ts`)
- Full type-safe wrappers for all commands (`createCollection`, `upsert`, `search`, `hybridSearch`, etc.)
- `StorageMode` and `DistanceMetric` union types
- `FusionStrategy` and `FusionParams` types for multi-query search
- `CommandError` interface with `message` and `code` fields

### Changed

- `VelesDbState::with_db` closure now receives `Arc<Database>` (not `&Arc<Database>`)
- `helpers::parse_fusion_strategy` accepts both snake_case and camelCase param keys
- Graph commands extracted to `commands_graph.rs` and re-exported from `commands.rs`

### Testing

- Test coverage across all modules: state, commands, graph commands, types, events, helpers, errors
- Integration tests in `tests/` directory

---

## [0.1.0] - 2025-12-22

### Added

#### Core Plugin
- Plugin initialization with `init(path)` and `init_default()`
- State management with thread-safe database access via `VelesDbState`
- Error handling with `Error` and `CommandError` types

#### Collection Management
- `create_collection` — create vector collections with configurable metrics and storage modes
- `delete_collection` — remove collections
- `list_collections` — list all collections with metadata
- `get_collection` — get detailed collection info

#### Vector Operations
- `upsert` — insert or update vectors with JSON payloads
- `get_points` — retrieve points by IDs
- `delete_points` — delete points by IDs
- `search` — vector similarity search with configurable top_k

#### Text Search
- `text_search` — full-text search using BM25 ranking

#### Hybrid Search
- `hybrid_search` — combined vector + text search with RRF fusion
- Configurable `vector_weight` parameter (0.0–1.0)

#### VelesQL
- `query` — execute VelesQL queries

#### Distance Metrics
- Cosine (default), Euclidean, Dot product, Hamming, Jaccard

#### Tauri v2 Integration
- Auto-generated permissions for all commands
- TypeScript type definitions
- Comprehensive documentation

---

[1.4.5]: https://github.com/cyberlife-coder/VelesDB/releases/tag/v1.4.5
[0.1.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/tauri-plugin-v0.1.0
