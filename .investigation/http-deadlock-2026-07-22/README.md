# Deadlock investigation — HTTP transport concurrency (2026-07-22)

Real, mechanically-confirmed deadlock (flat CPU time over 25s, not starvation)
under concurrent `remember`/`recall` on a shared MCP session. NOT in rmcp/HTTP —
localized to velesdb-core's collection locking:
`SemanticMemory::{store,query_excluding,get_metadata_batch}` -> `Collection`
lock acquisition. Zero rmcp/axum/tower frames in the stuck call chain.

Suspected trigger: `MemoryService::recall` (crates/velesdb-memory/src/service.rs:442-462)
does two sequential storage ops — `search()` then `get_metadata_batch()` — which
an isolated single-op repro did not reproduce (30/30 clean).

Files:
- `repro.rs` — standalone HTTP shared-session concurrent-load repro (velesdb-memory layer).
- `stack-sample-shared-session.txt` — stack sample from repro.rs hanging.
- `stack-sample-pr-tests.txt` — stack sample from PR #1524's own concurrency tests hanging.

Work in progress on branch `fix/core-collection-lock-contention` (off `develop`,
separate from `feat/memory-http-transport`/PR #1524, which depends on this fix).
Fix not yet written as of this checkpoint — see `crates/velesdb-core/tests/tokio_blocking_lock_contention_repro.rs`.
