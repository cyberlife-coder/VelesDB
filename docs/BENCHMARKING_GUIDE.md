# 📊 VelesDB Benchmarking Guide

This guide explains how to run reproducible benchmarks for VelesDB.

## 🖥️ System Preparation

### Windows

1. **Power Plan**: Set to High Performance
   ```powershell
   powercfg /setactive 8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c
   ```

2. **Close background applications**:
   - Web browsers (especially Chrome/Edge with many tabs)
   - IDE indexing (VS Code, IntelliJ)
   - Windows Update
   - OneDrive sync

3. **Disable Windows Defender real-time scanning** (temporarily):
   ```powershell
   # Run as Administrator
   Set-MpPreference -DisableRealtimeMonitoring $true
   # Re-enable after benchmarking:
   # Set-MpPreference -DisableRealtimeMonitoring $false
   ```

### Linux

1. **CPU Governor**: Set to performance mode
   ```bash
   sudo cpupower frequency-set -g performance
   # Or for all cores:
   echo performance | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor
   ```

2. **Disable CPU frequency scaling**:
   ```bash
   sudo cpupower frequency-set -d 3.5GHz -u 3.5GHz  # Adjust to your CPU
   ```

3. **Pin process to specific cores** (optional):
   ```bash
   taskset -c 0-3 cargo bench ...
   ```

---

## 🏃 Running Benchmarks

### Basic Commands

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench --bench simd_benchmark

# Run with specific filter
cargo bench --bench overhead_benchmark -- cosine

# Skip plot generation (faster)
cargo bench -- --noplot
```

### Best Practices

1. **Always use release mode** (automatic with `cargo bench`)

2. **Run benchmarks sequentially**, not in parallel:
   ```bash
   # ✅ Good: Sequential
   cargo bench --bench simd_benchmark
   cargo bench --bench filter_benchmark
   
   # ❌ Bad: Parallel (causes resource contention)
   cargo bench --bench simd_benchmark &
   cargo bench --bench filter_benchmark &
   ```

3. **Let the system warm up** - Criterion handles warm-up automatically

4. **Run 3 times and take median** for important measurements

5. **Close resource-intensive apps** before benchmarking

---

## ⚙️ Criterion Configuration

### Default Configuration

VelesDB benchmarks use Criterion with these defaults:
- Sample size: 100 iterations
- Warm-up time: 3 seconds
- Measurement time: 5 seconds

### Custom Configuration

```rust
use criterion::{criterion_group, Criterion};

fn bench_with_config(c: &mut Criterion) {
    let mut group = c.benchmark_group("my_benchmark");
    group.sample_size(200);           // More samples for stability
    group.warm_up_time(Duration::from_secs(5));
    group.measurement_time(Duration::from_secs(10));
    
    // ... benchmarks ...
    
    group.finish();
}
```

---

## 📈 Interpreting Results

### Understanding Output

```
cosine_similarity/768d    time:   [76.12 ns 76.36 ns 76.64 ns]
                          change: [-76.37% -75.89% -75.42%] (p = 0.00 < 0.05)
                          Performance has improved.
```

- **[76.12 ns 76.36 ns 76.64 ns]**: Lower bound, estimate, upper bound
- **change**: Comparison to previous run (if available)
- **p value**: Statistical significance (< 0.05 = significant)

### Acceptable Variance

| Metric | Acceptable | Warning |
|--------|------------|---------|
| Coefficient of variation | < 5% | > 10% |
| Outliers | < 5% | > 15% |

### Outliers

Criterion reports outliers:
- **mild**: Slightly outside expected range
- **severe**: Significantly outside expected range

A few outliers (< 5%) are normal. More indicates system instability.

---

## 🔧 Available Benchmarks

| Benchmark | Description | Command |
|-----------|-------------|---------|
| `simd_benchmark` | SIMD kernel comparison | `cargo bench --bench simd_benchmark` |
| `overhead_benchmark` | API overhead analysis | `cargo bench --bench overhead_benchmark` |
| `filter_benchmark` | Metadata filtering | `cargo bench --bench filter_benchmark` |
| `column_filter_benchmark` | Column Store vs JSON | `cargo bench --bench column_filter_benchmark` |
| `search_benchmark` | Distance functions | `cargo bench --bench search_benchmark` |
| `hnsw_benchmark` | HNSW index operations | `cargo bench --bench hnsw_benchmark` |
| `recall_benchmark` | Search quality metrics (Recall@k) | `cargo bench --bench recall_benchmark` |
| `velesql_benchmark` | VelesQL parsing | `cargo bench --bench velesql_benchmark` |

---

## 📋 Benchmark Checklist

Before running benchmarks:

- [ ] Close browser and heavy applications
- [ ] Set power plan to High Performance
- [ ] Check CPU temperature (avoid thermal throttling)
- [ ] Ensure no background updates running
- [ ] Use `--release` mode (automatic with `cargo bench`)

After running benchmarks:

- [ ] Check for excessive outliers
- [ ] Verify coefficient of variation < 5%
- [ ] Compare with baseline if available
- [ ] Document results in `docs/BENCHMARKS.md`

---

## 🐛 Troubleshooting

### High Variance

**Symptoms**: Large confidence intervals, many outliers

**Solutions**:
1. Close background applications
2. Disable CPU frequency scaling
3. Increase sample size
4. Run on dedicated hardware

### Inconsistent Results

**Symptoms**: Results vary significantly between runs

**Solutions**:
1. Wait for system to stabilize after boot
2. Check for thermal throttling
3. Pin to specific CPU cores
4. Use `--noplot` to reduce I/O

### Slow Compilation

**Solutions**:
```bash
# Use incremental compilation
export CARGO_INCREMENTAL=1

# Use faster linker (Linux)
# Add to ~/.cargo/config.toml:
# [target.x86_64-unknown-linux-gnu]
# linker = "clang"
# rustflags = ["-C", "link-arg=-fuse-ld=lld"]
```

---

## 📚 References

- [Criterion.rs Documentation](https://bheisler.github.io/criterion.rs/book/)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [BENCHMARKS.md](./BENCHMARKS.md) - Current benchmark results
- [PERFORMANCE_ROADMAP.md](./PERFORMANCE_ROADMAP.md) - Optimization plan
