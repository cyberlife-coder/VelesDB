# Why VelesDB?

VelesDB is **the explainable, local-first memory engine for AI agents.** Under the hood it fuses Vector, Graph, and ColumnStore engines into a single embeddable ~9 MB binary, queried through VelesQL — the multi-engine design is *how* it delivers memory that survives restarts, connects facts, and stays on your machine. It targets AI agent workloads such as RAG, semantic search, and knowledge graphs, with sub-millisecond query latency.

The differentiator is **`why()`**: instead of returning only an answer, VelesDB returns the *evidence path* behind every recall — which facts it used and how they connect, by walking typed links to context that shares no words with your question. That built-in, auditable recall trail is exactly the kind of traceability that regulations such as the [EU AI Act](https://artificialintelligenceact.eu/implementation-timeline/) (enforceable from Aug 2026) ask of AI systems; running fully local, VelesDB **helps meet** those data-residency and explainability expectations rather than claiming certified compliance.

---

## Positioning

*Feature comparison as of March 2026. Competitor capabilities evolve; verify with official documentation.*

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

**Sources:** [Qdrant docs](https://qdrant.tech/documentation/), [Pinecone docs](https://docs.pinecone.io/), [ChromaDB docs](https://docs.trychroma.com/), [Weaviate docs](https://weaviate.io/developers/weaviate).

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

- **Native HNSW implementation** — in our internal benchmarks (5K vectors, 128D, 100 queries), our implementation measured 26.9ms vs ~32ms for `hnsw_rs`, approximately 1.2x faster. Results may vary by dataset and parameters
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
| LlamaIndex | `llama-index-vector-stores-velesdb` | Stable |

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
