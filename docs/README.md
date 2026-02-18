# 📚 VelesDB Documentation

Welcome to the VelesDB documentation. This guide will help you get started and make the most of VelesDB.

---

## Quick Links

- **[Getting Started](./getting-started.md)** - Quick installation and first steps
- **[Why VelesDB?](./WHY_VELESDB.md)** - Our unique value proposition
- **[Benchmarks](./BENCHMARKS.md)** - Performance comparison with other vector databases

---

## 📖 User Guides

Detailed guides for using VelesDB features:

| Guide | Description |
|-------|-------------|
| [Installation](./guides/INSTALLATION.md) | All installation methods (cargo, binaries, Docker) |
| [Configuration](./guides/CONFIGURATION.md) | `velesdb.toml` configuration reference |
| [Search Modes](./guides/SEARCH_MODES.md) | Understanding Fast/Balanced/Accurate/HighRecall/Perfect modes |
| [CLI & REPL](./guides/CLI_REPL.md) | Command-line interface and interactive shell |
| [Quantization](./guides/QUANTIZATION.md) | Vector compression (SQ8, Binary) |

---

## 📐 Technical Reference

In-depth technical documentation:

| Reference | Description |
|-----------|-------------|
| [Architecture](./reference/ARCHITECTURE.md) | System design and internals |
| [VelesQL Specification](./reference/VELESQL_SPEC.md) | Query language grammar and syntax |
| [VelesQL Contract](./reference/VELESQL_CONTRACT.md) | Canonical REST contract (`/query`, `/match`, error model) |
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
| [Ecosystem Sync Report](./ecosystem-sync.md) | **Overview** | Feature parity matrix across all components |
| [velesdb-core](../crates/velesdb-core/README.md) | Core | Rust core library |
| [velesdb-server](../crates/velesdb-server/README.md) | Server | REST API server |
| [velesdb-cli](../crates/velesdb-cli/README.md) | CLI | Command-line interface & REPL |
| [velesdb-wasm](../crates/velesdb-wasm/README.md) | SDK | WebAssembly for browsers |
| [velesdb-python](../crates/velesdb-python/README.md) | SDK | Python bindings (PyO3) |
| [velesdb-mobile](../crates/velesdb-mobile/README.md) | SDK | iOS/Android (UniFFI) |
| [TypeScript SDK](../sdks/typescript/README.md) | SDK | TypeScript/JavaScript client |
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

## 🔗 External Resources

- [GitHub Repository](https://github.com/velesdb/velesdb)
- [crates.io](https://crates.io/crates/velesdb-core)
- [Discord Community](https://discord.gg/velesdb)

---

*VelesDB — Vector Search in Microseconds*
