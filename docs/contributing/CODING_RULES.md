# VelesDB-Core - Development Rules

## Project Goal

VelesDB-Core is the **open-source vector database engine**. It provides the public API and core functionality consumed by VelesDB-Premium.

---

## Architecture

### Crate Structure

```
velesdb-core/
├── crates/
│   ├── velesdb-core/          # Core engine (storage, indexing, search)
│   ├── velesdb-server/        # Axum REST API server
│   ├── velesdb-cli/           # CLI / VelesQL REPL
│   ├── velesdb-python/        # Python bindings (PyO3)
│   ├── velesdb-wasm/          # Browser WASM (no persistence)
│   ├── velesdb-mobile/        # iOS/Android (UniFFI)
│   ├── velesdb-migrate/       # Migration tooling
│   └── tauri-plugin-velesdb/  # Tauri plugin
```

### Architectural Principles

- **Separation of concerns**: Each module has a single responsibility
- **Stable API**: Core is a versioned dependency of Premium
- **Zero-copy**: Prefer `&[u8]`, `Bytes`, `memmap2` for performance
- **Concurrency**: Use `parking_lot::RwLock` throughout (not `std::sync::RwLock`)
- **Error handling**: Use `thiserror` for typed errors. Do not use `anyhow` in library crates.

---

## Test-Driven Development (TDD)

### Required Workflow

1. **Red**: Write a failing test
2. **Green**: Write the minimum code to pass the test
3. **Blue**: Refactor without breaking tests

### Minimum Coverage

- **Target**: > 80% code coverage
- **Tool**: `cargo tarpaulin`

### Test Execution

Tests must run single-threaded to avoid file system races between test fixtures:

```bash
cargo test --workspace --features persistence,gpu,update-check \
  --exclude velesdb-python -- --test-threads=1
```

### Test Types

```rust
// Unit test (in the same file)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_basic() {
        // Arrange
        // Act
        // Assert
    }
}

// Integration test (in tests/)
#[tokio::test]
async fn test_integration_scenario() {
    // ...
}

// Benchmark (in benches/)
fn benchmark_search(c: &mut Criterion) {
    // ...
}
```

---

## Code Standards

### Formatting

```bash
cargo fmt --all -- --check
```

### Linting

Use explicit features -- never `--all-features`:

```bash
cargo clippy --workspace --all-targets --features persistence,gpu,update-check \
  --exclude velesdb-python -- -D warnings -D clippy::pedantic
```

### Naming Conventions

| Type | Convention | Example |
|------|------------|---------|
| Structs | PascalCase | `VectorIndex` |
| Traits | PascalCase | `Searchable` |
| Functions | snake_case | `find_nearest` |
| Constants | SCREAMING_SNAKE | `MAX_DIMENSIONS` |
| Modules | snake_case | `vector_storage` |

### Code Quality Limits

| Metric | Limit | Scope |
|--------|-------|-------|
| Function NLOC | 50 | Per function/method |
| File NLOC | 500 | Per source file |
| Cyclomatic complexity | 8 | Per function/method |

### Specific Rules

- **No `unwrap()`** in production code (only after validation)
- **Error handling** with `thiserror` only (not `anyhow`)
- **Documentation** required on all public API items (`///`)
- **Unsafe code** must include a `// SAFETY:` comment explaining the invariant
- **TODO comments** must carry an issue tag -- `// TODO(EPIC-XXX): ...`, `// TODO(US-XXX): ...`, `// TODO: ... #123`, or `// TODO: ... #issue` -- bare `TODO`/`FIXME`/`HACK` are rejected by CI (`scripts/check-todo-annotations.py`)
- **Numeric casts**: Use `try_from` for `u64`-to-`usize` casts, never `as usize` (clippy::pedantic)

---

## Security

### Automated Audit

```bash
cargo audit
cargo deny check
```

### Rules

- No `unsafe` without a documented `// SAFETY:` comment
- Validate all user input
- No secrets in code

---

## Performance

### Benchmarks

Run benchmarks with explicit features:

```bash
cargo bench -p velesdb-core --bench simd_benchmark -- --noplot
```

### Principles

- **Measure before optimizing**
- Use `criterion` for benchmarks
- Profile with `cargo flamegraph`
- Performance regression baseline is at `benchmarks/baseline.json`

---

## Release

### Semantic Versioning

| Type | When |
|------|------|
| MAJOR | Breaking API or on-disk format change |
| MINOR | New backward-compatible feature |
| PATCH | Bug fix |

### Command

```bash
./scripts/release.sh patch|minor|major
```

---

## Pre-commit Checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --features persistence,gpu,update-check --exclude velesdb-python -- -D warnings -D clippy::pedantic`
- [ ] `cargo test --workspace --features persistence,gpu,update-check --exclude velesdb-python -- --test-threads=1`
- [ ] Documentation up to date
- [ ] No secrets in code
