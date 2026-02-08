# Testing Patterns

**Analysis Date:** 2026-02-06

## Test Framework

**Runner:** Built-in Rust test (`cargo test`)

**Assertion Library:** Standard Rust assertions (`assert!`, `assert_eq!`, etc.)

**Additional Testing Dependencies:**
- `criterion` - Benchmark framework
- `proptest` - Property-based testing
- `tempfile` - Temporary files for tests
- `tokio-test` - Async testing utilities
- `serial_test` - Sequential test execution (for GPU tests)
- `loom` - Concurrency testing (optional feature)

**Run Commands:**
```bash
# Run all tests
cargo test --workspace

# Run tests for specific crate
cargo test -p velesdb-core

# Run with single thread (if parallel issues)
cargo test -p velesdb-core -- --test-threads=1

# Run with output
cargo test -- --nocapture

# Run ignored/benchmark tests
cargo test -- --ignored

# Run GPU tests (requires GPU)
cargo test -p velesdb-core --features gpu

# Run loom concurrency tests (requires nightly)
cargo +nightly test --features loom --test loom_tests
```

## Test File Organization

**Location Pattern:**
- Unit tests: `[module]_tests.rs` alongside source files
- Integration tests: `tests/*.rs` at crate root
- Benchmarks: `benches/*.rs` at crate root

**Naming Convention:**
```rust
#[test]
fn test_[function]_[scenario]_[expected_result]() {
    // ...
}

// Examples:
fn test_insert_single_vector_returns_id()
fn test_search_empty_collection_returns_empty_vec()
fn test_hnsw_build_with_1000_vectors_completes_under_1s()
fn test_validate_multiple_similarity_with_or_detected()
```

**Directory Structure:**
```
crates/velesdb-core/
├── src/
│   ├── lib.rs
│   ├── simd_native.rs
│   ├── simd_native_tests.rs      # Unit tests for simd_native
│   ├── velesql/
│   │   ├── mod.rs
│   │   ├── parser.rs
│   │   └── parser_tests.rs        # Unit tests for parser
│   └── ...
├── tests/
│   ├── recall_validation.rs       # Integration tests
│   ├── loom_tests.rs              # Concurrency tests
│   └── crash_recovery_tests.rs    # Recovery tests
└── benches/
    ├── search_benchmark.rs        # Performance benchmarks
    └── simd_benchmark.rs          # SIMD benchmarks
```

## Test Structure

**AAA Pattern (Arrange-Act-Assert):**
```rust
#[test]
fn test_example() {
    // ARRANGE - Setup
    let collection = Collection::new("test", 128, DistanceMetric::Cosine);
    
    // ACT - Action
    let result = collection.insert(vector);
    
    // ASSERT - Verification
    assert!(result.is_ok());
    assert_eq!(collection.len(), 1);
}
```

**Given-When-Then Comments:**
```rust
#[test]
fn test_validate_multiple_similarity_with_or_detected() {
    // Given: A query with multiple similarity() conditions using OR
    let query = create_query_with_multiple_similarity_or();

    // When: Validation is performed
    let result = QueryValidator::validate(&query);

    // Then: ValidationError is returned - OR is not supported
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind, ValidationErrorKind::MultipleSimilarity);
}
```

## Mocking

**Framework:** No external mocking framework - use manual test doubles

**Pattern:**
```rust
// Create test-specific implementations
struct TestEdgeStore {
    // Simplified version for testing
}

impl TestEdgeStore {
    fn new() -> Self { ... }
}

#[test]
fn test_edge_store_behavior() {
    let store = TestEdgeStore::new();
    // Test the behavior
}
```

**What to Mock:**
- External dependencies (file system, network)
- Time sources for deterministic tests
- Random number generators

**What NOT to Mock:**
- Core algorithm implementations
- Data structures being tested

## Test Fixtures and Factories

**Helper Functions Pattern:**
```rust
// Factory functions for test data
fn create_simple_query() -> Query {
    Query {
        select: SelectStatement {
            distinct: DistinctMode::None,
            columns: SelectColumns::All,
            from: "docs".to_string(),
            // ...
        },
        compound: None,
        match_clause: None,
    }
}

fn create_query_with_similarity_or_metadata() -> Query {
    // Build specific test query
}
```

**Synthetic Data Generation:**
```rust
/// Generate synthetic vectors for testing.
#[allow(clippy::cast_precision_loss)]
fn generate_vectors(count: usize, dim: usize) -> Vec<Vec<f32>> {
    (0..count)
        .map(|i| {
            (0..dim)
                .map(|d| ((i * 31 + d * 17) % 1000) as f32 / 1000.0)
                .collect()
        })
        .collect()
}
```

## GPU Tests

**Required Annotation:**
```rust
use serial_test::serial;

#[test]
#[serial(gpu)]  // OBLIGATOIRE pour éviter deadlocks wgpu/pollster
fn test_gpu_xxx() {
    if let Some(gpu) = GpuAccelerator::new() {
        // Test GPU functionality
    }
}
```

## Performance/Benchmark Tests

**Criterion Benchmarks:**
```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn benchmark_search(c: &mut Criterion) {
    let data = setup_data();
    
    c.bench_function("search_10k_vectors", |b| {
        b.iter(|| {
            search(black_box(&data), black_box(&query), 10)
        })
    });
}

criterion_group!(benches, benchmark_search);
criterion_main!(benches);
```

**Performance Tests (Ignored by Default):**
```rust
#[test]
#[ignore = "Benchmark test - run manually with --ignored"]
fn test_recall_vs_ef() {
    // Long-running performance test
}
```

**Inline Performance Assertions:**
```rust
#[test]
fn test_perf_search_latency() {
    let start = std::time::Instant::now();
    // ... perform search ...
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(100), 
            "Search took {:?}, expected < 100ms", elapsed);
}
```

## Concurrency Tests

**Loom Tests (for concurrency verification):**
```rust
//! Loom concurrency tests for concurrent structures.
#![cfg(all(loom, feature = "persistence"))]

use loom::sync::Arc;
use loom::thread;

#[test]
fn test_loom_concurrent_edge_insert() {
    loom::model(|| {
        let store = Arc::new(LoomEdgeStore::new(4));
        
        let s1 = Arc::clone(&store);
        let t1 = thread::spawn(move || {
            s1.add_edge(TestEdge::new(1, 0, 4, "knows"))
        });
        
        let s2 = Arc::clone(&store);
        let t2 = thread::spawn(move || {
            s2.add_edge(TestEdge::new(2, 0, 8, "likes"))
        });
        
        t1.join().unwrap();
        t2.join().unwrap();
        
        assert_eq!(store.edge_count(), 2);
    });
}
```

## Coverage

**No Enforced Target:** Coverage requirements not explicitly configured

**View Coverage:**
```bash
# Generate coverage report (requires cargo-tarpaulin or similar)
cargo tarpaulin --out Html
```

## Test Types

**Unit Tests:**
- Location: `src/*_tests.rs` or inline `#[cfg(test)]` modules
- Scope: Single functions or small modules
- Fast execution (<1s per test)
- No I/O or external dependencies

**Integration Tests:**
- Location: `tests/*.rs`
- Scope: Multiple modules, end-to-end workflows
- May use temporary files
- Example: `tests/recall_validation.rs`

**Property-Based Tests:**
- Framework: `proptest`
- Location: With integration tests
- Pattern: `proptest! { ... }`

**Concurrency Tests:**
- Framework: `loom` (optional feature)
- Location: `tests/loom_tests.rs`
- Run with: `cargo +nightly test --features loom --test loom_tests`

## Common Patterns

**Async Testing:**
```rust
#[tokio::test]
async fn test_async_operation() {
    let result = async_operation().await;
    assert!(result.is_ok());
}
```

**Error Testing:**
```rust
#[test]
fn test_error_condition() {
    let result = operation_that_fails();
    assert!(result.is_err());
    
    let err = result.unwrap_err();
    assert_eq!(err.kind, ExpectedErrorKind);
}
```

**Float Comparison:**
```rust
// Use epsilon comparison for floats
assert!((result - expected).abs() < 1e-5);

// Or for larger values
assert!(
    (result - expected).abs() < 0.01,
    "result={result}, expected={expected}"
);
```

**Test Organization with Modules:**
```rust
// In velesql/mod.rs
#[cfg(test)]
mod validation_tests;
#[cfg(test)]
mod parser_tests;
#[cfg(test)]
mod planner_tests;
// ... many more test modules
```

## Checklist New Test

Per `crates/velesdb-core/tests/AGENTS.md`:

- [ ] Nom descriptif (`test_[fonction]_[scenario]_[resultat]`)
- [ ] Structure AAA respectée
- [ ] `#[serial(gpu)]` si utilise wgpu
- [ ] `#[ignore]` si test de performance long
- [ ] Assertions significatives (pas juste `assert!(true)`)

---

*Testing analysis: 2026-02-06*
