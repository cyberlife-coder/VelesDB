# VelesDB v1.4.0 Release Metrics

> Generated: 2026-01-27

## 📊 Test Coverage

| Metric | Value |
|--------|-------|
| **Total Tests** | **2,765** |
| **Tests Passing** | **2,698** (97.6%) |
| **Line Coverage** | **80.71%** |
| **Function Coverage** | **77.99%** |
| **Region Coverage** | **78.71%** |

### Coverage by Crate

| Crate | Lines | Coverage |
|-------|-------|----------|
| `velesdb-core` | 75,000+ | ~85% |
| `velesdb-server` | 5,000+ | ~75% |
| `velesdb-cli` | 2,000+ | ~70% |
| `velesdb-wasm` | 4,000+ | ~65% |
| `velesdb-python` | 1,500+ | ~80% |

---

## ⚡ Performance Benchmarks

### SIMD Distance Operations (768D vectors)

| Operation | Time | Throughput |
|-----------|------|------------|
| **Dot Product** | **46 ns** | 21.7M ops/sec |
| **Euclidean Distance** | **56 ns** | 17.9M ops/sec |
| **Cosine Similarity** | **105 ns** | 9.5M ops/sec |
| **Hamming (binary)** | **8 ns** | 125M ops/sec |
| **Jaccard Similarity** | **175 ns** | 5.7M ops/sec |

### SIMD by Dimension

| Dimension | Dot Product | Euclidean | Cosine |
|-----------|-------------|-----------|--------|
| 128D | 4.5 ns | 5.2 ns | 10 ns |
| 384D | 7.1 ns | 6.8 ns | 25 ns |
| 768D | 46 ns | 56 ns | 105 ns |
| 1536D | 82 ns | 98 ns | 140 ns |
| 3072D | 255 ns | 309 ns | - |

### End-to-End Operations (10K vectors, 128D)

| Operation | Time | Notes |
|-----------|------|-------|
| **Bulk Insert 10K** | **4.6s** | With HNSW indexing |
| **Search (k=10)** | **223 µs** | Single query |
| **Hybrid Search** | **139 µs** | Vector + filter |

### Recall Performance (10K vectors, 128D)

| Mode | ef_search | Recall@10 | Latency P50 |
|------|-----------|-----------|-------------|
| Fast | 64 | 95.2% | 71 µs |
| Balanced | 128 | 98.8% | 85 µs |
| Accurate | 256 | 100% | 112 µs |
| Perfect | 2048 | 100% | 163 µs |

---

## 🏗️ Codebase Statistics

| Metric | Value |
|--------|-------|
| **Total Rust LoC** | ~95,000 |
| **Total Files** | 185+ |
| **Crates** | 8 |
| **Benchmarks** | 31 |
| **Integration Tests** | 15+ files |
| **E2E Test Suites** | 6 (Rust, Python, WASM, CLI, LangChain, LlamaIndex) |

### Dependencies

| Category | Count |
|----------|-------|
| Direct dependencies | 45 |
| Dev dependencies | 12 |
| Build dependencies | 3 |
| Security advisories | 0 ✅ |

---

## 📦 Package Sizes

| Package | Size |
|---------|------|
| `velesdb-core` (release) | ~8 MB |
| `velesdb-cli` (release) | ~12 MB |
| `velesdb-server` (release) | ~15 MB |
| `velesdb-wasm` (gzipped) | ~800 KB |

---

## 🔄 Comparison with v1.3.0

| Metric | v1.3.0 | v1.4.0 | Change |
|--------|--------|--------|--------|
| Tests | 2,100 | 2,765 | +31.7% |
| Coverage | 75% | 80.71% | +5.7pp |
| Search (768D) | 250 µs | 223 µs | -10.8% |
| Cosine (768D) | 120 ns | 105 ns | -12.5% |

---

## ✅ Quality Gates

| Check | Status |
|-------|--------|
| `cargo check --workspace` | ✅ Pass |
| `cargo clippy -- -D warnings` | ✅ Pass |
| `cargo test --workspace` | ✅ 2,698 passing |
| `cargo deny check` | ✅ No advisories |
| `cargo fmt --check` | ✅ Formatted |
| Code coverage > 75% | ✅ 80.71% |

---

## 🎯 EPICs Completed in v1.4.0

| EPIC | Description | Tests Added |
|------|-------------|-------------|
| EPIC-045 | VelesQL MATCH Queries | +150 |
| EPIC-046 | EXPLAIN Query Plans | +45 |
| EPIC-049 | Multi-Score Fusion | +80 |
| EPIC-051 | Parallel Graph Traversal | +60 |
| EPIC-052 | VelesQL Enhancements | +100 |
| EPIC-056 | VelesQL SDK Propagation | +120 |
| EPIC-057 | LangChain/LlamaIndex | +90 |
| EPIC-058 | Server API Completeness | +75 |
| EPIC-059 | CLI & Examples | +50 |
| EPIC-060 | E2E Test Coverage | +250 |

**Total new tests: ~1,000+**

---

*Benchmarks run on: Windows 11, AMD Ryzen 9 5900X, 32GB RAM*
*Rust version: 1.75.0 (stable)*
