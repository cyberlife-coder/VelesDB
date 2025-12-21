# 📊 VelesDB Performance Benchmarks

*Last updated: December 2025*

This document details the performance benchmarks for VelesDB v0.1.1. Tests were conducted on a standard workstation (8-core CPU, AVX2 support).

## 🚀 Summary

| Operation | Metric | Time (768d) | Throughput | Speedup vs Baseline |
|-----------|--------|-------------|------------|---------------------|
| **Cosine Similarity** | Latency | **~325 ns** | ~3.0M ops/sec | **2.5x** |
| **Euclidean Distance** | Latency | **~135 ns** | ~7.4M ops/sec | **2.1x** |
| **Dot Product** | Latency | **~140 ns** | ~7.1M ops/sec | **2.0x** |
| **Hamming (Binary)** | Latency | **~13 ns** | ~76.9M ops/sec | **>50x** (vs f32) |
| **Metadata Filter** | Equality | **~18 µs / 1k** | ~55M items/sec | - |
| **VelesQL Parser** | Simple | **~750 ns** | ~1.3M qps | - |

---

## ⚡ SIMD Vector Operations

Comparison between standard Rust iterators (Baseline) and VelesDB's explicit SIMD optimizations (Optimized).

### Cosine Similarity (768 dimensions)
> Used for semantic search and text embeddings.

| Implementation | Time per op | Throughput |
|----------------|-------------|------------|
| Baseline (Auto-vec) | 809 ns | 1.2M ops/s |
| **VelesDB Optimized** | **324 ns** | **3.0M ops/s** |
| **Improvement** | **-60% latency** | **2.5x throughput** |

### Euclidean Distance (768 dimensions)
> Used for spatial data and image features.

| Implementation | Time per op | Throughput |
|----------------|-------------|------------|
| Baseline (Auto-vec) | 286 ns | 3.5M ops/s |
| **VelesDB Optimized** | **134 ns** | **7.4M ops/s** |
| **Improvement** | **-53% latency** | **2.1x throughput** |

### Binary Hamming Distance (768 bits / 12 u64)
> Used for binary fingerprints and image hashing.

| Implementation | Time per op | Throughput |
|----------------|-------------|------------|
| Float32 Baseline | ~800 ns | 1.25M ops/s |
| **VelesDB Optimized** | **13 ns** | **76.9M ops/s** |
| **Improvement** | **-98% latency** | **~60x throughput** |

---

## 🔍 Metadata Filtering

Benchmarks for filtering 10,000 items with various conditions.

| Filter Type | Condition | Time (10k items) | Throughput |
|-------------|-----------|------------------|------------|
| **Equality** | `category = 'tech'` | ~180 µs | 55M items/s |
| **In List** | `status IN ('a', 'b')` | ~211 µs | 47M items/s |
| **Range** | `price > 100` | ~490 µs | 20M items/s |
| **Complex** | `(A AND B) OR C` | ~672 µs | 15M items/s |

---

## 📝 VelesQL Parsing

Performance of the SQL-like query parser.

| Query Type | Complexity | Time | Throughput |
|------------|------------|------|------------|
| **Simple** | `SELECT * FROM table` | 748 ns | 1.3M qps |
| **Vector** | `... WHERE vector NEAR $v` | 1.05 µs | 950k qps |
| **Complex** | Multiple conditions | 5.76 µs | 175k qps |

---

## 🧪 Methodology

- **Hardware**: Windows Workstation, 8-core CPU
- **Environment**: Rust 1.83, Release build (`--release`)
- **Framework**: Criterion.rs
- **Optimizations**: AVX2 enabled, `target-cpu=native`
