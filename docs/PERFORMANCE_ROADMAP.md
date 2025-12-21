# 🚀 VelesDB Performance Optimization Roadmap

*Created: December 2025*
*Linear EPIC: [WIS-44](https://linear.app/tui-france/issue/WIS-44)*

## 📊 Current State (Baseline)

Based on sequential benchmark runs (December 2025):

| Operation | Latency | Throughput | Gap vs Optimal |
|-----------|---------|------------|----------------|
| Cosine Similarity (768d) | ~310 ns | 3.2M ops/s | ~3.5x vs kernel |
| Euclidean Distance (768d) | ~138 ns | 7.2M ops/s | ~2x vs kernel |
| Dot Product (768d) | ~130 ns | 7.7M ops/s | ~2x vs kernel |
| Hamming Binary (768b) | ~6 ns | 164M ops/s | ✅ Optimal |
| Metadata Filter (100k) | ~5.2 ms | 19M items/s | -70% vs 10k |
| VelesQL Parse (simple) | ~528 ns | 1.9M qps | ✅ OK |

### Key Observations

1. **SIMD Kernel vs API Gap**: Raw SIMD kernels (`simd_explicit.rs`) are 2-3.5x faster than the public API (`simd.rs`)
2. **Filtering Scale Issue**: Throughput drops from 65M/s at 10k items to 19M/s at 100k items
3. **Workload Analysis**: Filtering is **frequent (50%+)** for target RAG use cases

---

## 🎯 Optimization Priorities

### Phase A: Diagnostic (WIS-45)

**Goal**: Identify and measure the exact sources of overhead

- Create `benches/overhead_benchmark.rs`
- Measure: assertions, dispatch, alignment, inlining
- **Target**: Identify top 3 overhead sources

### Phase B: High-Performance Filtering (WIS-46)

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

### Phase C: SIMD Tuning (WIS-47)

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

### Phase D: Documentation (WIS-48)

**Goal**: Reproducible benchmarks

- `docs/BENCHMARKING_GUIDE.md`
- Windows/Linux setup instructions
- Criterion configuration

---

## 📈 Success Metrics

| Metric | Current | Target | Improvement |
|--------|---------|--------|-------------|
| Cosine (768d) | 310 ns | <220 ns | -30% |
| Euclidean (768d) | 138 ns | <100 ns | -30% |
| Filter (100k) | 19M/s | 50M/s | +160% |
| Filter (1M) | TBD | 40M/s | - |

---

## 🗓️ Timeline

| Phase | Issue | Priority | Estimated Effort |
|-------|-------|----------|------------------|
| A | WIS-45 | 🔴 High | 2-3 days |
| B | WIS-46 | 🔴 High | 5-7 days |
| C | WIS-47 | 🟡 Medium | 3-4 days |
| D | WIS-48 | 🟢 Low | 1 day |

**Total**: ~2-3 weeks

---

## 📚 References

- [BENCHMARKS.md](./BENCHMARKS.md) - Detailed benchmark results
- `crates/velesdb-core/src/simd.rs` - Current optimized functions
- `crates/velesdb-core/src/simd_explicit.rs` - Explicit SIMD kernels
- `crates/velesdb-core/src/filter.rs` - Filtering module
- Apache Arrow / DataFusion - Column store inspiration
