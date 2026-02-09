# VelesDB Core — Feature Truth Matrix

**Audit date:** 2026-02-09 (updated post-Phase 8)
**Source:** Systematic codebase verification against grammar.pest, parser, executor, server routes

Legend:
- ✅ **Works** = Parser + Executor + Tests
- 🟡 **Parse-only** = Grammar + Parser, NO execution path
- ⚠️ **Caveat** = Works with limitations
- ❌ **Missing** = Not implemented

---

## 1. VelesQL — SELECT Queries

| Feature | Status | Evidence |
|---------|--------|----------|
| `SELECT columns FROM table` | ✅ Works | `query/mod.rs:execute_query` |
| `SELECT *` | ✅ Works | `grammar.pest:122` |
| `SELECT DISTINCT` | ✅ Works | `distinct.rs`, EPIC-052 US-001 |
| `FROM table AS alias` | ✅ Works | `grammar.pest:71`, EPIC-052 US-003 |
| `WHERE comparisons (=, <>, <, >, <=, >=)` | ✅ Works | `grammar.pest:208` |
| `AND / OR / NOT` | ✅ Works | `grammar.pest:142-143` |
| `IN (value_list)` | ✅ Works | `grammar.pest:192` |
| `BETWEEN val1 AND val2` | ✅ Works | `grammar.pest:196` |
| `LIKE / ILIKE pattern` | ✅ Works | `grammar.pest:199-200`, `filter_like_tests.rs` |
| `IS NULL / IS NOT NULL` | ✅ Works | `grammar.pest:203` |
| `vector NEAR $v` | ✅ Works | `grammar.pest:169-171` |
| `vector NEAR_FUSED [v1,v2] USING FUSION 'rrf'` | ✅ Works | `grammar.pest:174-183`, `dispatch.rs:29-77` |
| `similarity(field, vector) > threshold` | ✅ Works | `grammar.pest:161-163` |
| `column MATCH 'text'` (BM25) | ✅ Works | `grammar.pest:189` |
| `GROUP BY columns` | ✅ Works | `grammar.pest:85`, `aggregation/mod.rs` |
| `HAVING aggregate_fn op value` | ✅ Works | `grammar.pest:92` |
| `COUNT / SUM / AVG / MIN / MAX` | ✅ Works | `grammar.pest:131` |
| `ORDER BY column ASC/DESC` | ✅ Works | `grammar.pest:109` |
| `ORDER BY similarity(field, vector)` | ✅ Works | `grammar.pest:112` |
| `ORDER BY aggregate_function` | ✅ Works | `grammar.pest:111` |
| `LIMIT n` | ✅ Works | `grammar.pest:211` |
| `OFFSET n` | ✅ Works | `grammar.pest:212` |
| `WITH (param=value)` | ✅ Works | `grammar.pest:116-119` |
| `USING FUSION (strategy=..., weight_0=...)` | ✅ Works | `grammar.pest:78-82` |
| Scalar subqueries in WHERE | ✅ Works | `grammar.pest:217-218`, VP-002, `subquery_tests.rs` |
| Temporal: `NOW()`, `INTERVAL '7 days'` | ✅ Works | `grammar.pest:224-228`, `temporal_tests.rs` |
| Temporal arithmetic: `NOW() - INTERVAL '7d'` | ✅ Works | `grammar.pest:225`, converts to epoch seconds |
| Quoted identifiers: \`col\`, "col" | ✅ Works | `grammar.pest:237-251` |
| `JOIN table ON condition` (INNER) | ✅ Works | `grammar.pest:99`, `join.rs`, `Database::execute_query()` Phase 8 |
| `LEFT JOIN` | ✅ Works | `grammar.pest:100`, `join.rs` LEFT JOIN support, Phase 8 Plan 08-02 |
| `RIGHT/FULL JOIN` | ⚠️ **Caveat** | Parsed; returns `UnsupportedFeature` error at execution (Phase 8) |
| `JOIN ... USING (col)` | 🟡 **Parse-only** | `grammar.pest:103`, USING not supported in executor |
| `UNION / UNION ALL` | ✅ Works | `grammar.pest:57`, `compound.rs`, `Database::execute_query()` Phase 8 |
| `INTERSECT / EXCEPT` | ✅ Works | `grammar.pest:57`, `compound.rs`, `Database::execute_query()` Phase 8 |

---

## 2. VelesQL — MATCH Queries (Graph)

| Feature | Status | Evidence |
|---------|--------|----------|
| `MATCH (a:Label)` node pattern | ✅ Works | `grammar.pest:22-31`, `match_exec/mod.rs` |
| `MATCH (a)-[:REL]->(b)` single hop | ✅ Works | `grammar.pest:34-37`, BFS traversal |
| `MATCH (a)-[:R1]->(b)-[:R2]->(c)` multi-hop | ✅ Works | VP-004, `execute_multi_hop_chain()` |
| `MATCH (a)-[*1..3]->(b)` variable-length | ✅ Works | `grammar.pest:43-44`, `compute_max_depth()` |
| `MATCH ... WHERE condition` | ✅ Works | `match_exec/where_eval.rs` |
| `MATCH ... WHERE similarity() > threshold` | ✅ Works | `match_exec/similarity.rs:109-178` |
| Subqueries in MATCH WHERE | ✅ Works | `where_eval.rs:38-39`, VP-002 |
| `RETURN a.name, b.title` | ✅ Works | `grammar.pest:48-53`, EPIC-058 US-007 |
| `RETURN COUNT(*)` aggregation | ✅ Works | `return_agg.rs` |
| `RETURN similarity()` | ✅ Works | `grammar.pest:52` |
| `ORDER BY` in MATCH | ✅ Works | EPIC-045 US-005, `match_parser.rs:39-65` |
| `LIMIT` in MATCH | ✅ Works | `match_parser.rs:67-73` |
| Cross-store: MATCH + NEAR | ✅ Works | VP-010, `query/mod.rs:192-223` |
| Bidirectional relationships `<-[]-` | ✅ Works | `grammar.pest:35` |
| Undirected relationships `-[]-` | ✅ Works | `grammar.pest:37` |
| Relationship type filters `[:TYPE1\|TYPE2]` | ✅ Works | `grammar.pest:41` |
| Node property inline `{key: value}` | ✅ Works | `grammar.pest:28-31` |

---

## 3. Distance Metrics

| Metric | Status | Evidence |
|--------|--------|----------|
| Cosine | ✅ Works | `distance.rs`, SIMD optimized |
| Euclidean (L2) | ✅ Works | `distance.rs`, SIMD optimized |
| Dot Product | ✅ Works | `distance.rs`, SIMD optimized |
| Hamming | ✅ Works | `distance.rs`, Harley-Seal SIMD |
| Jaccard | ✅ Works | `distance.rs` |

---

## 4. Quantization

| Feature | Status | Evidence |
|---------|--------|----------|
| SQ8 (Scalar Quantization 8-bit) | ✅ Works | `quantization.rs` |
| Binary Quantization | ✅ Works | `quantization.rs` |
| Half-precision (f16) | ✅ Works | `half_precision.rs` |
| Dual-precision search | ✅ Works | `hnsw/native/dual_precision.rs` |

---

## 5. Indexes

| Feature | Status | Evidence |
|---------|--------|----------|
| HNSW (Hierarchical Navigable Small World) | ✅ Works | `index/hnsw/` |
| Configurable ef_search (via WITH clause) | ✅ Works | `query/mod.rs:144` |
| Property Index (Hash) | ✅ Works | `collection/graph/property_index/` |
| Property Index (Range) | ✅ Works | `collection/graph/property_index/` |
| Trigram Index (text search) | ✅ Works | `index/trigram/` |
| BM25 scoring | ✅ Works | `index/bm25.rs` |
| Auto-reindex | ✅ Works | `collection/auto_reindex/` |

---

## 6. REST API Endpoints (Server)

### Actually Routed ✅

| Method | Path | Handler |
|--------|------|---------|
| GET | `/health` | `health_check` |
| GET | `/collections` | `list_collections` |
| POST | `/collections` | `create_collection` |
| GET | `/collections/{name}` | `get_collection` |
| DELETE | `/collections/{name}` | `delete_collection` |
| GET | `/collections/{name}/empty` | `is_empty` |
| POST | `/collections/{name}/flush` | `flush_collection` |
| POST | `/collections/{name}/points` | `upsert_points` |
| GET | `/collections/{name}/points/{id}` | `get_point` |
| DELETE | `/collections/{name}/points/{id}` | `delete_point` |
| POST | `/collections/{name}/search` | `search` |
| POST | `/collections/{name}/search/batch` | `batch_search` |
| POST | `/collections/{name}/search/multi` | `multi_query_search` |
| POST | `/collections/{name}/search/text` | `text_search` |
| POST | `/collections/{name}/search/hybrid` | `hybrid_search` |
| GET | `/collections/{name}/indexes` | `list_indexes` |
| POST | `/collections/{name}/indexes` | `create_index` |
| DELETE | `/collections/{name}/indexes/{label}/{property}` | `delete_index` |
| POST | `/query` | `query` (VelesQL) |
| POST | `/collections/{name}/match` | `match_query` |
| GET | `/collections/{name}/graph/edges` | `get_edges` |
| POST | `/collections/{name}/graph/edges` | `add_edge` |
| POST | `/collections/{name}/graph/traverse` | `traverse_graph` |
| GET | `/collections/{name}/graph/nodes/{node_id}/degree` | `get_node_degree` |
| GET | `/metrics` | `prometheus_metrics` (feature-gated) |
| POST | `/query/explain` | `explain` handler (Phase 8, Plan 08-04) |
| — | `/swagger-ui` | Swagger UI |

---

## 7. Ecosystem Components

| Component | Path | Status |
|-----------|------|--------|
| velesdb-core | `crates/velesdb-core/` | ✅ Production |
| velesdb-server | `crates/velesdb-server/` | ✅ Production |
| velesdb-cli | `crates/velesdb-cli/` | ✅ Production |
| velesdb-python (PyO3) | `crates/velesdb-python/` | ✅ Functional |
| velesdb-wasm | `crates/velesdb-wasm/` | ✅ Functional |
| velesdb-mobile (UniFFI) | `crates/velesdb-mobile/` | ⚠️ Minimal (5 src files) |
| tauri-plugin-velesdb | `crates/tauri-plugin-velesdb/` | ✅ Functional |
| TypeScript SDK | `sdks/typescript/` | ✅ Functional |
| LangChain integration | `integrations/langchain/` | ✅ Functional |
| LlamaIndex integration | `integrations/llamaindex/` | ✅ Functional |
| velesdb-migrate | `crates/velesdb-migrate/` | ✅ Functional |
| WASM browser demo | `examples/wasm-browser-demo/` | ✅ Exists (HTML+README) |
| E-commerce example | `examples/ecommerce_recommendation/` | ✅ Full example |
| RAG PDF demo | `demos/rag-pdf-demo/` | ✅ Full demo |
| Tauri RAG app | `demos/tauri-rag-app/` | ✅ Full demo |

---

## 8. Core Modules

| Module | Status | Evidence |
|--------|--------|----------|
| Collection (vector store) | ✅ Works | `collection/` |
| EdgeStore (graph edges) | ✅ Works | `collection/graph/` |
| ColumnStore (structured data) | ✅ Works | `column_store/` |
| VelesQL Parser (PEST) | ✅ Works | `velesql/grammar.pest` |
| VelesQL Executor | ✅ Works | `collection/search/query/` |
| SIMD dispatch (x86 + ARM) | ✅ Works | `simd_native/`, `simd_neon/` |
| Agent memory (episodic) | ✅ Works | `agent/episodic_memory.rs` |
| Agent memory (procedural) | ✅ Works | `agent/procedural_memory.rs` |
| Agent reinforcement | ✅ Works | `agent/reinforcement.rs` |
| Agent TTL | ✅ Works | `agent/ttl.rs` |
| Agent temporal index | ✅ Works | `agent/temporal_index.rs` |
| Query cache (LRU) | ✅ Works | `cache/` |
| GPU acceleration | ⚠️ Feature-gated | `gpu.rs` |
| Storage (mmap) | ✅ Works | `storage/mmap.rs` |
| Compression | ✅ Works | `compression.rs` |
| Fusion strategies | ✅ Works | `fusion.rs` |
| IR metrics (NDCG, MRR, etc.) | ✅ Works | `metrics.rs` |
| Update check | ✅ Works | `update_check.rs` |
| Guardrails | ✅ Works | `guardrails.rs` |

---

## 9. Server Features

| Feature | Status | Evidence |
|---------|--------|----------|
| Axum HTTP server | ✅ Works | `velesdb-server/src/main.rs` |
| Swagger/OpenAPI | ✅ Works | `utoipa` annotations + SwaggerUi |
| CORS permissive | ✅ Works | `CorsLayer::permissive()` |
| Request tracing | ✅ Works | `TraceLayer::new_for_http()` |
| 100MB upload limit | ✅ Works | `DefaultBodyLimit::max(100MB)` |
| Prometheus metrics | ⚠️ Feature-gated | `#[cfg(feature = "prometheus")]` |

---

*This document is the source of truth for what velesdb-core delivers. Update it when features ship.*
