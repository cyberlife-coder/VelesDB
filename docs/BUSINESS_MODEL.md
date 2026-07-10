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
- **Agent Memory SDK**: Semantic, episodic, and procedural memory patterns for AI agents,
  including in-process state serialization (`AgentMemory::snapshot()`) for Rust
- **Multi-platform SDKs**: Python (PyO3), WASM, Mobile (iOS/Android), Tauri, TypeScript
- **Ecosystem Integrations**: LangChain, LlamaIndex connectors

> **Component licensing.** Every artifact that embeds the engine (the SDKs above
> compile or bundle `velesdb-core`) ships under the VelesDB Core License 1.0.
> Only the ecosystem connectors (`integrations/*`) and sample code
> (`examples/*`, `demos/*`) are MIT. See [LICENSING.md](LICENSING.md) for the
> full matrix and the rule that decides it.

## Premium Features (Commercial License)

Premium capabilities are delivered through the `DatabaseObserver` hook system:

- **Encryption at Rest**: AES-256-GCM for data-at-rest protection
- **High Availability**: Raft-based cluster consensus for fault tolerance
- **Database Snapshots**: Managed point-in-time backups of the full database state,
  with retention policies and restore workflows. Available under Enterprise.
  > **Distinct from Agent Memory serialization**: the `AgentMemory::snapshot()` /
  > `load_latest_snapshot()` API in the Rust SDK is a **Core** primitive that
  > persists only agent memory subsystem state (semantic/episodic/procedural +
  > TTL). It is entirely separate from database-level backups.
- **Agent Hooks & Triggers**: Event-driven callbacks (on_upsert, on_query, on_collection_created)
- **Multi-tenancy**: Namespace isolation with per-tenant resource quotas
- **Advanced Analytics**: EXPLAIN ANALYZE with query plan visualization
- **WebAdmin UI**: Browser-based management dashboard
- **Priority Support**: Dedicated engineering support channel

## Extensibility Model

The `DatabaseObserver` trait enables premium features without forking the core:

```rust
pub trait DatabaseObserver: Send + Sync {
    // Lifecycle
    fn on_collection_created(&self, name: &str, kind: &CollectionType);
    fn on_collection_deleted(&self, name: &str);

    // Telemetry (core-invoked, since 3.9.0)
    fn on_upsert(&self, collection: &str, point_count: usize);
    fn on_query(&self, collection: &str, duration_us: u64);

    // Control-plane gates
    fn on_ddl_request(&self, operation: &str, collection: &str) -> Result<()>;
    fn on_dml_mutation_request(&self, operation: &str, collection: &str) -> Result<()>;
    fn on_query_request(&self, ctx: &QueryAccessContext) -> Result<AccessDecision>; // read path, since 3.9.0
}
```

All methods have default implementations (telemetry/lifecycle no-op, gates
allow-all), so the overhead when no observer is attached is a single pointer
check and existing implementers keep compiling as new hooks are added. Premium
implements this trait and injects it via
`Database::open_with_observer(path, observer)`.

Since 3.9.0 the port covers the **read path**: `on_query_request` fires inside
the core use-case layer before every query executes and returns an
`AccessDecision` (`Allow` / `Deny` / `AllowWithScope` — the latter AND-composes
a tenant/row filter into the query). This means RBAC, tenant isolation, and
audit apply to **every** consumer through the port, not only the REST adapter.
Telemetry (`on_upsert` / `on_query`) is also core-invoked, so consumers emit
consistent telemetry without firing it manually.

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
| Self-contained single binary (~9 MB) | Yes | Containers / clusters |
| Sub-millisecond latency (43 us) | Yes | 50-100 ms (cloud) |
| WASM / Mobile native | Yes | Server-only |
| SQL-like query language (VelesQL) | Yes | JSON DSL / SDK-only |

## Deployment Options

| Tier | Deployment | License |
|------|------------|---------|
| Community | Single-node, self-hosted | VelesDB Core License 1.0 |
| Professional | Multi-node, managed | Commercial |
| Enterprise | On-premise cluster with SLA | Commercial |
