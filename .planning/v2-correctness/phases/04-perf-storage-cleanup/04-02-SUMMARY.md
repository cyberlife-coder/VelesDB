---
phase: 4
plan: 2
completed: 2026-02-08
duration: ~25min
---

# Phase 4 Plan 2: Storage Integrity — Summary

## One-liner

WAL per-entry CRC32 corruption detection, batch flush for reduced I/O syscalls, and lock-free AtomicU64 snapshot decisions.

## What Was Built

Added CRC32 checksums to every WAL store entry, enabling corruption detection both during WAL replay (startup) and on-demand retrieval. The WAL format changed from `[marker][id][len][payload]` to `[marker][id][len][crc32][payload]`. Any bit-flip or partial write is now detected with a clear `InvalidData` error including the offset and expected/actual CRC values.

Added `store_batch()` method to `LogPayloadStorage` that writes N entries with a single `flush()` call, reducing I/O syscalls from N to 1 for bulk insertions. Each batch entry is CRC32-protected identically to single entries.

Replaced the lock-based `should_create_snapshot()` implementation (which acquired a write lock on the WAL just to read file position) with an `AtomicU64` that tracks WAL position lock-free. Position is updated on every `store()`, `delete()`, and `store_batch()` call using `Ordering::Release/Acquire`.

## Tasks Completed

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | WAL per-entry CRC32 [D-05] | a89b7f84 | log_payload.rs, log_payload_tests.rs, wal_recovery_tests.rs |
| 2 | LogPayloadStorage batch flush [D-06] | a89b7f84 | log_payload.rs, log_payload_tests.rs |
| 3 | Snapshot lock → AtomicU64 [D-07] | a89b7f84 | log_payload.rs |

## Key Files

**Modified:**
- `crates/velesdb-core/src/storage/log_payload.rs` — CRC32 in store/retrieve/replay, store_batch(), AtomicU64 wal_position
- `crates/velesdb-core/src/storage/log_payload_tests.rs` — 11 new tests (7 CRC32 + 6 batch flush)
- `crates/velesdb-core/src/storage/wal_recovery_tests.rs` — Updated helpers for new CRC32 format, fixed 2 tests

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Reuse existing `crc32_hash()` instead of adding `crc32fast` crate | Avoids new dependency; same IEEE 802.3 polynomial; performance difference negligible for payload sizes |
| CRC32 verified during replay (reads full payload) | Correctness over performance — detect corruption at startup, not just on retrieve |
| All 3 tasks in single commit | All modify same file, interdependent (batch uses CRC, atomic tracks writes from both store methods) |
| WAL format is a breaking change | Acceptable for v2-core-trust milestone; old WALs without CRC field incompatible |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug fix] WAL recovery tests using old format**
- Found during: Task 1
- Issue: `write_store_entry()` helper and 2 raw-WAL tests didn't include CRC32 field
- Fix: Updated helper to write new format; updated `test_wal_recovery_zero_length_payload` (add CRC) and `test_wal_recovery_flipped_bits_in_payload` (CRC now detects at replay, not retrieve)
- Files: `wal_recovery_tests.rs`
- Commit: a89b7f84

## Verification Results

```
cargo fmt --all --check          → ✅ Pass
cargo clippy -- -D warnings      → ✅ Pass (0 warnings)
cargo deny check                 → ✅ advisories ok, bans ok, licenses ok, sources ok
cargo test --workspace            → ✅ 2,491+ tests, 0 failures
  - log_payload_tests: 32 passed (was 21, +11 new)
  - wal_recovery_tests: 27 passed (all passing)
```

## Next Phase Readiness

- Plan 04-03 (ColumnStore Unification & Dead Code) is next — final plan in Phase 4
- All storage integrity improvements are in place for 04-03 to build on

---
*Completed: 2026-02-08T16:25+01:00*
