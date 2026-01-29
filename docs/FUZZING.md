# VelesDB Fuzzing Guide

> **EPIC-025**: Comprehensive fuzzing documentation for VelesDB security testing.

## Overview

VelesDB uses [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer) for continuous fuzzing of critical components. This document covers setup, running fuzzers, reproducing crashes, and contributing to fuzzing efforts.

## Setup

### Prerequisites

```bash
# Install Rust nightly (required for cargo-fuzz)
rustup install nightly

# Install cargo-fuzz
cargo install cargo-fuzz
```

### Verify Installation

```bash
cd fuzz
cargo +nightly fuzz list
```

Expected output:
```
fuzz_velesql_parser
fuzz_distance_metrics
fuzz_snapshot_parser
```

## Fuzz Targets

| Target | Component | Invariants | Priority |
|--------|-----------|------------|----------|
| `fuzz_velesql_parser` | VelesQL Parser | No panic, valid AST or error | 🔥 HIGH |
| `fuzz_distance_metrics` | Distance Calculations | No NaN/panic, bounded output | 🔥 HIGH |
| `fuzz_snapshot_parser` | Snapshot Decoder | Roundtrip, no corruption | 🚀 MEDIUM |

### fuzz_velesql_parser

Tests the VelesQL SQL parser with arbitrary input strings:
- **Input**: Arbitrary bytes (converted to UTF-8)
- **Invariant**: Parser must never panic; returns `Ok(AST)` or `Err(ParseError)`
- **Coverage**: SELECT, MATCH, JOIN, ORDER BY, WHERE clauses

```bash
cargo +nightly fuzz run fuzz_velesql_parser
```

### fuzz_distance_metrics

Tests SIMD distance calculations with arbitrary vectors:
- **Input**: Pairs of f32 vectors (arbitrary length and values)
- **Invariant**: Must handle NaN, Inf, denormals without panic
- **Coverage**: Cosine, Euclidean, DotProduct, Hamming, Jaccard

```bash
cargo +nightly fuzz run fuzz_distance_metrics
```

### fuzz_snapshot_parser

Tests snapshot serialization/deserialization:
- **Input**: Arbitrary bytes
- **Invariant**: Decode → Encode → Decode must roundtrip (if decode succeeds)
- **Coverage**: Collection snapshots, metadata, vector data

```bash
cargo +nightly fuzz run fuzz_snapshot_parser
```

## Running Fuzzers

### Quick Run (1 minute)

```bash
cd fuzz
cargo +nightly fuzz run fuzz_velesql_parser -- -max_total_time=60
```

### Long Run (1 hour)

```bash
cargo +nightly fuzz run fuzz_velesql_parser -- -max_total_time=3600
```

### Parallel Fuzzing (Multiple Cores)

```bash
# Run with 4 workers
cargo +nightly fuzz run fuzz_velesql_parser -- -workers=4 -jobs=4
```

### With Specific Corpus

```bash
cargo +nightly fuzz run fuzz_velesql_parser fuzz/corpus/velesql_parser/
```

## Reproducing Crashes

When a crash is found, a crash file is saved to `fuzz/artifacts/<target>/`.

### View Crash Input

```bash
xxd fuzz/artifacts/fuzz_velesql_parser/crash-xxx
```

### Reproduce Crash

```bash
cargo +nightly fuzz run fuzz_velesql_parser fuzz/artifacts/fuzz_velesql_parser/crash-xxx
```

### Minimize Crash

Minimize the crash input to find the smallest reproducer:

```bash
cargo +nightly fuzz tmin fuzz_velesql_parser fuzz/artifacts/fuzz_velesql_parser/crash-xxx
```

### Debug Crash

```bash
# With debug symbols
RUST_BACKTRACE=1 cargo +nightly fuzz run fuzz_velesql_parser fuzz/artifacts/fuzz_velesql_parser/crash-xxx
```

## Corpus Management

### Versioned Corpus

The corpus directory contains seed inputs that guide fuzzing:

```
fuzz/
├── corpus/
│   ├── velesql_parser/     # SQL query seeds
│   │   ├── valid_select_01
│   │   ├── valid_match_01
│   │   └── edge_case_empty
│   ├── distance_metrics/   # Vector pair seeds
│   └── snapshot_parser/    # Binary snapshot seeds
```

### Adding Seeds

Add interesting inputs that exercise different code paths:

```bash
# Add a valid query
echo 'SELECT * FROM docs WHERE vector NEAR $v LIMIT 10' > fuzz/corpus/velesql_parser/valid_near_query

# Add edge case
echo '' > fuzz/corpus/velesql_parser/empty_input
```

### Minimizing Corpus

Remove redundant inputs:

```bash
cargo +nightly fuzz cmin fuzz_velesql_parser fuzz/corpus/velesql_parser/
```

## CI Integration

### GitHub Actions (Optional Nightly Run)

```yaml
fuzz:
  name: Fuzzing (Nightly)
  runs-on: ubuntu-latest
  if: github.event_name == 'schedule'
  steps:
    - uses: actions/checkout@v4
    
    - name: Install cargo-fuzz
      run: cargo install cargo-fuzz
    
    - name: Run VelesQL parser fuzzer
      run: |
        cd fuzz
        cargo +nightly fuzz run fuzz_velesql_parser -- -max_total_time=300
    
    - name: Upload crashes
      if: failure()
      uses: actions/upload-artifact@v4
      with:
        name: fuzz-crashes
        path: fuzz/artifacts/
```

### Local Pre-Push Check

```bash
# Quick fuzz check before push
cd fuzz
for target in $(cargo +nightly fuzz list); do
  cargo +nightly fuzz run $target -- -max_total_time=30 || exit 1
done
```

## Writing New Fuzz Targets

### 1. Create Fuzz Target

```rust
// fuzz/fuzz_targets/fuzz_new_component.rs
#![no_main]

use libfuzzer_sys::fuzz_target;
use arbitrary::Arbitrary;

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    data: Vec<u8>,
    option: bool,
}

fuzz_target!(|input: FuzzInput| {
    // Your component under test
    // MUST NOT panic on any input
    let _ = your_component::process(&input.data, input.option);
});
```

### 2. Add to Cargo.toml

```toml
[[bin]]
name = "fuzz_new_component"
path = "fuzz_targets/fuzz_new_component.rs"
test = false
doc = false
bench = false
```

### 3. Create Initial Corpus

```bash
mkdir -p fuzz/corpus/new_component
echo 'valid input' > fuzz/corpus/new_component/seed_01
```

### 4. Document in this Guide

Add entry to the Fuzz Targets table above.

## Best Practices

### For Fuzz Targets

1. **No panics**: Handle all errors gracefully with `Result`
2. **Bounded resources**: Use timeouts and size limits
3. **Deterministic**: Same input → same behavior
4. **Fast**: Each iteration should be < 1ms

### For Corpus Seeds

1. **Diverse**: Cover different code paths
2. **Minimal**: Smallest input that exercises a path
3. **Valid + Invalid**: Include both correct and malformed inputs
4. **Edge cases**: Empty, max size, special characters

### For CI Integration

1. **Time-boxed**: Always use `-max_total_time`
2. **Artifact upload**: Preserve crashes for debugging
3. **Non-blocking**: Use `continue-on-error: true` initially

## Reporting Issues

When reporting a fuzzing crash:

1. **Minimize** the crash file
2. **Check** if it's a known issue
3. **Open issue** with:
   - Crash file (base64 encoded)
   - VelesDB version (`cargo pkgid velesdb-core`)
   - Reproduction command
   - Stack trace if available

### Template

```markdown
## Fuzzing Crash Report

**Target**: fuzz_velesql_parser
**Version**: velesdb-core 1.4.0
**Date**: 2026-01-29

### Reproduction

```bash
cargo +nightly fuzz run fuzz_velesql_parser crash_input.txt
```

### Crash Input (base64)

```
<base64 encoded crash file>
```

### Stack Trace

```
thread 'main' panicked at ...
```
```

## References

- [cargo-fuzz book](https://rust-fuzz.github.io/book/)
- [libFuzzer documentation](https://llvm.org/docs/LibFuzzer.html)
- [Rust Fuzz Book](https://rust-fuzz.github.io/book/)
- [AFL.rs](https://github.com/rust-fuzz/afl.rs) (alternative fuzzer)

---

*Last updated: 2026-01-29 (EPIC-025)*
