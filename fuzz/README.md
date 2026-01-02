# VelesDB Fuzzing

Security fuzzing targets for VelesDB using [cargo-fuzz](https://rust-fuzz.github.io/book/cargo-fuzz.html).

## Prerequisites

```bash
# Install cargo-fuzz (requires nightly)
cargo install cargo-fuzz

# Or use rustup
rustup install nightly
```

## Fuzz Targets

### `fuzz_velesql_parser`

Tests the VelesQL SQL parser with arbitrary input strings to find:
- Panics on malformed queries
- Memory safety issues in pest parsing
- Stack overflows from deeply nested expressions

```bash
cd fuzz
cargo +nightly fuzz run fuzz_velesql_parser
```

### `fuzz_distance_metrics`

Tests SIMD distance calculations with arbitrary vectors to find:
- Panics on edge cases (NaN, Inf, denormals)
- Numerical stability issues
- SIMD alignment problems

```bash
cd fuzz
cargo +nightly fuzz run fuzz_distance_metrics
```

## Running Fuzzing

### Quick Run (1 minute)

```bash
cd fuzz
cargo +nightly fuzz run fuzz_velesql_parser -- -max_total_time=60
```

### Long Run (1 hour)

```bash
cd fuzz
cargo +nightly fuzz run fuzz_velesql_parser -- -max_total_time=3600
```

### Check Coverage

```bash
cargo +nightly fuzz coverage fuzz_velesql_parser
```

## Reproducing Crashes

If a crash is found, it will be saved in `fuzz/artifacts/`. Reproduce with:

```bash
cargo +nightly fuzz run fuzz_velesql_parser fuzz/artifacts/fuzz_velesql_parser/<crash_file>
```

## CI Integration

Add to GitHub Actions:

```yaml
- name: Fuzz Test (Quick)
  run: |
    cargo install cargo-fuzz
    cd fuzz
    cargo +nightly fuzz run fuzz_velesql_parser -- -max_total_time=60
```

## Adding New Targets

1. Create `fuzz/fuzz_targets/fuzz_<name>.rs`
2. Add `[[bin]]` entry to `fuzz/Cargo.toml`
3. Document in this README
