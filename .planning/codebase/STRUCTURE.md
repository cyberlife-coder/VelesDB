# Codebase Structure

**Analysis Date:** 2026-02-06

## Directory Layout

```
velesdb-core/
├── Cargo.toml                 # Workspace root configuration
├── crates/                    # Rust crate packages
│   ├── velesdb-core/         # Core library (engine)
│   ├── velesdb-server/       # HTTP REST API server
│   ├── velesdb-cli/          # Command-line interface
│   ├── velesdb-python/       # Python bindings (PyO3)
│   ├── velesdb-wasm/         # WebAssembly bindings
│   ├── velesdb-mobile/       # Mobile bindings (UniFFI)
│   ├── velesdb-migrate/      # Migration tool
│   └── tauri-plugin-velesdb/ # Tauri desktop plugin
├── sdks/                     # Language SDKs
│   └── typescript/           # TypeScript SDK
├── integrations/             # Framework integrations
│   ├── langchain/            # LangChain VectorStore
│   └── llamaindex/           # LlamaIndex integration
├── tests/                    # E2E integration tests
├── examples/                 # Usage examples
│   ├── rust/                 # Rust examples
│   ├── python/               # Python examples
│   ├── ecommerce_recommendation/
│   └── mini_recommender/
├── demos/                    # Demo applications
│   └── tauri-rag-app/        # Tauri RAG demo
├── docs/                     # Documentation
├── scripts/                  # Build and CI scripts
└── .epics/                   # Epic tracking directories
```

## Directory Purposes

**crates/velesdb-core/src/:**
- Purpose: Core vector database engine
- Contains: Indexing, storage, query engine, SIMD operations
- Key files: `lib.rs`, `collection/`, `index/`, `storage/`, `velesql/`

**crates/velesdb-server/src/:**
- Purpose: Axum-based HTTP API
- Contains: Route handlers, OpenAPI docs, middleware
- Key files: `main.rs`, `handlers/`, `types.rs`

**crates/velesdb-cli/src/:**
- Purpose: Interactive CLI and REPL
- Contains: Command parsing, import/export, graph commands
- Key files: `main.rs`, `repl.rs`, `import.rs`

**crates/velesdb-python/src/:**
- Purpose: Python bindings via PyO3
- Contains: Python module exports, collection wrapper, graph support
- Key files: `lib.rs`, `collection.rs`, `graph.rs`

**crates/velesdb-wasm/src/:**
- Purpose: Browser-compatible WASM bindings
- Contains: VectorStore, GraphStore, VelesQL bindings
- Key files: `lib.rs`, `store_*.rs`, `graph.rs`

**crates/velesdb-migrate/src/:**
- Purpose: Data migration from other vector DBs
- Contains: Connectors, transforms, interactive wizard
- Key files: `main.rs`, `connectors/`, `wizard/`

**sdks/typescript/:**
- Purpose: TypeScript client SDK
- Contains: HTTP client, type definitions

**integrations/:**
- Purpose: Third-party framework integrations
- Contains: LangChain VectorStore, LlamaIndex reader

**tests/:**
- Purpose: End-to-end integration tests
- Contains: `e2e_complete.rs`

**examples/:**
- Purpose: Usage demonstrations
- Contains: Rust, Python, WASM examples

## Key File Locations

### Entry Points
- `crates/velesdb-server/src/main.rs`: HTTP server startup
- `crates/velesdb-cli/src/main.rs`: CLI entry
- `crates/velesdb-migrate/src/main.rs`: Migration tool
- `crates/velesdb-core/src/lib.rs`: Library API

### Configuration
- `Cargo.toml`: Workspace dependencies and members
- `crates/velesdb-core/Cargo.toml`: Core features (persistence, gpu)
- `crates/velesdb-core/src/config.rs`: Runtime configuration

### Core Logic
- `crates/velesdb-core/src/lib.rs`: Database and Collection API
- `crates/velesdb-core/src/collection/types.rs`: Collection struct
- `crates/velesdb-core/src/index/hnsw/native_index.rs`: HNSW index
- `crates/velesdb-core/src/storage/mmap.rs`: Memory mapping
- `crates/velesdb-core/src/velesql/parser.rs`: Query parser

### Testing
- `crates/velesdb-core/src/*/tests.rs`: Co-located unit tests
- `crates/velesdb-core/tests/*.rs`: Integration tests
- `tests/e2e_complete.rs`: E2E tests

### Benchmarks
- `crates/velesdb-core/benches/*.rs`: Criterion benchmarks
- `crates/velesdb-wasm/benches/*.rs`: WASM-specific benches

## Naming Conventions

### Files
- **Module entry:** `mod.rs` for directory modules
- **Tests:** `*_tests.rs` alongside source files or in `tests/` subdirectory
- **Benchmarks:** `*_benchmark.rs` in `benches/`
- **Examples:** Descriptive names (e.g., `crash_driver.rs`)

### Directories
- **Modules:** Lowercase with underscores (e.g., `collection/`, `velesql/`)
- **Crates:** `velesdb-*` prefix for all packages
- **Tests:** `tests/` directory for integration tests

### Types
- **Structs:** PascalCase (e.g., `HnswIndex`, `Collection`)
- **Traits:** PascalCase with descriptive names (e.g., `VectorIndex`)
- **Enums:** PascalCase, variants PascalCase (e.g., `DistanceMetric::Cosine`)
- **Functions:** snake_case (e.g., `create_collection`)
- **Constants:** SCREAMING_SNAKE_CASE (e.g., `DEFAULT_EF_CONSTRUCTION`)

## Where to Add New Code

### New Feature in Core
- **Implementation:** `crates/velesdb-core/src/[module]/`
- **Tests:** `crates/velesdb-core/src/[module]/[feature]_tests.rs`
- **Benchmarks:** `crates/velesdb-core/benches/[feature]_benchmark.rs`

### New API Endpoint
- **Handler:** `crates/velesdb-server/src/handlers/[resource].rs`
- **Types:** `crates/velesdb-server/src/types.rs`
- **Registration:** `crates/velesdb-server/src/main.rs` router

### New CLI Command
- **Implementation:** `crates/velesdb-cli/src/[command].rs`
- **CLI definition:** `crates/velesdb-cli/src/main.rs` `Commands` enum
- **Dispatch:** `crates/velesdb-cli/src/main.rs` match statement

### New Python Binding
- **Implementation:** `crates/velesdb-python/src/[module].rs`
- **Registration:** `crates/velesdb-python/src/lib.rs` `#[pymodule]`

### New WASM Export
- **Implementation:** `crates/velesdb-wasm/src/[module].rs`
- **Registration:** `crates/velesdb-wasm/src/lib.rs` `#[wasm_bindgen]`

### New Migration Connector
- **Implementation:** `crates/velesdb-migrate/src/connectors/[source].rs`
- **Tests:** `crates/velesdb-migrate/src/connectors/[source]_tests.rs`

## Special Directories

**.epics/:**
- Purpose: Epic completion tracking
- Structure: One directory per completed epic (e.g., `EPIC-001-code-quality-refactoring-done/`)
- Generated: No (manual creation)
- Committed: Yes

**.cargo/:**
- Purpose: Cargo configuration
- Contains: Build profiles, registry config

**target/:**
- Purpose: Build artifacts
- Generated: Yes
- Committed: No (in .gitignore)

**fuzz/:**
- Purpose: Fuzzing targets
- Contains: LibFuzzer harnesses

---

*Structure analysis: 2026-02-06*
