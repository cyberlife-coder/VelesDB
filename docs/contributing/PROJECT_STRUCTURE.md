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
│   │   │   ├── index/         # HNSW, BM25, Trigram, Secondary indexes
│   │   │   ├── storage/       # mmap, WAL, sharded vectors, compaction
│   │   │   ├── velesql/       # VelesQL parser (pest), planner, executor, cache
│   │   │   ├── simd_native/   # AVX-512, AVX2, NEON distance kernels
│   │   │   ├── simd_dispatch.rs # Runtime SIMD path selection
│   │   │   ├── column_store/  # Typed column storage for metadata
│   │   │   ├── quantization/  # SQ8 (4x) and Binary (32x) compression
│   │   │   ├── fusion/        # RRF score fusion for hybrid search
│   │   │   ├── agent/         # Agent Memory Patterns SDK
│   │   │   ├── observer.rs    # DatabaseObserver trait (premium hooks)
│   │   │   └── guardrails/    # Allocation guards, memory limits
│   │   ├── benches/           # Criterion benchmarks
│   │   └── tests/             # Integration tests
│   │
│   ├── velesdb-server/        # Axum REST API server (37 endpoints)
│   │   ├── Cargo.toml
│   │   └── src/
│   │
│   ├── velesdb-cli/           # Interactive REPL for VelesQL
│   │   ├── Cargo.toml
│   │   └── src/
│   │
│   ├── velesdb-python/        # Python bindings (PyO3 + NumPy)
│   │   ├── Cargo.toml
│   │   └── src/
│   │
│   ├── velesdb-wasm/          # Browser-side vector search (no persistence)
│   │   ├── Cargo.toml
│   │   └── src/
│   │
│   ├── velesdb-mobile/        # iOS/Android bindings (UniFFI)
│   │   ├── Cargo.toml
│   │   └── src/
│   │
│   ├── velesdb-migrate/       # Schema and data migration tooling
│   │   ├── Cargo.toml
│   │   └── src/
│   │
│   └── tauri-plugin-velesdb/  # Tauri desktop integration
│       ├── Cargo.toml
│       └── src/
│
├── conformance/               # VelesQL cross-ecosystem conformance cases
│
├── integrations/
│   └── langchain-velesdb/     # LangChain VectorStore
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
- `repl`: Interactive VelesQL shell
- `query`: Single query execution
- `info`: Database information

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
