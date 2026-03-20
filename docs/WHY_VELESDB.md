# Why VelesDB?

VelesDB is a local-first vector database that combines Vector, Graph, and ColumnStore engines into a single, embeddable library. It is designed for AI agent workloads such as RAG, semantic search, and knowledge graphs, with sub-millisecond query latency.

---

## Positioning

| Capability | VelesDB | Qdrant | Pinecone | ChromaDB | Weaviate |
|---|---|---|---|---|---|
| **Deployment** | Embedded / local-first | Client-server | Managed cloud only | Embedded / client-server | Client-server |
| **Vector Search** | Native HNSW + AVX2/AVX-512 SIMD | HNSW | Proprietary | HNSW (hnswlib) | HNSW |
| **Graph Engine** | Built-in (nodes, edges, traversal) | No | No | No | Cross-references only |
| **ColumnStore** | Built-in typed columns | Payload indexes | Metadata filtering | Metadata filtering | Inverted indexes |
| **Query Language** | VelesQL (SQL-like + vector + graph) | REST / gRPC filters | REST filters | Python DSL | GraphQL |
| **Hybrid Search** | Vector + BM25 + Graph in one query | Vector + payload | Vector + metadata | Vector + metadata | Vector + BM25 |
| **Quantization** | SQ8 (4x), Binary (32x), PQ, RaBitQ | SQ, PQ | Proprietary | None | PQ, BQ |
| **WASM Support** | Yes (browser-side search) | No | No | No | No |
| **Mobile Support** | iOS / Android (UniFFI) | No | No | No | No |
| **Latency** | Sub-millisecond (in-process) | ~1-5 ms (network) | ~10-50 ms (cloud) | ~1-5 ms (in-process) | ~5-20 ms (network) |
| **License** | Source-available | Apache-2.0 | Proprietary | Apache-2.0 | BSD-3 |

---

## Local-First Advantages

### Zero Network Overhead

VelesDB runs in-process. There is no serialization, no TCP round-trip, and no connection pool to manage. Query latency is determined by computation, not infrastructure.

### Data Sovereignty

All data stays on the device. This matters for:

- **Privacy-sensitive applications** (medical, legal, financial)
- **Offline-capable agents** (mobile, desktop, edge devices)
- **Air-gapped environments** (defense, regulated industries)

### Simplified Operations

No cluster to provision. No replicas to manage. No cloud bills to monitor. A single `Database::open("./data")` call is the entire deployment.

### Deterministic Performance

Latency is not subject to network jitter, noisy neighbors, or cloud region availability. Benchmarks on your development machine predict production behavior.

---

## Vector + Graph + ColumnStore Architecture

Most vector databases treat metadata as a second-class citizen and have no graph support at all. VelesDB integrates all three models natively.

### Single-Query Hybrid

VelesQL supports vector similarity, graph traversal, and column filtering in a single query:

```sql
MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)
WHERE similarity(doc.embedding, $question) > 0.8
  AND author.department = 'Engineering'
RETURN author.name, doc.title
ORDER BY similarity() DESC LIMIT 5
```

This eliminates the "glue code" pattern where applications stitch together separate vector, graph, and SQL databases with application-layer joins.

### Agent Memory Patterns

The integrated architecture directly supports AI agent memory requirements:

- **Semantic memory** (vector search): long-term knowledge retrieval
- **Episodic memory** (temporal index + vectors): event timeline with similarity recall
- **Procedural memory** (vectors + reinforcement): learned action patterns with confidence scoring

---

## Performance

VelesDB achieves sub-millisecond latency through:

- **Native HNSW implementation** that is 1.2x faster than the widely-used `hnsw_rs` crate (26.9ms vs ~32ms on 100 queries, 5K/128D vectors)
- **Explicit SIMD kernels** (AVX-512, AVX2, NEON) with runtime feature detection
- **Memory-mapped storage** for zero-copy vector access
- **Lock-free read paths** using `parking_lot::RwLock` with 256-shard concurrent edge stores

See [BENCHMARKS.md](./BENCHMARKS.md) for detailed numbers and methodology.

---

## Platform Coverage

| Platform | Crate | Status |
|---|---|---|
| Rust (native) | `velesdb-core` | Stable |
| REST API | `velesdb-server` | Stable |
| Python | `velesdb-python` (PyO3) | Stable |
| TypeScript | `@velesdb/sdk` | Stable |
| Browser (WASM) | `velesdb-wasm` | Stable |
| iOS / Android | `velesdb-mobile` (UniFFI) | Stable |
| Tauri Desktop | `tauri-plugin-velesdb` | Stable |
| LangChain | `langchain-velesdb` | Stable |
| LlamaIndex | `llamaindex-velesdb` | Stable |

---

## When to Use VelesDB

**Good fit:**

- Embedded AI agents that need vector + graph + metadata in one engine
- Desktop or mobile applications with local-first requirements
- Privacy-sensitive workloads where data must not leave the device
- Prototyping and development (zero infrastructure setup)
- Edge deployments with limited or no network connectivity

**Consider alternatives if:**

- You need a managed cloud service with zero operational burden (Pinecone)
- You need distributed multi-node clustering for billion-scale datasets (Qdrant, Weaviate)
- You only need simple vector search without graph or column queries (ChromaDB)
