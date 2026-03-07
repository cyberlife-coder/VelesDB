---
phase: 07-streaming-inserts
plan: 01
subsystem: database
tags: [tokio, mpsc, streaming, backpressure, micro-batch]

# Dependency graph
requires:
  - phase: 06-cache
    provides: write_generation counter for cache invalidation
provides:
  - StreamIngester with bounded mpsc channel and micro-batch drain
  - StreamingConfig (buffer_size, batch_size, flush_interval_ms)
  - WriteMode enum (Api vs Streaming)
  - BackpressureError (BufferFull, NotConfigured)
  - DeltaBuffer stub for Plan 02
  - Collection struct wired with stream_ingester and delta_buffer fields
affects: [07-02, 07-03, velesdb-server]

# Tech tracking
tech-stack:
  added: [tokio::sync::mpsc, tokio::sync::Notify, tokio::time::interval]
  patterns: [bounded-channel backpressure, micro-batch drain loop, spawn_blocking for sync upsert]

key-files:
  created:
    - crates/velesdb-core/src/collection/streaming/mod.rs
    - crates/velesdb-core/src/collection/streaming/ingester.rs
    - crates/velesdb-core/src/collection/streaming/delta.rs
  modified:
    - crates/velesdb-core/src/collection/mod.rs
    - crates/velesdb-core/src/collection/types.rs
    - crates/velesdb-core/src/collection/core/lifecycle.rs

key-decisions:
  - "Option<JoinHandle> pattern for StreamIngester to support both graceful shutdown (take + await) and Drop abort"
  - "BackpressureError maps both Full and Closed channel states to BufferFull (closed = drain exited = functionally full)"
  - "std::mem::take instead of drain(..).collect() for batch handoff to spawn_blocking"
  - "allow(dead_code) on stream_ingester/delta_buffer fields and accessors (wired in Plan 02)"

patterns-established:
  - "Streaming drain pattern: tokio::select! with shutdown/timer/recv branches, flush via spawn_blocking(upsert)"
  - "cfg-gated streaming fields on Collection: #[cfg(feature = persistence)] for WASM exclusion"
  - "Lock order position 10 for delta_buffer (after sparse_indexes at 9)"

requirements-completed: [STREAM-01, STREAM-02, STREAM-05]

# Metrics
duration: 14min
completed: 2026-03-07
---

# Phase 7 Plan 1: Streaming Core Pipeline Summary

**StreamIngester with bounded mpsc channel, micro-batch drain via upsert(), backpressure signaling, and DeltaBuffer stub**

## Performance

- **Duration:** 14 min
- **Started:** 2026-03-07T13:44:24Z
- **Completed:** 2026-03-07T13:58:29Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments
- StreamIngester accepts points via bounded tokio mpsc, drains micro-batches through existing Collection::upsert() pipeline
- Backpressure signaled via BackpressureError::BufferFull when channel at capacity
- Drain loop flushes at batch_size count OR flush_interval_ms timeout (whichever first)
- DeltaBuffer stub with is_active() check ready for Plan 02 search integration
- Collection struct wired with stream_ingester and delta_buffer fields (all cfg-gated)

## Task Commits

Each task was committed atomically:

1. **Task 1: Create streaming module with StreamIngester, StreamingConfig, WriteMode, and DeltaBuffer stub** - `cfd17f5e` (feat)
2. **Task 2: Wire StreamIngester and DeltaBuffer into Collection struct** - `606c55e3` (feat)

## Files Created/Modified
- `crates/velesdb-core/src/collection/streaming/mod.rs` - Module root with cfg-gated re-exports
- `crates/velesdb-core/src/collection/streaming/ingester.rs` - StreamIngester, StreamingConfig, WriteMode, BackpressureError, drain_loop, flush_batch
- `crates/velesdb-core/src/collection/streaming/delta.rs` - DeltaBuffer stub with is_active()
- `crates/velesdb-core/src/collection/mod.rs` - Added `pub mod streaming`
- `crates/velesdb-core/src/collection/types.rs` - Added stream_ingester, delta_buffer fields and accessors
- `crates/velesdb-core/src/collection/core/lifecycle.rs` - Initialized new fields in all 4 construction sites

## Decisions Made
- Used `Option<JoinHandle>` pattern so StreamIngester can support both graceful `shutdown()` (take + await) and `Drop` abort without conflicting with Rust ownership rules
- BackpressureError maps both Full and Closed channel states to BufferFull (a closed channel means drain task exited, functionally equivalent)
- Used `std::mem::take(batch)` instead of `batch.drain(..).collect()` per clippy::drain_collect
- Added `allow(dead_code)` on stream_ingester/delta_buffer fields and accessor methods since they are wired in Plan 02

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- StreamIngester ready for REST endpoint wiring (Plan 03)
- DeltaBuffer stub ready for search pipeline integration (Plan 02)
- write_generation already bumped per upsert call (inherited from existing pipeline)

---
*Phase: 07-streaming-inserts*
*Completed: 2026-03-07*
