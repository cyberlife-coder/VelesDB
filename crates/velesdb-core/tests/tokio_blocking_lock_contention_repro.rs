#![cfg(feature = "persistence")]
//! Minimal, isolated reproduction of the `Collection` write/read lock
//! contention hang originally observed end-to-end through
//! `velesdb-memory`'s HTTP/MCP transport (PR #1524 adversarial review).
//!
//! This bypasses `velesdb-memory`, MCP, and HTTP entirely: it drives
//! `velesdb_core` directly from many concurrent `tokio::task::spawn_blocking`
//! closures on the SAME collection, mirroring exactly how
//! `velesdb_memory::mcp::McpServer::remember`/`recall` dispatch each tool
//! call (`crates/velesdb-memory/src/mcp.rs:172,199`), which in turn go
//! through `NativeStore::store`/`query_excluding`
//! (`crates/velesdb-memory/src/storage.rs:184,259`) straight to
//! `velesdb_core::agent::semantic_memory::SemanticMemory::store`/
//! `query_excluding`.
//!
//! # Two variants
//!
//! - `concurrent_spawn_blocking_upserts_and_searches_complete_within_bound`:
//!   drives `VectorCollection` (the public `Collection` wrapper) directly —
//!   the tightest possible isolation of `crud.rs`'s `batch_store_all`.
//! - `concurrent_spawn_blocking_semantic_memory_store_and_query_within_bound`:
//!   drives `SemanticMemory` (the actual layer `velesdb-memory` calls
//!   through), at the real production dimension (384) with real text
//!   payloads and a pre-seeded collection, using the EXACT two-read shape
//!   `MemoryService::recall` performs (`crates/velesdb-memory/src/service.rs:442-462`):
//!   a `query` (vector read lock) immediately followed by a separate
//!   `get_metadata_batch` call (payload read lock) over the returned ids.
//!
//! # Status as of this revision — root-caused and fixed
//!
//! Two independent, now-fixed bugs contributed to the hang, both found by
//! driving this repro to a live, sustained reproduction rather than trusting
//! a benign-looking absence of failures:
//!
//! 1. **`rayon::join` pool exhaustion** (`Collection::batch_store_all`,
//!    `crates/velesdb-core/src/collection/core/crud.rs`): payload and vector
//!    writes ran concurrently via `rayon::join`, dispatching onto rayon's
//!    small global thread pool from a foreign (`spawn_blocking`) thread.
//!    Fixed by running the two writes sequentially.
//! 2. **ABBA lock-order deadlock** (`Collection::get_raw`,
//!    `crates/velesdb-core/src/collection/core/crud_read_delete.rs`): acquired
//!    `payload_storage` then `vector_storage` — the reverse of
//!    `Collection::search`'s canonical `vector_storage` (rank 2) then
//!    `payload_storage` (rank 3) order. Under `parking_lot`'s
//!    writer-preferring `RwLock`, two readers taking the same lock pair in
//!    opposite orders — with `batch_store_all`'s writers queued on either
//!    lock — cycle forever. This was the dominant failure mode: fixing (1)
//!    alone left the repro red roughly 1 run in 15-25 (a sustained 186s hang,
//!    flat CPU, two `sample` captures 33s apart with identical stuck frames,
//!    confirmed this was a real cycle, not benign contention). A wider audit
//!    found and fixed the same reversed order at every other site in
//!    `Collection` that acquired both locks — see the fix commits for the
//!    full list.
//!
//! See `.investigation/http-deadlock-2026-07-22/` for the archived stack
//! samples and the original PR-level reproduction that started this
//! investigation.
//!
//! # Why `tokio::task::spawn_blocking` and not `std::thread::spawn`
//!
//! `crates/velesdb-core/tests/stress_concurrency_tests.rs` already stresses
//! `Collection` with up to 50 raw `std::thread::spawn` threads (25
//! writers + 25 readers x 100 ops) and completes in ~9s — no hang.
//!
//! # Anti-hang guard
//!
//! Every phase is wrapped in `tokio::time::timeout`. A stuck run FAILS
//! (panics) within the bound instead of hanging the test binary forever.
//! Per-task completion is tracked with `AtomicUsize` counters so a timeout
//! failure message reports exactly how many of the N concurrent operations
//! actually completed before the deadline.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;
use velesdb_core::agent::SemanticMemory;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::point::Point;
use velesdb_core::{Database, VectorCollection};

#[allow(clippy::cast_precision_loss)] // values are % 97, always < 97.0 — no precision loss
fn make_vector(dimension: usize, seed: u64) -> Vec<f32> {
    (0..dimension)
        .map(|i| ((seed.wrapping_add(i as u64) % 97) as f32) / 97.0)
        .collect()
}

/// Returns the collection AND its backing `TempDir` — the caller must keep
/// the `TempDir` alive for as long as the collection is used (it deletes the
/// directory on drop). Previously this leaked a fresh temp directory on
/// every single run via `mem::forget`; returning it instead lets the caller
/// hold it for the test's duration and clean it up normally at the end.
fn open_collection(label: &str, dimension: usize) -> (Arc<VectorCollection>, tempfile::TempDir) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join(label);
    let collection = VectorCollection::create(
        path,
        label,
        dimension,
        DistanceMetric::Cosine,
        velesdb_core::StorageMode::Full,
    )
    .expect("create collection");
    (Arc::new(collection), dir)
}

/// Prints `msg` to the real process stderr even when called from a
/// `#[tokio::test]` right before `std::process::exit` — libtest captures
/// stdout/stderr per-test on the thread it spawned to run the test (and only
/// flushes that capture when the test fails through its normal panic-based
/// reporting), so a message printed via `eprintln!` on that same thread,
/// followed immediately by `process::exit`, is silently swallowed. A brand
/// new OS thread has no capture override installed, so printing from one
/// bypasses this entirely.
fn print_to_real_stderr_before_exit(msg: String) {
    let _ = std::thread::spawn(move || eprintln!("{msg}")).join();
}

/// The load shape that hung end-to-end: 20 concurrent single-point
/// `remember`-equivalent upserts + 10 concurrent `recall`-equivalent
/// searches, all multiplexed against ONE shared collection via
/// `spawn_blocking`, exactly as `McpServer::remember`/`recall` do.
///
/// At tiny scale (dimension 8, no payload) this variant does NOT reproduce
/// the hang — see the module doc for what does.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_spawn_blocking_upserts_and_searches_complete_within_bound() {
    const DIMENSION: usize = 8;
    let (collection, _tempdir) = open_collection("tokio_repro", DIMENSION);

    let remembers_completed = Arc::new(AtomicUsize::new(0));
    let recalls_completed = Arc::new(AtomicUsize::new(0));

    let mut tasks = tokio::task::JoinSet::new();

    for i in 0..20u64 {
        let collection = Arc::clone(&collection);
        let counter = Arc::clone(&remembers_completed);
        tasks.spawn(async move {
            tokio::task::spawn_blocking(move || {
                let point = Point::without_payload(i, make_vector(DIMENSION, i));
                collection.upsert(vec![point])
            })
            .await
            .expect("upsert task must not panic")
            .expect("upsert must succeed");
            counter.fetch_add(1, Ordering::SeqCst);
        });
    }

    for i in 0..10u64 {
        let collection = Arc::clone(&collection);
        let counter = Arc::clone(&recalls_completed);
        tasks.spawn(async move {
            tokio::task::spawn_blocking(move || {
                let query = make_vector(DIMENSION, i * 7);
                collection.search(&query, 5)
            })
            .await
            .expect("search task must not panic")
            .expect("search must succeed");
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
        // keeps the OS thread occupied even after this test gives up on it.
        // A `panic!` here would unwind into the `#[tokio::test]` runtime's
        // `Drop`, which blocks shutdown until every outstanding blocking
        // task finishes — turning a bounded test timeout into an unbounded
        // hang during teardown (observed directly: `sample` on a stuck run
        // showed `tokio::runtime::blocking::pool::BlockingPool::shutdown`
        // waiting on exactly this). Exiting the process immediately makes
        // the failure fast and visible instead, matching this crate's
        // anti-hang guarantee for every concurrency test.
        print_to_real_stderr_before_exit(format!(
            "HANG REPRODUCED (VectorCollection layer): 30 concurrent spawn_blocking \
             upsert/search calls on a shared VectorCollection did not complete within \
             20s. Completed before timeout: {}/20 upserts, {}/10 searches.",
            remembers_completed.load(Ordering::SeqCst),
            recalls_completed.load(Ordering::SeqCst),
        ));
        std::process::exit(1);
    }

    assert_eq!(remembers_completed.load(Ordering::SeqCst), 20);
    assert_eq!(recalls_completed.load(Ordering::SeqCst), 10);
}

/// Production-shaped repro: `SemanticMemory` (the layer
/// `velesdb-memory::NativeStore` actually calls through
/// `AgentMemory::semantic()`) at the real default dimension (384), with
/// real text payloads and a pre-seeded collection, under the same 20
/// concurrent `remember`-equivalent stores + 10 concurrent
/// `recall`-equivalent queries multiplexed via `spawn_blocking`.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_spawn_blocking_semantic_memory_store_and_query_within_bound() {
    const DIMENSION: usize = velesdb_core::agent::DEFAULT_DIMENSION; // 384, matches production
    const SEED_POINTS: u64 = 200;

    let dir = tempdir().expect("tempdir");
    let db = Arc::new(Database::open(dir.path()).expect("open database"));
    let semantic = Arc::new(
        SemanticMemory::new_from_db(Arc::clone(&db), DIMENSION).expect("open semantic memory"),
    );

    // Pre-seed so searches have a realistic corpus to scan under contention.
    for i in 0..SEED_POINTS {
        semantic
            .store(
                i,
                &format!("seed fact number {i} with some realistic prose content"),
                &make_vector(DIMENSION, i),
            )
            .expect("seed store");
    }

    let stores_completed = Arc::new(AtomicUsize::new(0));
    let queries_completed = Arc::new(AtomicUsize::new(0));

    let mut tasks = tokio::task::JoinSet::new();

    for i in 0..20u64 {
        let semantic = Arc::clone(&semantic);
        let counter = Arc::clone(&stores_completed);
        let id = SEED_POINTS + i;
        tasks.spawn(async move {
            tokio::task::spawn_blocking(move || {
                semantic.store(
                    id,
                    &format!("shared fact {id}: concurrent remember payload text"),
                    &make_vector(DIMENSION, id),
                )
            })
            .await
            .expect("store task must not panic")
            .expect("store must succeed");
            counter.fetch_add(1, Ordering::SeqCst);
        });
    }

    // Mirrors MemoryService::recall exactly (crates/velesdb-memory/src/service.rs:442-462):
    // a `search` (SemanticMemory::query -> Collection::search, vector read lock)
    // immediately followed by a SEPARATE `get_metadata_batch` call
    // (SemanticMemory::get_metadata_batch -> Collection::get, payload read lock)
    // over the ids the search returned. A `recall`-only repro that stops at the
    // first read (as the earlier revision of this test did) does not reproduce
    // the hang; this two-read shape is the ingredient that was missing.
    for i in 0..10u64 {
        let semantic = Arc::clone(&semantic);
        let counter = Arc::clone(&queries_completed);
        tasks.spawn(async move {
            tokio::task::spawn_blocking(move || {
                let query = make_vector(DIMENSION, i * 7);
                let hits = semantic.query(&query, 10)?;
                let ids: Vec<u64> = hits.iter().map(|(id, _, _)| *id).collect();
                semantic.get_metadata_batch(&ids)
            })
            .await
            .expect("query task must not panic")
            .expect("query + get_metadata_batch must succeed");
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
        // See the sibling test's comment on why this is `process::exit`, not
        // `panic!`: uncancellable `spawn_blocking` stragglers can otherwise
        // turn a bounded 20s test timeout into an unbounded hang in the
        // `#[tokio::test]` runtime's teardown.
        print_to_real_stderr_before_exit(format!(
            "HANG REPRODUCED (SemanticMemory layer, dimension={DIMENSION}): 30 concurrent \
             spawn_blocking store/query calls did not complete within 20s. Completed before \
             timeout: {}/20 stores, {}/10 queries.",
            stores_completed.load(Ordering::SeqCst),
            queries_completed.load(Ordering::SeqCst),
        ));
        std::process::exit(1);
    }

    assert_eq!(stores_completed.load(Ordering::SeqCst), 20);
    assert_eq!(queries_completed.load(Ordering::SeqCst), 10);
}
