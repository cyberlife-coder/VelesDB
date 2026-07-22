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
//!   payloads and a pre-seeded collection, since a from-scratch/tiny-vector
//!   `VectorCollection`-only run did not reproduce the hang (see comment
//!   below) — this variant is what actually reproduces it.
//!
//! # Why `tokio::task::spawn_blocking` and not `std::thread::spawn`
//!
//! `crates/velesdb-core/tests/stress_concurrency_tests.rs` already stresses
//! `Collection` with up to 50 raw `std::thread::spawn` threads (25
//! writers + 25 readers x 100 ops) and completes in ~9s — no hang. A
//! same-shape `VectorCollection`-only repro through `tokio::spawn_blocking`
//! (see the first test below) ALSO completes instantly (~0.1s) — so the
//! hang is not simply "many concurrent Collection ops through
//! spawn_blocking" either. It only reproduces at the `SemanticMemory`
//! layer with production-realistic dimension/payload/scale, which is what
//! the second test drives.
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

fn make_vector(dimension: usize, seed: u64) -> Vec<f32> {
    (0..dimension)
        .map(|i| ((seed.wrapping_add(i as u64) % 97) as f32) / 97.0)
        .collect()
}

fn open_collection(label: &str, dimension: usize) -> Arc<VectorCollection> {
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
    std::mem::forget(dir);
    Arc::new(collection)
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
    let collection = open_collection("tokio_repro", DIMENSION);

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
        panic!(
            "HANG REPRODUCED (VectorCollection layer): 30 concurrent spawn_blocking \
             upsert/search calls on a shared VectorCollection did not complete within \
             20s. Completed before timeout: {}/20 upserts, {}/10 searches.",
            remembers_completed.load(Ordering::SeqCst),
            recalls_completed.load(Ordering::SeqCst),
        );
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
    let semantic = Arc::new(SemanticMemory::new_from_db(Arc::clone(&db), DIMENSION).expect("open semantic memory"));

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

    for i in 0..10u64 {
        let semantic = Arc::clone(&semantic);
        let counter = Arc::clone(&queries_completed);
        tasks.spawn(async move {
            tokio::task::spawn_blocking(move || {
                let query = make_vector(DIMENSION, i * 7);
                semantic.query(&query, 10)
            })
            .await
            .expect("query task must not panic")
            .expect("query must succeed");
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
        panic!(
            "HANG REPRODUCED (SemanticMemory layer, dimension={DIMENSION}): 30 concurrent \
             spawn_blocking store/query calls did not complete within 20s. Completed before \
             timeout: {}/20 stores, {}/10 queries.",
            stores_completed.load(Ordering::SeqCst),
            queries_completed.load(Ordering::SeqCst),
        );
    }

    assert_eq!(stores_completed.load(Ordering::SeqCst), 20);
    assert_eq!(queries_completed.load(Ordering::SeqCst), 10);
}
