#![cfg(feature = "persistence")]
//! Reproduction of the `velesdb-core` `Collection` lock-contention deadlock
//! at the `MemoryService` layer — no HTTP, no MCP, no rmcp.
//!
//! # Why this file exists — root cause found and fixed in `velesdb-core`
//!
//! (see `.investigation/http-deadlock-2026-07-22/` in `velesdb-core`'s repo
//! for the full trail)
//!
//! Two bugs in `velesdb-core`'s `Collection` locking contributed to the
//! end-to-end HTTP hang, both now fixed:
//!
//! 1. `Collection::batch_store_all` ran payload/vector writes concurrently
//!    via `rayon::join`, dispatched from a foreign (`spawn_blocking`) thread
//!    onto rayon's small global pool — exhaustible under concurrent load.
//! 2. `Collection::get_raw` acquired `payload_storage` then `vector_storage`,
//!    the reverse of `Collection::search`'s canonical order — a classic ABBA
//!    deadlock under `parking_lot`'s writer-preferring `RwLock`. This was the
//!    dominant failure mode: fixing (1) alone still hung roughly 1 run in
//!    15-25 (confirmed via a sustained 186s hang with flat CPU and two stack
//!    samples 33s apart showing identical frames — a real cycle, not benign
//!    contention).
//!
//! This test drives `MemoryService::remember`/`recall` directly (the real
//! call path `McpServer::remember`/`recall` dispatch to via
//! `tokio::task::spawn_blocking` — `crates/velesdb-memory/src/mcp.rs:172,199`),
//! bypassing MCP/rmcp/HTTP/axum entirely — confirming the deadlock is
//! reachable through `MemoryService`/`NativeStore` alone, nothing to do with
//! the transport. Unlike the `velesdb-core`-only repro, this uses the REAL
//! `MemoryService::remember`/`recall` code paths (id derivation, embedder,
//! TTL bookkeeping, the real `search` + `get_metadata_batch` pair inside
//! `recall`) rather than an approximation of them.
//!
//! # Anti-hang guard
//!
//! Wrapped in `tokio::time::timeout`. A stuck run FAILS within the bound
//! instead of hanging the test binary forever. Per-task completion is
//! tracked so a timeout failure message reports exactly how many of the N
//! concurrent operations completed before the deadline.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;
use velesdb_memory::{HashEmbedder, MemoryService, DEFAULT_DIMENSION};

/// 20 concurrent `remember`s + 10 concurrent `recall`s against ONE shared
/// `MemoryService`, multiplexed via `spawn_blocking` exactly as
/// `McpServer::remember`/`recall` do — the load shape that hung end-to-end
/// over HTTP. No HTTP, no MCP, no rmcp anywhere in this test.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_spawn_blocking_remember_and_recall_within_bound() {
    let dir = tempdir().expect("tempdir");
    let embedder = HashEmbedder::new(DEFAULT_DIMENSION);
    let service = Arc::new(MemoryService::open(dir.path(), embedder).expect("open memory store"));

    // Pre-seed so recalls have a realistic corpus to scan under contention.
    for i in 0..200u64 {
        service
            .remember(
                &format!("seed fact number {i} with some realistic prose content"),
                &[],
                None,
            )
            .expect("seed remember");
    }

    let remembers_completed = Arc::new(AtomicUsize::new(0));
    let recalls_completed = Arc::new(AtomicUsize::new(0));

    let mut tasks = tokio::task::JoinSet::new();

    for i in 0..20u64 {
        let service = Arc::clone(&service);
        let counter = Arc::clone(&remembers_completed);
        tasks.spawn(async move {
            tokio::task::spawn_blocking(move || {
                service.remember(
                    &format!("shared fact {i}: concurrent remember payload text"),
                    &[],
                    None,
                )
            })
            .await
            .expect("remember task must not panic")
            .expect("remember must succeed");
            counter.fetch_add(1, Ordering::SeqCst);
        });
    }

    for i in 0..10u64 {
        let service = Arc::clone(&service);
        let counter = Arc::clone(&recalls_completed);
        tasks.spawn(async move {
            tokio::task::spawn_blocking(move || {
                service.recall(&format!("shared fact {i}"), 10, None)
            })
            .await
            .expect("recall task must not panic")
            .expect("recall must succeed");
            counter.fetch_add(1, Ordering::SeqCst);
        });
    }

    let all = async {
        while let Some(result) = tasks.join_next().await {
            result.expect("spawned task must not panic");
        }
    };

    let outcome = tokio::time::timeout(Duration::from_secs(20), all).await;

    if outcome.is_err() {
        // `std::process::exit`, not `panic!`: `tokio::task::spawn_blocking`
        // closures cannot be cancelled once started, so a still-stuck one
        // keeps its OS thread occupied even after this test gives up on it.
        // A `panic!` here would unwind into the `#[tokio::test]` runtime's
        // `Drop`, which blocks shutdown until every outstanding blocking
        // task finishes — turning a bounded test timeout into an unbounded
        // hang during teardown. Exiting the process immediately makes the
        // failure fast and visible instead.
        //
        // Printed from a fresh OS thread, not directly via `eprintln!` here:
        // libtest captures stdout/stderr per-test on the thread it spawned,
        // only flushing that capture on a normal panic-based failure — a
        // direct `eprintln!` immediately followed by `process::exit` is
        // silently swallowed. A brand new thread has no capture override.
        let msg = format!(
            "HANG REPRODUCED (MemoryService layer, no HTTP/MCP): 30 concurrent \
             spawn_blocking remember/recall calls did not complete within 20s. \
             Completed before timeout: {}/20 remembers, {}/10 recalls.",
            remembers_completed.load(Ordering::SeqCst),
            recalls_completed.load(Ordering::SeqCst),
        );
        let _ = std::thread::spawn(move || eprintln!("{msg}")).join();
        std::process::exit(1);
    }

    assert_eq!(remembers_completed.load(Ordering::SeqCst), 20);
    assert_eq!(recalls_completed.load(Ordering::SeqCst), 10);
}
