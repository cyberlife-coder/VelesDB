# Crash Recovery Testing Guide

This document describes the crash recovery test harness for `VelesDB`, which validates that the database survives abrupt shutdowns without data corruption.

## Overview

The crash recovery harness consists of:

```
┌─────────────────────────────────────────────────────────────────┐
│                    Crash Recovery Harness                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐       │
│  │ crash_driver │───▶│  Collection  │───▶│   Storage    │       │
│  │   (binary)   │    │  (VelesDB)   │    │  (WAL+Mmap)  │       │
│  └──────────────┘    └──────────────┘    └──────────────┘       │
│         │                                       │                │
│         │ SIGKILL                               │                │
│         ▼                                       ▼                │
│  ┌──────────────┐                      ┌──────────────┐         │
│  │ crash_test   │                      │  Disk Files  │         │
│  │   (script)   │                      │  (partial)   │         │
│  └──────────────┘                      └──────────────┘         │
│         │                                       │                │
│         │ Restart                               │                │
│         ▼                                       ▼                │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐       │
│  │  Validator   │◀───│  Collection  │◀───│  WAL Replay  │       │
│  │  (check)     │    │  (reopen)    │    │  (recovery)  │       │
│  └──────────────┘    └──────────────┘    └──────────────┘       │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Components

### 1. `CrashTestDriver` (Rust)

Deterministic test driver that performs operations with seed control for reproducibility.

**Location:** `tests/crash_recovery/driver.rs`

```rust
let config = DriverConfig {
    data_dir: PathBuf::from("./test_data"),
    seed: 42,           // Reproducible randomness
    count: 10000,       // Number of vectors
    dimension: 128,     // Vector dimension
    flush_interval: 100 // Flush every N ops
};

let driver = CrashTestDriver::new(config);
driver.run_insert()?;  // Insert with periodic flushes
```

### 2. `IntegrityValidator` (Rust)

Post-crash integrity verification that checks:
- Collection can be opened (WAL replay succeeds)
- Vector dimensions are correct
- No NaN/Inf values in vectors
- Checksums match stored values

**Location:** `tests/crash_recovery/validator.rs`

```rust
let validator = IntegrityValidator::new(data_dir);
let report = validator.validate()?;

assert!(report.is_valid);
assert!(report.vectors_consistent);
```

### 3. `crash_driver` (Binary)

CLI binary for external crash simulation.

**Location:** `examples/crash_driver.rs`

```bash
# Insert mode
cargo run --release --example crash_driver -- \
    --mode insert --seed 42 --count 10000 --data-dir ./test_data

# Check mode (integrity validation)
cargo run --release --example crash_driver -- \
    --mode check --seed 42 --data-dir ./test_data
```

### 4. PowerShell Scripts

**`scripts/crash_test.ps1`** - Single crash test:
```powershell
.\scripts\crash_test.ps1 -Seed 42 -Count 10000 -CrashAfterMs 5000
```

**`scripts/soak_crash_test.ps1`** - Multiple iterations:
```powershell
.\scripts\soak_crash_test.ps1 -Iterations 100 -Count 5000
```

## Running Tests

### Unit Tests (In-Process)

```bash
cargo test -p velesdb-core --test crash_recovery_tests
```

### External Crash Simulation

```powershell
# Single test with specific seed
.\scripts\crash_test.ps1 -Seed 12345

# Soak test (100 iterations with random seeds)
.\scripts\soak_crash_test.ps1 -Iterations 100
```

## Checksum Validation

Each inserted vector includes a checksum in its payload:

```rust
let checksum = compute_checksum(&vector);
let payload = json!({
    "id": i,
    "seed": seed,
    "checksum": checksum,
});
```

The validator recomputes checksums after recovery to detect bit-level corruption.

## Reproduction

If a test fails, use the seed to reproduce:

```powershell
# Failed with seed 12345
.\scripts\crash_test.ps1 -Seed 12345 -Count 10000 -CrashAfterMs 5000
```

## Architecture Notes

### WAL (Write-Ahead Log)

`VelesDB` uses a WAL for durability:
1. Operations are logged to WAL before applying
2. Periodic flushes sync WAL to disk
3. On recovery, WAL is replayed to restore state

### Snapshot System

For fast recovery, `LogPayloadStorage` supports snapshots:
- Snapshot captures index state at a point in time
- Recovery loads snapshot + replays WAL delta
- CRC32 checksum validates snapshot integrity

### HNSW Gap Detection (Issue #382)

After WAL replay and HNSW load, `Collection::open()` runs gap detection:
1. Compares `storage.ids()` vs `index.mappings.contains()` (lock-free)
2. Re-indexes gap vectors via `insert_batch_parallel()`
3. Skips vectors with mismatched dimensions (corruption defense)

This recovers vectors that were written to storage but not yet indexed
in HNSW due to a crash before a deferred-indexer or async-builder merge.

Unit tests in `collection/core/recovery_tests.rs` cover:
- No-gap fast path (O(1) count comparison)
- Simulated gap (direct storage write bypassing HNSW)
- Full reopen cycle (create → gap → drop → reopen → verify search)
- Metadata-only skip (no false positives)

### Batch Pipeline and the Gap Window (v1.7.2+)

The 3-phase upsert pipeline in `crud.rs` (`batch_store_all` -> `per_point_updates`
-> `bulk_index_or_defer`) enlarges the crash recovery window compared to the
previous per-point approach. Vectors and payloads are durably written in
Phase 1, but HNSW graph insertion happens only in Phase 3. A crash between
these phases leaves vectors in storage that are not yet present in the HNSW
index.

The HNSW gap detection mechanism described above recovers these vectors
automatically on the next `Collection::open()`. The gap window is bounded
by the batch size: at most one batch worth of vectors may need re-indexing
after a crash.

## Future Work (EPIC-024)

- **US-002**: Corruption tests (truncation, bitflip)
- **US-003**: Document WAL/pages format specification
