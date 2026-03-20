# VelesDB Business Model

## Open-Core Architecture

VelesDB follows an open-core model under the VelesDB Core License 1.0 (based on ELv2).
The core engine ships with full search, graph, and AI agent capabilities.
Premium features are injected via the `DatabaseObserver` trait -- no code forks needed.

## Core Features (VelesDB Core License 1.0 -- Source Available)

- **Vector Search**: HNSW with SIMD acceleration (AVX-512/AVX2/NEON), sub-millisecond latency
- **Knowledge Graph**: Full graph engine with BFS/DFS traversal, MATCH queries
- **VelesQL**: SQL-like query language with `similarity()` and graph pattern matching
- **Hybrid Search**: Dense + sparse (BM25) fusion with RRF/RSF strategies
- **Agent Memory SDK**: Semantic, episodic, and procedural memory patterns for AI agents
- **Multi-platform SDKs**: Python (PyO3), WASM, Mobile (iOS/Android), Tauri, TypeScript
- **Ecosystem Integrations**: LangChain, LlamaIndex connectors

## Premium Features (Commercial License)

Premium capabilities are delivered through the `DatabaseObserver` hook system:

- **Encryption at Rest**: AES-256-GCM for data-at-rest protection
- **High Availability**: Raft-based cluster consensus for fault tolerance
- **Agent Hooks & Triggers**: Event-driven callbacks (on_upsert, on_query, on_collection_created)
- **Multi-tenancy**: Namespace isolation with per-tenant resource quotas
- **Advanced Analytics**: EXPLAIN ANALYZE with query plan visualization
- **WebAdmin UI**: Browser-based management dashboard
- **Priority Support**: Dedicated engineering support channel

## Extensibility Model

The `DatabaseObserver` trait enables premium features without forking the core:

```rust
pub trait DatabaseObserver: Send + Sync {
    fn on_collection_created(&self, name: &str, kind: &CollectionType);
    fn on_collection_deleted(&self, name: &str);
    fn on_upsert(&self, collection: &str, point_count: usize);
    fn on_query(&self, collection: &str, duration_us: u64);
}
```

All methods have default no-op implementations. The overhead when no observer is
attached is a single pointer check. Premium implements this trait and injects it
via `Database::open_with_observer(path, observer)`.

This design ensures:

- **Zero coupling**: The core library has no knowledge of premium internals.
- **No code forks**: Premium is a separate crate that depends on the core.
- **Minimal overhead**: Community users pay no runtime cost for hooks they don't use.

## Target Market

- **AI/ML Teams**: RAG pipelines, semantic search, knowledge graph construction
- **Agent Developers**: Autonomous agent memory (LangChain, CrewAI, AutoGPT)
- **Edge/Embedded**: Local-first deployments (mobile, desktop, IoT) via WASM and native bindings

## Competitive Differentiators

| Capability | VelesDB | Typical Competitors |
|------------|---------|---------------------|
| Unified Vector + Graph engine | Yes | Separate systems |
| Self-contained single binary (~6 MB) | Yes | Containers / clusters |
| Sub-millisecond latency (43 us) | Yes | 50-100 ms (cloud) |
| WASM / Mobile native | Yes | Server-only |
| SQL-like query language (VelesQL) | Yes | JSON DSL / SDK-only |

## Deployment Options

| Tier | Deployment | License |
|------|------------|---------|
| Community | Single-node, self-hosted | VelesDB Core License 1.0 |
| Professional | Multi-node, managed | Commercial |
| Enterprise | On-premise cluster with SLA | Commercial |
