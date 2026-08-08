# VelesDB Integration Tests

This directory contains integration tests that simulate real-world usage scenarios for VelesDB.
These tests serve as **living documentation** - copy-pastable examples that are guaranteed to work.

## Quick Start

```bash
# Run all integration tests
cargo test -p velesdb-core --test integration_scenarios
cargo test -p velesdb-core --test use_cases_integration_tests

# Run specific use case
cargo test -p velesdb-core --test use_cases_integration_tests use_case_1
```

---

## 📚 Use Cases (10 Documented Patterns)

See `use_cases_integration_tests.rs` for **23 tests** covering the 10 hybrid use cases from `docs/guides/USE_CASES.md`.

| # | Use Case | Tests | Key Features |
|---|----------|-------|--------------|
| 1 | Contextual RAG | 2 | Vector similarity + graph context |
| 2 | Expert Finder | 2 | Multi-hop graph + filtering |
| 3 | Knowledge Discovery | 2 | Variable-depth traversal |
| 4 | Document Clustering | 2 | GROUP BY + similarity |
| 5 | Semantic Search + Filters | 2 | Vector NEAR + metadata |
| 6 | Recommendation Engine | 2 | User-item similarity |
| 7 | Entity Resolution | 2 | High-threshold deduplication |
| 8 | Trend Analysis | 2 | Temporal aggregations |
| 9 | Impact Analysis | 2 | Dependency graph |
| 10 | Conversational Memory | 3 | Agent memory patterns |

### Example: Use Case 1 - Contextual RAG

```rust
// Find documents similar to a query
let results = collection.search(&query_embedding, 5)?;

// VelesQL equivalent
let query = "SELECT * FROM documents WHERE similarity(embedding, $q) > 0.75 LIMIT 20";
```

---

## Test Scenarios

### 1. RAG Pipeline (3 tests)

Simulates Retrieval-Augmented Generation workflows commonly used in AI applications.

| Test | Description | Validates |
|------|-------------|-----------|
| `test_rag_complete_workflow` | Full RAG pipeline with document ingestion and semantic search | Upsert, search, payload handling |
| `test_rag_incremental_updates` | Adding documents incrementally to existing collection | Batch vs incremental upsert |
| `test_rag_delete_and_search` | Deleting documents and verifying search exclusion | Delete, index consistency |

**Use Case**: Knowledge bases, document Q&A, chatbot context retrieval.

---

### 2. E-commerce Search (2 tests)

Simulates product catalog semantic search.

| Test | Description | Validates |
|------|-------------|-----------|
| `test_product_catalog_indexing` | Index products with rich metadata (name, category, price) | Payload storage, search accuracy |
| `test_batch_product_indexing_performance` | Batch insert 1000 products, verify search <100ms | Batch performance, latency |

**Performance**: Search over 1000 products completes in <100ms.

**Use Case**: E-commerce, product recommendations, catalog search.

---

### 3. Multi-Collection Workflow (3 tests)

Simulates multi-tenant or multi-domain deployments.

| Test | Description | Validates |
|------|-------------|-----------|
| `test_multi_tenant_isolation` | Data isolation between tenant collections | Collection isolation |
| `test_collection_lifecycle` | Create, list, delete collections | CRUD operations |
| `test_different_metrics_per_collection` | Different distance metrics per collection | Metric configuration |

**Use Case**: SaaS platforms, multi-tenant applications, domain separation.

---

### 4. Hybrid Search (2 tests)

Simulates combined vector + full-text search.

| Test | Description | Validates |
|------|-------------|-----------|
| `test_vector_and_text_search` | Vector search + BM25 text search | Dual search modes |
| `test_hybrid_search_ranking` | RRF fusion of vector and text results | Hybrid ranking |

**Use Case**: Document search, semantic + keyword matching.

---

### 5. Persistence (2 tests)

Simulates data durability and concurrent access.

| Test | Description | Validates |
|------|-------------|-----------|
| `test_collection_data_persistence` | Data persists after flush | Flush, durability |
| `test_concurrent_read_operations` | 4 threads performing concurrent searches | Thread safety |

**Use Case**: Production deployments, high-concurrency scenarios.

---

## Performance Benchmarks

Current numbers come from the criterion benches, not from this README — run
them locally (see the `cargo bench` commands below) and read the report in
`target/criterion/report/index.html`.

---

## Running Tests

```bash
# Run all integration tests
cargo test -p velesdb-core --test integration_scenarios

# Run specific scenario
cargo test -p velesdb-core --test integration_scenarios rag_pipeline

# Run with verbose output
cargo test -p velesdb-core --test integration_scenarios -- --nocapture

# Run benchmarks
cargo bench -p velesdb-core --bench hnsw_benchmark
cargo bench -p velesdb-core --bench simd_benchmark
cargo bench -p velesdb-core --bench bm25_benchmark
```

---

## Test Coverage

These integration tests complement the unit tests embedded in `velesdb-core`'s
source modules, providing:

- **End-to-end validation** of complete workflows
- **Performance regression detection** via timing assertions
- **Concurrency safety verification**
- **Multi-collection isolation testing**

This directory holds several dozen integration test files (`ls crates/velesdb-core/tests/*.rs`
for the current count); `cargo test -p velesdb-core` reports the authoritative totals.

---

## Known Limitations

1. **Cosine with unnormalized vectors**: When using `DistanceMetric::Cosine`, vectors should ideally be normalized to avoid numerical precision issues in edge cases.

---

## Contributing

When adding new integration tests:

1. Follow the Arrange-Act-Assert pattern
2. Use `TempDir` for isolated test environments
3. Use the `create_and_get_collection` helper
4. Add timing assertions for performance-critical paths
5. Document the use case being tested
