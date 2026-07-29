# 📚 VelesDB Documentation

> **VelesDB — the explainable, local-first memory engine for AI agents.** One ~9 MB binary fuses vector + graph + columnar under VelesQL; [`why()`](./guides/AGENT_MEMORY.md) returns the evidence path behind every recall. Zero cloud.

Welcome to the VelesDB documentation. This guide will help you get started and make the most of VelesDB.

---

## Quick Links

- **[Getting Started](./getting-started.md)** - Quick installation and first steps
- **[Why VelesDB?](./WHY_VELESDB.md)** - Our unique value proposition
- **[Benchmarks](./BENCHMARKS.md)** - Performance comparison with other vector databases

---

## 📖 User Guides

Detailed guides for using VelesDB features. **[Full guides index →](./guides/README.md)**
(all guides, including migration notes and tutorials).

| Guide | Description |
|-------|-------------|
| [Installation](./guides/INSTALLATION.md) | All installation methods (cargo, binaries, Docker) |
| [Installation options](./guides/INSTALL_OPTIONS.md) | Decision aid: which install path to pick for core vs. agent memory |
| [Configuration](./guides/CONFIGURATION.md) | `velesdb.toml` configuration reference |
| [Search Modes](./guides/SEARCH_MODES.md) | Understanding Fast/Balanced/Accurate/Perfect modes |
| [CLI & REPL](./guides/CLI_REPL.md) | Command-line interface and interactive shell |
| [Quantization](./guides/QUANTIZATION.md) | Vector compression (SQ8, PQ, Binary, RaBitQ) |
| [Tuning Guide](./guides/TUNING_GUIDE.md) | HNSW parameter tuning and performance optimization |
| [Agent Memory](./guides/AGENT_MEMORY.md) | AI agent memory: semantic, episodic, procedural, TTL, snapshots |
| [MCP server setup](./guides/MCP_SERVER_SETUP.md) | velesdb-memory: install, client config, shared HTTPS daemon, embedding/extraction backends |
| [Context compiler](./guides/CONTEXT_COMPILER.md) | Deterministic prompt compression: budgets, preservation rules, `risk`, the `PostToolUse` hook |
| [Graph Patterns](./guides/GRAPH_PATTERNS.md) | Graph modeling and `MATCH` pattern recipes |
| [Multi-Model Queries](./guides/MULTIMODEL_QUERIES.md) | Combining vector, graph, and structured data in one VelesQL query |
| [Server Security](./guides/SERVER_SECURITY.md) | API keys, TLS, CORS, and operations hardening |
| [Business Scenarios](./guides/BUSINESS_SCENARIOS.md) | End-to-end business problems solved with single queries |
| [Python Performance](./guides/PYTHON_PERFORMANCE.md) | Throughput tuning for the Python binding |
| [Concurrency & Locking](./guides/CONCURRENCY_LOCKING.md) | Concurrent access and file-locking behavior |
| [Write Concurrency](./guides/WRITE_CONCURRENCY.md) | Single-writer-per-collection model, batching patterns, Enterprise tier |
| [Use Cases](./guides/USE_CASES.md) | Common use cases and recommended configurations |
| [Migration v1.6](./guides/MIGRATION_v1.6.md) / [v1.7](./guides/MIGRATION_v1.7.md) | Version migration guides |
| [Migration v3.3.0](./guides/MIGRATION_v3.3.0.md) | Error codes, REST statuses and query results that changed in 3.3.0. |
| [Migration v4.0.0](./guides/MIGRATION_v4.0.0.md) | The 4.0.0 breaking changes — start with the WASM `weighted` reordering, which is silent. |
| [Troubleshooting](./NEW_USER_TROUBLESHOOTING.md) | Solutions for common issues new users encounter |

### API reference, per binding

Each binding has its own reference guide; the
[full guides index](./guides/README.md) groups every guide by surface.

| Binding | Reference |
|---------|-----------|
| Rust (`velesdb-core`) | [Public API map](./guides/CORE_API_MAP.md) · [VelesQL](./guides/CORE_VELESQL_REFERENCE.md) |
| Python | [API reference](./guides/PYTHON_API_REFERENCE.md) |
| WASM (browser) | [JavaScript API](./guides/WASM_API.md) |
| Node.js | [Addon reference](./guides/NODE_ADDON.md) |
| Mobile (Swift / Kotlin) | [Mobile API](./guides/MOBILE_API.md) |
| Server (REST) | [REST tour](./guides/SERVER_REST_TOUR.md) |
| Tauri plugin | [Plugin reference](./guides/TAURI_PLUGIN_REFERENCE.md) |
| Data migration | [`velesdb-migrate` CLI](./guides/MIGRATE_CLI.md) |

---

## 📐 Technical Reference

In-depth technical documentation. **[Full reference index →](./reference/README.md)**
(all reference docs, plus the machine-readable promise contract).

| Reference | Description |
|-----------|-------------|
| [Architecture](./reference/ARCHITECTURE.md) | System design and internals |
| [VelesQL Specification](./VELESQL_SPEC.md) | Query language grammar and syntax (v3.12.0, canonical) |
| [VelesQL Cheat Sheet](./reference/VELESQL_CHEATSHEET.md) | One-page quick reference: search, filter, graph MATCH, fusion, sparse, EXPLAIN |
| [VelesQL Contract](./reference/VELESQL_CONTRACT.md) | Canonical REST contract (`/query`, `/match`, error model) |
| [VelesQL Conformance](./reference/VELESQL_CONFORMANCE_MATRIX.md) | Cross-ecosystem conformance matrix |
| [MCP Tool Reference](./reference/MCP_TOOLS.md) | velesdb-memory: every MCP tool, one section each — parameters, returns, limits, error model |
| [Performance SLO](./reference/PERFORMANCE_SLO.md) | CI-enforced performance objectives and budget gates |
| [REST API](./reference/api-reference.md) | HTTP API endpoints |
| [SIMD Performance](./reference/SIMD_PERFORMANCE.md) | SIMD optimizations and benchmarks |

---

## 🎓 Tutorials

Step-by-step tutorials:

| Tutorial | Description |
|----------|-------------|
| [Tauri RAG App](./tutorials/tauri-rag-app/) | Build a desktop RAG application with Tauri |

---

## 🤝 Contributing

For contributors and developers:

| Guide | Description |
|-------|-------------|
| [Coding Rules](./contributing/CODING_RULES.md) | Code style and conventions |
| [TDD Rules](./contributing/TDD_RULES.md) | Test-driven development practices |
| [Benchmarking Guide](./contributing/BENCHMARKING_GUIDE.md) | How to run and interpret benchmarks |
| [Code Signing](./contributing/CODE_SIGNING.md) | Release signing process |
| [Project Structure](./contributing/PROJECT_STRUCTURE.md) | Codebase organization |

---

## 🌐 Ecosystem

VelesDB provides a complete ecosystem of SDKs and integrations:

| Component | Type | Description |
|-----------|------|-------------|
| [Ecosystem Sync Report](./reference/ECOSYSTEM_PARITY.md) | **Overview** | Feature parity matrix across all components |
| [velesdb-core](../crates/velesdb-core/README.md) | Core | Rust core library |
| [velesdb-server](../crates/velesdb-server/README.md) | Server | REST API server |
| [velesdb-cli](../crates/velesdb-cli/README.md) | CLI | Command-line interface & REPL |
| [velesdb-wasm](../crates/velesdb-wasm/README.md) | SDK | WebAssembly for browsers |
| [velesdb-python](../crates/velesdb-python/README.md) | SDK | Python bindings (PyO3) |
| [velesdb-mobile](../crates/velesdb-mobile/README.md) | SDK | iOS/Android (UniFFI) |
| [TypeScript SDK](../sdks/typescript/README.md) | SDK | TypeScript/JavaScript client |
| [velesdb-memory](../crates/velesdb-memory/README.md) | Server | MCP agent-memory server (`why()` wedge) |
| [velesdb-node](../crates/velesdb-node/README.md) | SDK | Node.js agent-memory binding (napi-rs) |
| [tauri-plugin-velesdb](../crates/tauri-plugin-velesdb/README.md) | Plugin | Tauri desktop integration |
| [LangChain](../integrations/langchain/README.md) | Integration | LangChain VectorStore |
| [LlamaIndex](../integrations/llamaindex/README.md) | Integration | LlamaIndex VectorStore |

---

## 📦 Crate Documentation

Each crate has its own README with specific documentation:

| Crate | Description |
|-------|-------------|
| [velesdb-core](../crates/velesdb-core/README.md) | Core library |
| [velesdb-server](../crates/velesdb-server/README.md) | REST API server |
| [velesdb-cli](../crates/velesdb-cli/README.md) | Command-line interface |
| [velesdb-mobile](../crates/velesdb-mobile/README.md) | iOS/Android bindings |
| [velesdb-migrate](../crates/velesdb-migrate/README.md) | Migration tools |

---

## 🔬 Design, Internals & Reference

Background documents: how the engine is built, what it guarantees, and where
the remaining debt is. Read these when you need the *why* behind a behaviour.

| Document | Description |
|----------|-------------|
| [Architecture](./ARCHITECTURE.md) | How the engine is put together, layer by layer. |
| [Concurrency model](./CONCURRENCY_MODEL.md) | Which operations run in parallel, and the locks that make that safe. |
| [Storage format](./STORAGE_FORMAT.md) | The on-disk layout and its compatibility rules. |
| [Soundness](./SOUNDNESS.md) | The invariants the engine relies on, and why they hold. |
| [Fuzzing](./FUZZING.md) | The fuzz targets and how to run them locally. |
| [GPU acceleration](./GPU_ACCELERATION.md) | What the `gpu` feature accelerates, and what it does not. |
| [ANN state-of-the-art audit](./ANN_SOTA_AUDIT.md) | How the index compares to published ANN work. |
| [Core wiring debt](./CORE_WIRING_DEBT.md) | Known gaps between what core exposes and what the surfaces use. |
| [Core / Premium split](./CORE_PREMIUM_SPLIT.md) | Where the open-core boundary sits, and the contract both repos read identically. |

---

## 📄 Project & Licensing

| Document | Description |
|----------|-------------|
| [FAQ](./FAQ.md) | The questions newcomers actually ask. |
| [Licensing](./LICENSING.md) | The VelesDB Core License, in practical terms. |
| [Business model](./BUSINESS_MODEL.md) | What is open, what is premium, and why the line sits there. |

---

## 🔗 External Resources

- [GitHub Repository](https://github.com/cyberlife-coder/VelesDB)
- [crates.io](https://crates.io/crates/velesdb-core)
- [Discord Community](https://discord.gg/velesdb)

---

*VelesDB — the explainable, local-first memory engine for AI agents. (Microsecond vector search is the proof, not the pitch.)*

---
Last updated: 2026-07-25 · Applies to: velesdb-core 4.2.0
