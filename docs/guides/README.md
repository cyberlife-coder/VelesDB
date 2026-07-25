# User Guides

Detailed, task-oriented guides for using VelesDB and velesdb-memory. See also
the [docs index](../README.md) for the full documentation map.

| Guide | Description |
|-------|-------------|
| [Installation](./INSTALLATION.md) | All installation methods (cargo, binaries, Docker) |
| [Installation options](./INSTALL_OPTIONS.md) | Decision aid: which install path to pick for core vs. agent memory |
| [Configuration](./CONFIGURATION.md) | `velesdb.toml` configuration reference |
| [Search Modes](./SEARCH_MODES.md) | Understanding Fast/Balanced/Accurate/Perfect modes |
| [CLI & REPL](./CLI_REPL.md) | Command-line interface and interactive shell |
| [Quantization](./QUANTIZATION.md) | Vector compression (SQ8, PQ, Binary, RaBitQ) |
| [Tuning Guide](./TUNING_GUIDE.md) | HNSW parameter tuning and performance optimization |
| [Agent Memory](./AGENT_MEMORY.md) | AI agent memory: semantic, episodic, procedural, TTL, snapshots |
| [MCP server setup](./MCP_SERVER_SETUP.md) | velesdb-memory: install, every client config, the shared HTTPS daemon, embedding/extraction backends |
| [Context compiler](./CONTEXT_COMPILER.md) | Deterministic prompt compression: budgets, preservation rules, `risk`, transcripts, the `PostToolUse` hook |
| [Temporal Memory](./TEMPORAL_MEMORY.md) | Dated recall and reasoning about *when* things happened, on top of velesdb-memory |
| [Graph Patterns](./GRAPH_PATTERNS.md) | Graph modeling and `MATCH` pattern recipes |
| [Multi-Model Queries](./MULTIMODEL_QUERIES.md) | Combining vector, graph, and structured data in one VelesQL query |
| [Server Security](./SERVER_SECURITY.md) | API keys, TLS, CORS, and operations hardening |
| [Business Scenarios](./BUSINESS_SCENARIOS.md) | End-to-end business problems solved with single queries |
| [Python Performance](./PYTHON_PERFORMANCE.md) | Throughput tuning for the Python binding |
| [Concurrency & Locking](./CONCURRENCY_LOCKING.md) | Concurrent access and file-locking behavior |
| [Write Concurrency](./WRITE_CONCURRENCY.md) | Single-writer-per-collection model, batching patterns, Enterprise tier |
| [Use Cases](./USE_CASES.md) | Common use cases and recommended configurations |
| [Migration v1.6](./MIGRATION_v1.6.md) | Version migration guide |
| [Migration v1.7](./MIGRATION_v1.7.md) | Version migration guide |
| [Migration v3.3.0](./MIGRATION_v3.3.0.md) | VelesQL correctness + cross-surface parity release migration guide |

## Tutorials

| Tutorial | Description |
|----------|-------------|
| [Tauri RAG App](../tutorials/tauri-rag-app/) | Build a desktop RAG application with Tauri |
| [Mini Recommender](./tutorials/MINI_RECOMMENDER.md) | Build a product recommendation engine with vector search and metadata filtering |
