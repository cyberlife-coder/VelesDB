# VelesDB-Core - Project Structure

## Overview

VelesDB-Core is a **Cargo workspace** containing eight crates. It is the open-source engine for the VelesDB vector database combining Vector + Graph + ColumnStore in a single engine.

```
velesdb-core/
│
├── Cargo.toml                 # Workspace root
├── Cargo.lock                 # Dependency lockfile
│
├── rust-toolchain.toml        # Rust version (stable)
├── rustfmt.toml               # Formatting config
├── clippy.toml                # Linter config
├── deny.toml                  # Dependency security audit
├── Makefile.toml              # cargo-make tasks
│
├── .cargo/
│   └── config.toml            # Cargo aliases
│
├── .githooks/
│   └── pre-commit             # Pre-commit hook
│
├── crates/
│   ├── velesdb-core/          # Core engine (vector, graph, storage, VelesQL)
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── collection/    # Typed collections (Vector, Graph, Metadata) + legacy
│   │   │   │   ├── core/      # CRUD, flush, recovery, lifecycle, index management
│   │   │   │   ├── graph/     # ConcurrentEdgeStore, CsrSnapshot, traversal, streaming
│   │   │   │   ├── search/    # Query planner, filter pushdown, reranking
│   │   │   │   ├── streaming/ # Delta buffer, streaming insert
│   │   │   │   └── vector_collection/ # VectorCollection impl
│   │   │   ├── database/      # Database, typed registries, query engine, DDL/DML executors
│   │   │   ├── index/         # HNSW (native), BM25, Trigram, Secondary, Sparse indexes
│   │   │   │   └── hnsw/native/ # NativeHnsw, BatchEfSchedule, graph_io, search pipeline
│   │   │   ├── storage/       # mmap, WAL, sharded vectors, compaction, snapshots
│   │   │   ├── velesql/       # VelesQL parser (pest), planner, executor, cache, AST
│   │   │   ├── simd_native/   # AVX-512, AVX2, NEON distance kernels
│   │   │   ├── sparse_index/  # Inverted index, DAAT MaxScore search
│   │   │   ├── column_store/  # Typed column storage, bitmap filters, vacuum
│   │   │   ├── quantization/  # SQ8, Binary, Product Quantization, RaBitQ
│   │   │   ├── fusion/        # RRF/RSF/Weighted score fusion
│   │   │   ├── agent/         # Agent Memory SDK (semantic, episodic, procedural)
│   │   │   ├── cache/         # LRU, plan cache, bloom filter, lock-free cache
│   │   │   ├── filter/        # Filter builders, matching, conversion
│   │   │   ├── gpu/           # wgpu backend, PQ GPU, shaders
│   │   │   ├── guardrails/    # Allocation guards, memory limits, resilience
│   │   │   ├── metrics/       # Latency, query, retrieval, operational metrics
│   │   │   ├── api_types/     # Shared request/response types
│   │   │   ├── compression/   # Dictionary compression
│   │   │   └── update_check/  # Version check client
│   │   ├── benches/           # Criterion benchmarks
│   │   └── tests/             # Integration + BDD tests
│   │
│   ├── velesdb-server/        # Axum REST API server (37 endpoints)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── handlers/      # Route handlers (query/, search/, graph, admin)
│   │
│   ├── velesdb-cli/           # Interactive REPL for VelesQL
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs        # CLI entry point, sub-enum dispatch
│   │       ├── commands.rs    # CollectionCommands, DataCommands, QueryCommands
│   │       ├── repl*.rs       # REPL modules (collection, data, graph, search, config)
│   │       └── graph*.rs      # Graph CLI handlers
│   │
│   ├── velesdb-python/        # Python bindings (PyO3 + NumPy)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs         # PyO3 module registration
│   │       ├── database.rs    # VelesDatabase pyclass
│   │       ├── fusion.rs      # FusionStrategy pyclass
│   │       ├── graph_collection.rs # PyGraphCollection
│   │       └── collection/    # PyCollection methods
│   │
│   ├── velesdb-wasm/          # Browser-side vector search (no persistence)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs         # WASM entry point
│   │       ├── vector_store.rs # VectorStore struct + search/insert
│   │       ├── graph.rs       # GraphStore for in-browser knowledge graphs
│   │       └── velesql.rs     # Client-side VelesQL parsing
│   │
│   ├── velesdb-mobile/        # iOS/Android bindings (UniFFI)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs         # VelesDatabase + UniFFI exports
│   │       ├── collection.rs  # VelesCollection (full search API)
│   │       └── graph.rs       # MobileGraphStore (BFS/DFS/parallel)
│   │
│   ├── velesdb-migrate/       # Data migration from 12 sources
│   │   ├── Cargo.toml
│   │   └── src/
│   │
│   └── tauri-plugin-velesdb/  # Tauri v2 desktop integration
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs         # Plugin init + invoke handler macro
│           ├── commands.rs    # Core commands (CRUD, search)
│           ├── commands_graph.rs  # Graph commands (traverse, parallel BFS)
│           ├── commands_index.rs  # Index management
│           ├── commands_sparse.rs # Sparse vector commands
│           └── commands_memory.rs # Agent memory commands
│
├── sdks/
│   └── typescript/            # TypeScript SDK (REST + WASM backends)
│       ├── package.json
│       └── src/
│
├── integrations/
│   ├── common/                # Shared integration utilities
│   ├── langchain/             # LangChain VectorStore
│   └── llamaindex/            # LlamaIndex VectorStore
│
├── conformance/               # VelesQL cross-ecosystem conformance cases
│
├── docs/                      # Documentation
│
├── scripts/                   # CI, release, and validation scripts
│
└── examples/                  # Example applications
```

---

## Workspace Crates

### `velesdb-core`

Core engine. Contains:
- **HNSW Index**: Native implementation (1.2x faster than hnsw_rs (benchmarked: 26.9ms vs ~32ms on 100 queries, 5K vectors)) with AVX-512, AVX2, and NEON SIMD acceleration via runtime feature detection
- **Typed Collections**: `VectorCollection`, `GraphCollection`, `MetadataCollection` (plus legacy `Collection` for backward compatibility)
- **VelesQL**: SQL-like query language with vector and graph extensions (pest-based parser)
- **Storage**: Memory-mapped files, WAL, sharded vectors, compaction
- **Quantization**: SQ8 (4x), Binary (32x), Product Quantization (8-32x), RaBitQ (32x)
- **Agent Memory**: Semantic, episodic, and procedural memory patterns for AI agents
- **Graph Engine**: CsrSnapshot zero-copy BFS/DFS, parallel multi-source BFS, FxHashSet visited sets, parent-pointer path reconstruction

### `velesdb-server`

Axum-based REST API server with 37 endpoints. Exposes:
- CRUD endpoints for collections and points
- `/search`, `/search/batch`, `/search/hybrid` endpoints
- `/query` endpoint for VelesQL execution
- Optional OpenAPI documentation

### `velesdb-cli`

Command-line interface with:
- `repl`: Interactive VelesQL shell with dot-commands and backslash-commands
- `collection`: Create/list/show/delete/analyze collections (vector, graph, metadata)
- `data`: Import/export, upsert, get, delete points
- `query`: Single VelesQL query execution + multi-search fusion + explain
- `graph`: Add/remove edges, traverse (BFS/DFS), neighbors, degree, search, node payloads
- `index`: Create/list/drop secondary, property, and range indexes
- `simd`: SIMD diagnostics and benchmarks
- `license`: License management
- Commands grouped into sub-enums (`CollectionCommands`, `DataCommands`, `QueryCommands`)

### `velesdb-python`

Python bindings via PyO3:
- `Database`, `VelesDatabase`, `Collection`, `GraphCollection`, `AgentMemory` classes
- `FusionStrategy` pyclass (extracted to `fusion.rs`)
- NumPy array support (float32, float64)
- Parallel BFS with GIL release (`py.allow_threads`)
- Comprehensive pytest suite

### `velesdb-wasm`

Browser-side vector search. Must be built without the `persistence` feature:
```bash
cargo build -p velesdb-wasm --no-default-features --target wasm32-unknown-unknown
```

### `velesdb-mobile`

iOS and Android bindings via UniFFI:
- Swift bindings for iOS
- Kotlin bindings for Android
- `VelesCollection` (extracted to `collection.rs`) with full search API
- `MobileGraphStore` with BFS, DFS, and parallel multi-source BFS
- StorageMode support (Full, SQ8, Binary) for IoT/Edge

### `velesdb-migrate`

Schema and data migration tooling. Supports 12 source connectors: Qdrant, Pinecone, Weaviate, Milvus, ChromaDB, pgvector, Supabase, Elasticsearch, MongoDB Atlas, Redis, JSON, CSV.

### `tauri-plugin-velesdb`

Tauri desktop integration plugin for building local-first desktop applications with embedded vector search. Includes index management (create/drop/list), graph traversal (BFS/DFS/parallel BFS), sparse vectors, agent memory, and streaming insert.

---

## Feature Flags

| Flag | Purpose | Default |
|------|---------|---------|
| `persistence` | mmap, WAL, rayon, tokio | Yes |
| `gpu` | wgpu-based GPU acceleration | No |
| `update-check` | HTTP version checking | No |
| `loom` | Concurrency testing (nightly) | No |

The `persistence` feature must be disabled for WASM targets.

---

## Configuration Files

### `rust-toolchain.toml`

Pins the Rust toolchain version for all developers:

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

### `.cargo/config.toml`

Defines cargo aliases for common commands. Note: the `target-cpu=native` line must stay commented out to preserve CI compatibility.

### `.githooks/pre-commit`

Runs automatically before each `git commit`:
- Checks formatting
- Runs clippy
- Runs tests
- Detects secrets

Activate with: `git config core.hooksPath .githooks`

---

## Relationship with VelesDB-Premium

```
┌─────────────────────┐
│   velesdb-premium   │  (private repo)
│   Premium features  │
└─────────┬───────────┘
          │ depends via git
          ▼
┌─────────────────────┐
│    velesdb-core     │  (this repo)
│   Open-source core  │
└─────────────────────┘
```

Premium imports Core as a workspace dependency:

```toml
[workspace.dependencies]
velesdb-core = { git = "https://github.com/cyberlife-coder/velesdb.git", branch = "main" }
```
