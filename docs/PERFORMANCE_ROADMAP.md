# 🚀 VelesDB Performance Optimization Roadmap

*Created: December 2025*  
*Last Updated: January 2026*

## 📊 Current State (After Optimizations)

Based on benchmark runs (January 2026) with implemented Technical Stories:

| Operation | Latency | Throughput | Status |
|-----------|---------|------------|--------|
| Dot Product (768D) | **~38 ns** | 26M ops/s | ✅ Optimized |
| Euclidean Distance (768D) | **~47 ns** | 21M ops/s | ✅ Optimized |
| Cosine Similarity (768D) | **~83 ns** | 12M ops/s | ✅ Optimized |
| Hamming Distance (768D) | **~16 ns** | 62M ops/s | ✅ Optimal |
| Jaccard Similarity (768D) | **~90 ns** | 11M ops/s | ✅ New |
| VelesQL Parse (simple) | ~570 ns | 1.8M qps | ✅ OK |
| ColumnStore Filter (100k) | ~42 µs | 122x vs JSON | ✅ Optimized |

### ✅ Implemented Technical Stories (P0)

| Story | Description | Status |
|-------|-------------|--------|
| **TS-CORE-001** | Adaptive prefetch distance (4-16 based on vector size) | ✅ Done |
| **TS-CORE-002** | Batch search lock optimization (N→1 contention) | ✅ Done |
| **TS-CORE-004** | Storage compaction with atomic swap | ✅ Done |

### Key Improvements vs December 2025

1. **Cosine**: 310ns → **83ns** (3.7x faster)
2. **Euclidean**: 138ns → **47ns** (2.9x faster)  
3. **Dot Product**: 130ns → **38ns** (3.4x faster)
4. **ColumnStore Filtering**: 122x faster than JSON at 100k items

---

## 🎯 Optimization Priorities

### Phase A: Diagnostic

**Goal**: Identify and measure the exact sources of overhead

- Create `benches/overhead_benchmark.rs`
- Measure: assertions, dispatch, alignment, inlining
- **Target**: Identify top 3 overhead sources

### Phase B: High-Performance Filtering

**Goal**: Maintain 50M+ items/s at 100k scale

**Approach**: Column Store for frequently filtered fields

```
Current (JSON):
  serde_json::Value → pointer chasing → allocations → slow

Proposed (Column Store):
  Vec<i64> / Vec<f64> / StringTable → cache-friendly → fast
```

**Implementation**:
1. `src/column_store.rs` with typed columns
2. Auto-extract indexed fields at upsert
3. Fallback to JSON for non-indexed fields

**Expected Gain**: 3x+ throughput at scale

### Phase C: SIMD Tuning

**Goal**: +10-20% gains, no regressions

**Approach**: Adaptive dispatch based on vector size

```rust
match len {
    0..32   => scalar_simple(),
    32..128 => unroll_4x(),
    _       => simd_8x(),
}
```

**Testing Matrix**: 64, 128, 256, 384, 512, 768, 1024, 1536 dimensions

### Phase D: Documentation

**Goal**: Reproducible benchmarks

- `docs/BENCHMARKING_GUIDE.md`
- Windows/Linux setup instructions
- Criterion configuration

---

## 📈 Success Metrics (Updated January 2026)

| Metric | Dec 2025 | Target | Jan 2026 | Status |
|--------|----------|--------|----------|--------|
| Cosine (768D) | 310 ns | <220 ns | **83 ns** | ✅ Exceeded |
| Euclidean (768D) | 138 ns | <100 ns | **47 ns** | ✅ Exceeded |
| Dot Product (768D) | 130 ns | <100 ns | **38 ns** | ✅ Exceeded |
| Filter (100k) | 19M/s | 50M/s | **122x faster** | ✅ Exceeded |
| Recall@10 | ~95% | >98% | **99.4%** | ✅ Achieved |

---

## 🗓️ Timeline Status

| Phase | Issue | Priority | Status |
|-------|-------|----------|--------|
| A | Diagnostic | 🔴 High | ✅ Complete |
| B | ColumnStore Filtering | 🔴 High | ✅ Complete (122x faster) |
| C | SIMD Optimization | 🟡 Medium | ✅ Complete (3-4x faster) |
| D | Documentation | 🟢 Low | 🔄 In Progress |

## 🔮 Future Optimizations (P1-P3)

| Story | Priority | Description |
|-------|----------|-------------|
| TS-CORE-003 | P1 | AVX-512 native exploitation (currently AVX2 via `wide`) |
| TS-CORE-005 | P2 | Product Quantization (PQ) for memory reduction |
| TS-SERVER-001 | P1 | Tokio runtime tuning |
| TS-WASM-001 | P2 | Binary size reduction (<500KB) |

---

## 📚 References

- [BENCHMARKS.md](./BENCHMARKS.md) - Detailed benchmark results
- `crates/velesdb-core/src/simd.rs` - Current optimized functions
- `crates/velesdb-core/src/simd_explicit.rs` - Explicit SIMD kernels
- `crates/velesdb-core/src/filter.rs` - Filtering module
- Apache Arrow / DataFusion - Column store inspiration
