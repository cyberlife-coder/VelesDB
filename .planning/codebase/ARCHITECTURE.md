# Architecture

**Analysis Date:** 2026-02-06

## Pattern Overview

**Overall:** Layered Architecture with Hexagonal/Ports-and-Adapters pattern for multi-platform bindings

**Key Characteristics:**
- **Workspace-based monorepo:** Multiple crates in a single repository with shared dependencies
- **Core-periphery architecture:** `velesdb-core` contains business logic; other crates provide platform bindings
- **Trait-based abstractions:** `VectorIndex` trait allows different index implementations
- **Feature-gated compilation:** Optional persistence, GPU, and WASM features
- **SIMD-first design:** Explicit SIMD dispatch with runtime detection

## Layers

### Core Engine Layer
**Purpose:** Vector database engine with indexing, storage, and query execution
**Location:** `crates/velesdb-core/src/`
**Contains:** 
- Vector index implementations (HNSW native)
- Storage engines (mmap, vector bytes)
- Query language parser and executor (VelesQL)
- Distance metric calculations with SIMD
- Graph storage and traversal
**Depends on:** None (self-contained)
**Used by:** Server, CLI, Python bindings, WASM bindings, Mobile SDK

### Storage Layer
**Purpose:** Persistent and in-memory storage abstractions
**Location:** `crates/velesdb-core/src/storage/`
**Contains:**
- `mmap.rs` - Memory-mapped file storage
- `vector_bytes.rs` - Vector serialization
- `log_payload.rs` - WAL for durability
- `guard.rs` - Epoch-based memory reclamation
**Depends on:** Core types
**Used by:** Collection management, index implementations

### Index Layer
**Purpose:** Approximate nearest neighbor search and full-text indexing
**Location:** `crates/velesdb-core/src/index/`
**Contains:**
- `hnsw/` - Native HNSW implementation (native_inner.rs, native_index.rs)
- `bm25.rs` - Full-text BM25 scoring
- `trigram/` - Trigram-based text indexing
- Posting lists and mappings
**Depends on:** Storage, SIMD dispatch
**Used by:** Collection search operations

### Collection Layer
**Purpose:** High-level data container API
**Location:** `crates/velesdb-core/src/collection/`
**Contains:**
- `core/` - CRUD operations (crud.rs, lifecycle.rs)
- `graph/` - Knowledge graph (nodes, edges, property indexes)
- `search/` - Search execution
- `types.rs` - Collection and config types
**Depends on:** Index, Storage, VelesQL
**Used by:** Database API, bindings

### Query Engine Layer
**Purpose:** SQL-like query parsing and execution
**Location:** `crates/velesdb-core/src/velesql/`
**Contains:**
- `parser/` - PEST-based parser
- `ast.rs` - Abstract syntax tree
- `planner.rs` - Query planning
- `aggregator.rs` - Aggregation functions
- `hybrid.rs` - Vector + text fusion
**Depends on:** Collection layer
**Used by:** Server handlers, CLI REPL

### SIMD Layer
**Purpose:** Hardware-accelerated distance calculations
**Location:** `crates/velesdb-core/src/simd_native.rs`, `simd_neon.rs`
**Contains:**
- Runtime SIMD dispatch
- AVX-512, AVX2, SSE, ARM NEON implementations
- Batch operations with prefetching
**Depends on:** None
**Used by:** HNSW index, distance calculations

### API Adapter Layer
**Purpose:** HTTP REST API with async handlers
**Location:** `crates/velesdb-server/src/`
**Contains:**
- `main.rs` - Axum server bootstrap
- `handlers/` - Route handlers (collections, points, search, graph)
- `types.rs` - Request/response DTOs
**Depends on:** velesdb-core, tokio, axum
**Used by:** External HTTP clients

### CLI Layer
**Purpose:** Interactive command-line interface
**Location:** `crates/velesdb-cli/src/`
**Contains:**
- `main.rs` - CLI argument parsing
- `repl.rs` - Interactive REPL
- `import.rs` - CSV/JSONL import
- `graph.rs` - Graph commands
**Depends on:** velesdb-core, clap
**Used by:** End users

### Language Binding Layer
**Purpose:** FFI bindings for multiple languages
**Python:** `crates/velesdb-python/src/` - PyO3 bindings
**WASM:** `crates/velesdb-wasm/src/` - wasm-bindgen bindings  
**Mobile:** `crates/velesdb-mobile/src/` - UniFFI bindings
**Depends on:** velesdb-core
**Used by:** Python/JS/Swift/Kotlin applications

### Migration Layer
**Purpose:** Data migration from other vector databases
**Location:** `crates/velesdb-migrate/src/`
**Contains:**
- `connectors/` - Qdrant, Weaviate, Chroma, Pinecone, pgvector, Milvus, Redis, MongoDB, Elasticsearch
- `transform.rs` - Data transformation
- `wizard/` - Interactive migration UI
**Depends on:** velesdb-core, external SDKs
**Used by:** Migration workflows

## Data Flow

### Vector Search Flow:

1. **Request Entry:** HTTP POST `/collections/{name}/search` → `handlers/search.rs`
2. **Validation:** Request body validation → `types.rs`
3. **Collection Lookup:** `db.get_collection()` → `lib.rs`
4. **Search Execution:** `collection.search()` → `collection/core/crud.rs`
5. **Index Search:** `hnsw_index.search()` → `index/hnsw/native_index.rs`
6. **SIMD Dispatch:** Distance calculation → `simd_dispatch.rs` → `simd_native.rs`
7. **Result Aggregation:** Top-k selection → Return to handler
8. **Response Serialization:** JSON → HTTP response

### VelesQL Query Flow:

1. **Request Entry:** HTTP POST `/query` → `handlers/query.rs`
2. **Parse:** `Parser::parse()` → `velesql/parser/`
3. **Validate:** `QueryValidator::validate()` → `velesql/validation.rs`
4. **Plan:** `QueryPlanner::plan()` → `velesql/planner.rs`
5. **Execute:** 
   - Vector search → `collection/search/`
   - Graph traversal → `collection/graph/`
   - Aggregations → `velesql/aggregator.rs`
6. **Return:** Query results → JSON response

### Graph Traversal Flow:

1. **Request:** HTTP POST `/collections/{name}/graph/traverse`
2. **GraphService:** In-memory graph operations
3. **Traversal:** BFS/DFS with property indexes → `collection/graph/traversal.rs`
4. **Result Streaming:** Streaming response for large traversals

### State Management

**Database State:**
- `Database` struct holds `RwLock<HashMap<String, Collection>>`
- Collections are loaded lazily on access
- Persistence via memory-mapped files

**Collection State:**
- `Collection` holds `Arc` to shared index and storage
- HNSW index protected by `RwLock` for vectors and layers
- Lock ordering: vectors → layers → neighbors (prevents deadlocks)

**Graph State:**
- `GraphService` is in-memory only (preview feature)
- Nodes and edges stored in `DashMap` for concurrent access
- Property indexes for fast lookups

## Key Abstractions

**VectorIndex:**
- Purpose: Abstract interface for ANN indices
- Examples: `crates/velesdb-core/src/index/mod.rs`
- Pattern: Trait with `Send + Sync` bounds for thread safety

**Storage:**
- Purpose: Abstract vector and payload persistence
- Examples: `crates/velesdb-core/src/storage/traits.rs`
- Pattern: Trait-based with mmap implementation

**DistanceMetric:**
- Purpose: Pluggable distance calculations
- Examples: `crates/velesdb-core/src/distance.rs`
- Pattern: Enum with SIMD-optimized implementations

**Query Planner:**
- Purpose: Optimize query execution
- Examples: `crates/velesdb-core/src/velesql/planner.rs`
- Pattern: Cost-based planning with strategy selection

**GraphElement:**
- Purpose: Unified node/edge abstraction
- Examples: `crates/velesdb-core/src/collection/graph/mod.rs`
- Pattern: Enum with property storage

## Entry Points

**Server:**
- Location: `crates/velesdb-server/src/main.rs`
- Triggers: `cargo run -p velesdb-server`
- Responsibilities: HTTP server, routing, middleware

**CLI:**
- Location: `crates/velesdb-cli/src/main.rs`
- Triggers: `velesdb [command]`
- Responsibilities: Command parsing, REPL, import/export

**Python:**
- Location: `crates/velesdb-python/src/lib.rs`
- Triggers: `import velesdb`
- Responsibilities: PyO3 module initialization

**WASM:**
- Location: `crates/velesdb-wasm/src/lib.rs`
- Triggers: `init()` in browser
- Responsibilities: wasm-bindgen exports

**Migration:**
- Location: `crates/velesdb-migrate/src/main.rs`
- Triggers: `velesdb-migrate`
- Responsibilities: Interactive migration wizard

## Error Handling

**Strategy:** Result-based with custom error types

**Patterns:**
- `thiserror` for error definitions
- `anyhow` for application-level error handling
- `Result<T, Error>` returns throughout core
- HTTP 400/500 mapping in server handlers

## Cross-Cutting Concerns

**Logging:** `tracing` crate with structured logging
- All crates use `tracing::info!`, `tracing::debug!`
- No `println!` in production code

**Validation:** 
- Query validation in `velesql/validation.rs`
- Input bounds checking in handlers
- Guardrails for quotas and limits

**Authentication:**
- License validation for premium features
- Ed25519 signature verification

**Metrics:**
- Prometheus metrics endpoint (optional feature)
- Latency histograms and counters

---

*Architecture analysis: 2026-02-06*
