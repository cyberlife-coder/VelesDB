# VelesDB Integration Tests

This directory contains integration tests that simulate real-world usage scenarios for VelesDB.

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

Based on criterion benchmarks (Intel/AMD x86_64):

### Distance Calculations (768D vectors)

| Operation | Time | Speedup vs Baseline |
|-----------|------|---------------------|
| Dot Product (SIMD) | ~42 ns | 6.5x faster |
| Cosine Similarity | ~45 ns | 6.0x faster |
| Euclidean Distance | ~39 µs | 30% improved |
| Normalize In-place | ~218 ns | 15% improved |

### Search Operations

| Operation | Time | Notes |
|-----------|------|-------|
| Vector Search (1000 docs) | ~60 µs | HNSW index |
| Text Search (BM25) | ~32 µs | Inverted index |
| Hybrid Search | ~63 µs | RRF fusion |

### Recall Validation

| Dimension | Time | Status |
|-----------|------|--------|
| 128D | ~34 ms | ✅ Stable |
| 384D | ~52 ms | ✅ Stable |
| 768D | ~109 ms | ✅ Improved 7% |
| 1536D | ~257 ms | ✅ Stable |

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

These integration tests complement the 346 unit tests in `velesdb-core`, providing:

- **End-to-end validation** of complete workflows
- **Performance regression detection** via timing assertions
- **Concurrency safety verification**
- **Multi-collection isolation testing**

Total test count: **358 tests** (346 unit + 12 integration)

---

## Known Limitations

1. **Persistence across restarts**: The current `Database` API doesn't automatically reload collections on reopen. Use `Collection::open()` directly for persistence testing.

2. **Cosine with unnormalized vectors**: When using `DistanceMetric::Cosine`, vectors should ideally be normalized to avoid numerical precision issues in edge cases.

---

## Contributing

When adding new integration tests:

1. Follow the Arrange-Act-Assert pattern
2. Use `TempDir` for isolated test environments
3. Use the `create_and_get_collection` helper
4. Add timing assertions for performance-critical paths
5. Document the use case being tested
