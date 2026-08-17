# User Guides

Detailed, task-oriented guides for using VelesDB and velesdb-memory. See also
the [docs index](../README.md) for the full documentation map.

Guides are grouped by the surface they document. If you are looking for a
specific binding, start with its section — each crate README links here, and
each guide links back.

## Getting started

| Guide | Description |
|-------|-------------|
| [Installation](./INSTALLATION.md) | All installation methods (cargo, binaries, Docker) |
| [Installation options](./INSTALL_OPTIONS.md) | Decision aid: which install path to pick for core vs. agent memory |
| [Configuration](./CONFIGURATION.md) | `velesdb.toml` configuration reference |
| [Use Cases](./USE_CASES.md) | Common use cases and recommended configurations |
| [Business Scenarios](./BUSINESS_SCENARIOS.md) | End-to-end business problems solved with single queries |
| [API correspondence](./API_CORRESPONDENCE.md) | Rust, Python, TypeScript and MCP names for open, create, insert, search and recall |

## Core engine (Rust)

| Guide | Description |
|-------|-------------|
| [Core: public API map](./CORE_API_MAP.md) | Where to find what in `velesdb_core` — an import map, not a signature reference |
| [Core: collections and metrics](./CORE_COLLECTIONS_AND_METRICS.md) | The collection model: distance metrics, payloads, storage modes and their memory trade-offs |
| [Core: VelesQL reference](./CORE_VELESQL_REFERENCE.md) | The SQL-like language across the vector, graph and columnar engines |
| [Core: sparse vectors and fusion](./CORE_SPARSE_AND_FUSION.md) | Named sparse vectors (SPLADE, BM25, tag sets) beside dense embeddings, and result fusion |
| [Core: streaming inserts](./CORE_STREAMING_INSERTS.md) | Bounded-channel ingestion, backpressure and the delta buffer for continuously arriving data |
| [Core: query plan cache](./CORE_QUERY_PLAN_CACHE.md) | The two-tier LRU plan cache that lets repeated queries skip parsing and planning |
| [Core: Agent Memory SDK (Rust)](./AGENT_MEMORY.md#rust-api) | The `velesdb_core::agent` memory subsystems for chatbots, RAG and autonomous agents — the Rust API section of the Agent Memory guide |
| [Core: performance numbers](./CORE_PERFORMANCE.md) | The measured figures, with their methodology and hardware |
| [Search Modes](./SEARCH_MODES.md) | What Fast/Balanced/Accurate/Perfect/Adaptive mean and when to pick which |
| [Quantization](./QUANTIZATION.md) | Vector compression mechanisms (SQ8, PQ, Binary, RaBitQ): internals, training, persistence |
| [Tuning Guide](./TUNING_GUIDE.md) | The numeric reference: mode defaults, HNSW parameters, quantization trade-offs, memory estimation |
| [Graph Patterns](./GRAPH_PATTERNS.md) | Graph modeling and `MATCH` pattern recipes |
| [Multi-Model Queries](./MULTIMODEL_QUERIES.md) | Combining vector, graph, and structured data in one VelesQL query |
| [Concurrency & Locking](./CONCURRENCY_LOCKING.md) | Concurrent access and file-locking behavior |
| [Write Concurrency](./WRITE_CONCURRENCY.md) | Single-writer-per-collection model, batching patterns, Enterprise tier |

## Agent memory and MCP

| Guide | Description |
|-------|-------------|
| [Agent Memory](./AGENT_MEMORY.md) | AI agent memory: semantic, episodic, procedural, TTL, snapshots |
| [MCP server setup](./MCP_SERVER_SETUP.md) | velesdb-memory: install, every client config, the shared HTTPS daemon, embedding/extraction backends |
| [Context compiler](./CONTEXT_COMPILER.md) | Deterministic prompt compression: budgets, preservation rules, `risk`, transcripts, the `PostToolUse` hook |
| [Extraction models](./MEMORY_EXTRACTOR_MODELS.md) | Picking the local model that turns remembered facts into graph edges: criteria, VRAM tiers, and why schema discipline outranks size |
| [Temporal Memory](./TEMPORAL_MEMORY.md) | Dated recall and reasoning about *when* things happened |

## CLI and REPL

| Guide | Description |
|-------|-------------|
| [CLI command reference](./CLI_COMMAND_REFERENCE.md) | `velesdb`: every subcommand and flag, import/export formats, packaging, error reference |
| [REPL reference](./CLI_REPL_REFERENCE.md) | `velesdb repl`: dot-commands, session commands and settings, output formats |
| [VelesQL cookbook (CLI & REPL)](./CLI_VELESQL_COOKBOOK.md) | Runnable VelesQL snippets: vector, hybrid, sparse, temporal, graph, aggregation, JOIN |

## Server

| Guide | Description |
|-------|-------------|
| [Server REST tour](./SERVER_REST_TOUR.md) | `velesdb-server`: runnable `curl` recipes for collections, search modes, VelesQL, graph, `MATCH`, errors |
| [Server deployment](./SERVER_DEPLOYMENT.md) | `velesdb-server`: Docker, Kubernetes probes, rate limiting, CORS, startup update check |
| [Server Security](./SERVER_SECURITY.md) | API keys, TLS, CORS, and operations hardening |

## Python binding

| Guide | Description |
|-------|-------------|
| [Python API reference](./PYTHON_API_REFERENCE.md) | `Database`, `Collection`, search and storage — the authoritative signatures |
| [Python RAG pipeline](./PYTHON_RAG_PIPELINE.md) | From raw text to search results, end to end |
| [Python agent memory](./PYTHON_AGENT_MEMORY.md) | `MemoryService` and the Agent Memory SDK from Python |
| [Python context compiler](./PYTHON_CONTEXT_COMPILER.md) | Token-budgeted, provenance-audited prompt context |
| [Python graphs](./PYTHON_GRAPH.md) | Persistent `GraphCollection` and in-memory `GraphStore` |
| [Python VelesQL](./PYTHON_VELESQL.md) | The VelesQL parser API exposed to Python |
| [Python remote server](./PYTHON_REMOTE_SERVER.md) | Using the Python SDK alongside a remote `velesdb-server` |
| [Python Performance](./PYTHON_PERFORMANCE.md) | Throughput tuning for the Python binding |
| [Engine benchmarks](./PYTHON_ENGINE_BENCHMARKS.md) | The native Rust numbers behind the Python bindings |

## WASM (browser)

| Guide | Description |
|-------|-------------|
| [WASM JavaScript API](./WASM_API.md) | The full surface exposed by `@wiscale/velesdb-wasm` |
| [WASM persistence and format](./WASM_PERSISTENCE.md) | How a browser-side store survives a reload: IndexedDB, binary format, performance |
| [VelesQL in the browser](./WASM_VELESQL.md) | VelesQL parsed, validated and executed entirely client-side |

## Node.js addon

| Guide | Description |
|-------|-------------|
| [Node.js addon](./NODE_ADDON.md) | `@wiscale/velesdb-memory-node`: every method, the JS-side contracts, the context compiler, the bundled agent |
| [Building the Node addon](./NODE_ADDON_BUILD.md) | Building from source when no prebuilt binary covers your platform |

## Mobile (iOS / Android)

| Guide | Description |
|-------|-------------|
| [Mobile API](./MOBILE_API.md) | The complete `velesdb-mobile` UniFFI surface, in Swift and Kotlin |
| [Mobile build](./MOBILE_BUILD.md) | Cross-compiled libraries and bindings an Xcode or Gradle project can consume |

## Tauri plugin

| Guide | Description |
|-------|-------------|
| [Tauri plugin reference](./TAURI_PLUGIN_REFERENCE.md) | `tauri-plugin-velesdb`: every IPC command, events, permissions, storage modes, error codes |
| [Tauri plugin recipes](./TAURI_PLUGIN_RECIPES.md) | `tauri-plugin-velesdb`: runnable snippets for graph, sparse, indexes, VelesQL, events |

## Data migration (`velesdb-migrate`)

Importing from another database or a file dump. Not to be confused with the
version migration guides below.

| Guide | Description |
|-------|-------------|
| [Migrate: CLI and configuration](./MIGRATE_CLI.md) | Command surface and the full YAML schema |
| [Migrate: source reference](./MIGRATE_SOURCES.md) | Per-source configuration for every shipped connector |
| [Migrate: operations](./MIGRATE_OPERATIONS.md) | Throughput tuning, secret handling and troubleshooting |
| [Migrate: embeddings](./MIGRATE_EMBEDDINGS.md) | Rebuilding a `velesdb-memory` store against a new embedding model |

## Version migration guides

| Guide | Description |
|-------|-------------|
| [Migration v3.3.0](./MIGRATION_v3.3.0.md) | VelesQL correctness + cross-surface parity release migration guide |
| [Migration v4.0.0](./MIGRATION_v4.0.0.md) | Hardening + API-cleanup release migration guide |

## Tutorials

| Tutorial | Description |
|----------|-------------|
| [Tauri RAG App](../tutorials/tauri-rag-app/) | Build a desktop RAG application with Tauri |
| [Mini Recommender](./tutorials/MINI_RECOMMENDER.md) | Build a product recommendation engine with vector search and metadata filtering |
